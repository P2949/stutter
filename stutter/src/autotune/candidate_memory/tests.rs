use std::collections::BTreeMap;

use super::{
    key::{CandidateIdentitySummary, CandidateMemoryKey, ExecutableFingerprint, WorkloadIdentity},
    model::{CandidateClassCount, WorkloadActionMemory},
    *,
};
use crate::{
    actions::{ActionId, SafetyClass},
    autotune::{
        observation::{AutotuneObservation, WorkloadIdentity as ObservationWorkloadIdentity},
        state::SituationKind,
    },
    focus::FocusGroupKind,
    process_tree::TaskClass,
};

fn fake_candidate() -> crate::autotune::planning::candidate::CandidateAction {
    crate::autotune::planning::candidate::CandidateAction::fake(
        ActionId::new("cpu-affinity-profile:test".to_owned()),
        SafetyClass::ReversibleLowRisk,
    )
}

fn context_with_distribution(distribution: Vec<CandidateClassCount>) -> CandidateContextHashInput {
    CandidateContextHashInput {
        workload_identity: WorkloadIdentity::new("workload-a"),
        workload_root_pid: Some(1234),
        workload_starttime_ticks: Some(77),
        workload_focus_kind: Some(FocusGroupKind::Game),
        target_executable: Some(ExecutableFingerprint::new(8, 99)),
        cgroup_path: Some("/user.slice/app.scope".to_owned()),
        cpu_topology_signature: Some("cpu0-7:smt:on".to_owned()),
        profile_name: "fake-profile".to_owned(),
        situation: SituationKind::GameCpuSchedulerPressure,
        active_task_class_distribution: distribution,
    }
}

#[test]
fn context_hash_normalizes_class_distribution_order_and_zero_counts() {
    let left = context_with_distribution(vec![
        CandidateClassCount {
            class: TaskClass::WineServer,
            count: 1,
        },
        CandidateClassCount {
            class: TaskClass::Game,
            count: 2,
        },
        CandidateClassCount {
            class: TaskClass::Game,
            count: 3,
        },
        CandidateClassCount {
            class: TaskClass::Unknown,
            count: 0,
        },
    ]);

    let right = context_with_distribution(vec![
        CandidateClassCount {
            class: TaskClass::Game,
            count: 5,
        },
        CandidateClassCount {
            class: TaskClass::WineServer,
            count: 1,
        },
    ]);

    assert_eq!(left.context_hash(), right.context_hash());
    assert_eq!(
        left.normalized().active_task_class_distribution,
        vec![
            CandidateClassCount {
                class: TaskClass::Game,
                count: 5,
            },
            CandidateClassCount {
                class: TaskClass::WineServer,
                count: 1,
            },
        ]
    );
}

#[test]
fn typed_key_constructors_reject_blank_identity_and_key() {
    assert!(WorkloadIdentity::new("  ").is_none());
    assert!(CandidateMemoryKey::new("  ").is_none());
    assert!(CandidateIdentitySummary::new("  ").is_none());
    assert_eq!(
        ExecutableFingerprint::from_parts(Some(8), Some(99))
            .map(|fingerprint| (fingerprint.dev(), fingerprint.ino())),
        Some((8, 99))
    );
    assert!(ExecutableFingerprint::from_parts(Some(8), None).is_none());
}

#[test]
fn memory_records_candidate_result_context_hash_score_delta_reason_and_cooldown() {
    let candidate = fake_candidate();
    let context = context_with_distribution(vec![CandidateClassCount {
        class: TaskClass::Game,
        count: 4,
    }]);
    let mut memory = CandidateMemory::default();

    let record = memory.record_result(CandidateResultRecordInput {
        candidate: &candidate,
        context: &context,
        now_unix_nanos: 1_000,
        result: CandidateMemoryResult::Reverted,
        diagnostic_baseline_raw_score_total: Some(1_000),
        diagnostic_current_raw_score_total: Some(1_125),
        rollback_reason: Some(" candidate regressed ".to_owned()),
        cooldown_expires_unix_nanos: Some(301_000_000_000),
    });

    assert_eq!(
        record.action_id,
        ActionId::new("cpu-affinity-profile:test".to_owned())
    );
    assert_eq!(record.candidate_name, "fake-profile");
    assert_eq!(record.last_tried_unix_nanos, 1_000);
    assert_eq!(record.result, CandidateMemoryResult::Reverted);
    assert_eq!(record.context_hash, context.context_key());
    assert!(
        record
            .identity_summary
            .as_ref()
            .is_some_and(|summary| summary.as_str().contains("root_pid=1234"))
    );
    assert_eq!(record.degraded_reason, None);
    assert_eq!(record.score_delta, 125);
    assert_eq!(
        record.rollback_reason.as_deref(),
        Some("candidate regressed")
    );
    assert_eq!(record.cooldown_expires_unix_nanos, Some(301_000_000_000));
    assert_eq!(memory.records.len(), 1);
    assert_eq!(memory.latest(), Some(&record));
}

#[test]
fn memory_upserts_same_action_and_context() {
    let candidate = fake_candidate();
    let context = context_with_distribution(vec![CandidateClassCount {
        class: TaskClass::Game,
        count: 4,
    }]);
    let mut memory = CandidateMemory::default();

    memory.record_attempt(&candidate, &context, 1_000, None);
    let record = memory.record_result(CandidateResultRecordInput {
        candidate: &candidate,
        context: &context,
        now_unix_nanos: 2_000,
        result: CandidateMemoryResult::Kept,
        diagnostic_baseline_raw_score_total: Some(1_000),
        diagnostic_current_raw_score_total: Some(800),
        rollback_reason: None,
        cooldown_expires_unix_nanos: Some(62_000_000_000),
    });

    assert_eq!(memory.records.len(), 1);
    assert_eq!(memory.latest(), Some(&record));
    assert_eq!(memory.records[0].result, CandidateMemoryResult::Kept);
    assert_eq!(memory.records[0].score_delta, -200);
    assert_eq!(
        memory.records[0].cooldown_expires_unix_nanos,
        Some(62_000_000_000)
    );
    assert_eq!(memory.workload_actions.len(), 1);
    assert_eq!(
        memory.workload_actions[0].last_result,
        CandidateMemoryResult::Kept
    );
    assert_eq!(memory.workload_actions[0].score_delta, Some(-200.0));
    assert!(
        memory.workload_actions[0]
            .identity_summary
            .as_ref()
            .is_some_and(|summary| summary.as_str().contains("workload_hash=workload-a"))
    );
    assert_eq!(memory.workload_actions[0].degraded_reason, None);
}

#[test]
fn cooldown_remaining_uses_latest_record_for_action() {
    let candidate = fake_candidate();
    let context = context_with_distribution(vec![CandidateClassCount {
        class: TaskClass::Game,
        count: 4,
    }]);
    let action_id = candidate.action_id();
    let mut memory = CandidateMemory::default();

    memory.record_attempt(&candidate, &context, 1_000, Some(301_000_000_000));

    let remaining = memory
        .cooldown_remaining_for_action(&action_id, 300_000_000_000)
        .unwrap();

    assert_eq!(remaining.as_secs(), 1);
    assert_eq!(
        memory.cooldown_remaining_for_action(&action_id, 301_000_000_000),
        None
    );
}

#[test]
fn workload_action_memory_is_scoped_to_stable_workload_identity() {
    let candidate = fake_candidate();
    let context = context_with_distribution(vec![CandidateClassCount {
        class: TaskClass::Game,
        count: 4,
    }]);
    let mut other_context = context.clone();
    other_context.workload_identity = WorkloadIdentity::new("workload-b");
    other_context.target_executable = Some(ExecutableFingerprint::new(8, 100));
    let mut memory = CandidateMemory::default();

    memory.record_result(CandidateResultRecordInput {
        candidate: &candidate,
        context: &context,
        now_unix_nanos: 1_000,
        result: CandidateMemoryResult::Reverted,
        diagnostic_baseline_raw_score_total: Some(100),
        diagnostic_current_raw_score_total: Some(120),
        rollback_reason: None,
        cooldown_expires_unix_nanos: Some(10_000),
    });

    assert!(
        memory
            .cooldown_remaining_for_workload_action(&candidate, &context, 2_000)
            .is_some()
    );
    assert!(
        memory
            .cooldown_remaining_for_workload_action(&candidate, &other_context, 2_000)
            .is_none()
    );
}

#[test]
fn workload_action_memory_rejects_matching_hash_with_different_identity_summary() {
    let candidate = fake_candidate();
    let context = context_with_distribution(vec![CandidateClassCount {
        class: TaskClass::Game,
        count: 4,
    }]);
    let mut colliding_context = context.clone();
    colliding_context.workload_root_pid = Some(5678);
    colliding_context.workload_starttime_ticks = Some(88);
    let mut memory = CandidateMemory::default();

    memory.record_result(CandidateResultRecordInput {
        candidate: &candidate,
        context: &context,
        now_unix_nanos: 1_000,
        result: CandidateMemoryResult::Reverted,
        diagnostic_baseline_raw_score_total: Some(100),
        diagnostic_current_raw_score_total: Some(120),
        rollback_reason: None,
        cooldown_expires_unix_nanos: Some(10_000),
    });

    assert!(
        memory
            .cooldown_remaining_for_workload_action(&candidate, &colliding_context, 2_000)
            .is_none()
    );
}

#[test]
fn deserializing_memory_marks_context_hash_collisions_degraded() {
    let candidate = fake_candidate();
    let context = context_with_distribution(vec![CandidateClassCount {
        class: TaskClass::Game,
        count: 4,
    }]);
    let mut colliding_context = context.clone();
    colliding_context.workload_root_pid = Some(5678);
    colliding_context.workload_starttime_ticks = Some(88);
    let mut memory = CandidateMemory::default();

    memory.record_result(CandidateResultRecordInput {
        candidate: &candidate,
        context: &context,
        now_unix_nanos: 1_000,
        result: CandidateMemoryResult::Reverted,
        diagnostic_baseline_raw_score_total: Some(100),
        diagnostic_current_raw_score_total: Some(120),
        rollback_reason: None,
        cooldown_expires_unix_nanos: Some(10_000),
    });
    let mut colliding_record = memory.records[0].clone();
    colliding_record.identity_summary = Some(colliding_context.identity_summary());
    colliding_record.last_tried_unix_nanos = 2_000;
    memory.records.push(colliding_record);

    let encoded = serde_json::to_string(&memory).unwrap();
    let decoded: CandidateMemory = serde_json::from_str(&encoded).unwrap();

    assert_eq!(decoded.records.len(), 2);
    assert!(decoded.records.iter().all(|record| record.is_degraded()));
    assert!(decoded.last_for_action(&candidate.action_id()).is_none());
    assert!(
        decoded
            .degraded_diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.contains("context_hash"))
    );
}

#[test]
fn deserializing_memory_marks_workload_hash_collisions_degraded() {
    let candidate = fake_candidate();
    let action_id = candidate.action_id();
    let memory = CandidateMemory {
        records: Vec::new(),
        workload_actions: vec![
            WorkloadActionMemory {
                workload_hash: "same-workload-hash".to_owned(),
                action_id: action_id.clone(),
                action_kind: "nice".to_owned(),
                objective: crate::autotune::objective::ObjectiveKind::DesktopInteractivity,
                situation: SituationKind::GameCpuSchedulerPressure,
                last_result: CandidateMemoryResult::Reverted,
                score_delta: Some(10.0),
                last_seen_unix_nanos: 1_000,
                cooldown_until_unix_nanos: Some(10_000),
                exe_dev: Some(8),
                exe_ino: Some(99),
                cgroup_path: Some("/user.slice/app.scope".to_owned()),
                identity_summary: CandidateIdentitySummary::new("root_pid=1234 exe=8:99"),
                degraded_reason: None,
            },
            WorkloadActionMemory {
                workload_hash: "same-workload-hash".to_owned(),
                action_id,
                action_kind: "nice".to_owned(),
                objective: crate::autotune::objective::ObjectiveKind::DesktopInteractivity,
                situation: SituationKind::GameCpuSchedulerPressure,
                last_result: CandidateMemoryResult::Reverted,
                score_delta: Some(20.0),
                last_seen_unix_nanos: 2_000,
                cooldown_until_unix_nanos: Some(10_000),
                exe_dev: Some(8),
                exe_ino: Some(99),
                cgroup_path: Some("/user.slice/app.scope".to_owned()),
                identity_summary: CandidateIdentitySummary::new("root_pid=5678 exe=8:99"),
                degraded_reason: None,
            },
        ],
    };

    let encoded = serde_json::to_string(&memory).unwrap();
    let decoded: CandidateMemory = serde_json::from_str(&encoded).unwrap();

    assert_eq!(decoded.workload_actions.len(), 2);
    assert!(
        decoded
            .workload_actions
            .iter()
            .all(WorkloadActionMemory::is_degraded)
    );
    assert!(
        decoded
            .degraded_diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.contains("workload_hash"))
    );
}

#[test]
fn observation_context_uses_workload_identity_and_inventory_signature() {
    let candidate = fake_candidate();
    let observation = AutotuneObservation {
        primary_situation: SituationKind::BrowserCpuPressure,
        topology_signature: Some("inventory-a".to_owned()),
        workload_identity: Some(ObservationWorkloadIdentity {
            root_pid: 1234,
            process_starttime_ticks: Some(77),
            exe_dev: Some(8),
            exe_ino: Some(99),
            cgroup_path: Some("/user.slice/app.scope".to_owned()),
            focus_kind: Some(FocusGroupKind::Browser),
            class_distribution: BTreeMap::from([
                ("BrowserForeground".to_owned(), 1),
                ("BrowserRenderer".to_owned(), 3),
            ]),
            stable_hash: "workload-a".to_owned(),
        }),
        ..AutotuneObservation::default()
    };

    let context = CandidateContextHashInput::from_observation(&candidate, &observation, None);

    assert_eq!(
        context.target_executable.as_ref().map(|value| value.dev()),
        Some(8)
    );
    assert_eq!(
        context.target_executable.as_ref().map(|value| value.ino()),
        Some(99)
    );
    assert_eq!(
        context
            .workload_identity
            .as_ref()
            .map(|value| value.as_str()),
        Some("workload-a")
    );
    assert_eq!(context.workload_root_pid, Some(1234));
    assert_eq!(context.workload_starttime_ticks, Some(77));
    assert_eq!(context.workload_focus_kind, Some(FocusGroupKind::Browser));
    assert_eq!(
        context.cgroup_path.as_deref(),
        Some("/user.slice/app.scope")
    );
    assert_eq!(
        context.cpu_topology_signature.as_deref(),
        Some("inventory-a")
    );
    assert_eq!(context.situation, SituationKind::BrowserCpuPressure);
    assert_eq!(
        context.normalized().active_task_class_distribution,
        vec![
            CandidateClassCount {
                class: TaskClass::BrowserForeground,
                count: 1,
            },
            CandidateClassCount {
                class: TaskClass::BrowserRenderer,
                count: 3,
            },
        ]
    );
}
