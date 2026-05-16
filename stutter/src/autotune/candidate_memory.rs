use std::{collections::BTreeMap, time::Duration};

use serde::{Deserialize, Serialize};

use super::{
    candidate::CandidateAction, objective::ObjectiveKind, observation::AutotuneObservation,
    state::SituationKind,
};
use crate::{
    actions::ActionId,
    process_tree::{TargetSnapshot, TaskClass},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CandidateMemoryResult {
    Tried,
    Kept,
    Reverted,
    Faulted,
    Inconclusive,
    Rejected,
}

impl CandidateMemoryResult {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tried => "tried",
            Self::Kept => "kept",
            Self::Reverted => "reverted",
            Self::Faulted => "faulted",
            Self::Inconclusive => "inconclusive",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateClassCount {
    pub class: TaskClass,
    pub count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateContextHashInput {
    pub workload_hash: Option<String>,
    pub target_exe_dev: Option<u64>,
    pub target_exe_ino: Option<u64>,
    pub cgroup_path: Option<String>,
    pub cpu_topology_signature: Option<String>,
    pub profile_name: String,
    pub situation: SituationKind,
    pub active_task_class_distribution: Vec<CandidateClassCount>,
}

impl CandidateContextHashInput {
    pub fn for_candidate(candidate: &CandidateAction) -> Self {
        Self {
            workload_hash: None,
            target_exe_dev: None,
            target_exe_ino: None,
            cgroup_path: None,
            cpu_topology_signature: None,
            profile_name: candidate.candidate_name().to_owned(),
            situation: SituationKind::Unknown,
            active_task_class_distribution: Vec::new(),
        }
    }

    pub fn from_observation(
        candidate: &CandidateAction,
        observation: &AutotuneObservation,
        cpu_topology_signature: Option<&str>,
    ) -> Self {
        Self {
            workload_hash: observation
                .workload_identity
                .as_ref()
                .map(|identity| identity.stable_hash.clone()),
            target_exe_dev: observation
                .workload_identity
                .as_ref()
                .and_then(|identity| identity.exe_dev),
            target_exe_ino: observation
                .workload_identity
                .as_ref()
                .and_then(|identity| identity.exe_ino),
            cgroup_path: observation
                .workload_identity
                .as_ref()
                .and_then(|identity| normalize_optional_string(identity.cgroup_path.as_deref())),
            cpu_topology_signature: normalize_optional_string(
                cpu_topology_signature.or(observation.topology_signature.as_deref()),
            ),
            profile_name: candidate.candidate_name().to_owned(),
            situation: observation.primary_situation,
            active_task_class_distribution: class_distribution_from_workload_identity(observation),
        }
    }

    pub fn from_snapshot(
        candidate: &CandidateAction,
        observation: &AutotuneObservation,
        cpu_topology_signature: Option<&str>,
        snapshot: &TargetSnapshot,
    ) -> Self {
        let (target_exe_dev, target_exe_ino) =
            target_executable_inode_from_snapshot(observation, snapshot);

        Self {
            workload_hash: observation
                .workload_identity
                .as_ref()
                .map(|identity| identity.stable_hash.clone()),
            target_exe_dev,
            target_exe_ino,
            cgroup_path: observation
                .workload_identity
                .as_ref()
                .and_then(|identity| normalize_optional_string(identity.cgroup_path.as_deref())),
            cpu_topology_signature: normalize_optional_string(
                cpu_topology_signature.or(observation.topology_signature.as_deref()),
            ),
            profile_name: candidate.candidate_name().to_owned(),
            situation: observation.primary_situation,
            active_task_class_distribution: class_distribution_from_snapshot(snapshot),
        }
    }

    pub fn normalized(mut self) -> Self {
        self.cpu_topology_signature = self
            .cpu_topology_signature
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        self.workload_hash = self
            .workload_hash
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        self.cgroup_path = self
            .cgroup_path
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());

        self.active_task_class_distribution =
            normalize_class_distribution(self.active_task_class_distribution);

        self
    }

    pub fn context_hash(&self) -> String {
        let normalized = self.clone().normalized();
        let mut parts = Vec::new();

        parts.push(format!(
            "workload_hash={}",
            normalized.workload_hash.as_deref().unwrap_or("-")
        ));
        parts.push(format!(
            "target_exe={}:{}",
            normalized
                .target_exe_dev
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            normalized
                .target_exe_ino
                .map(|value| value.to_string())
                .unwrap_or_else(|| "-".to_owned())
        ));

        parts.push(format!(
            "cgroup={}",
            normalized.cgroup_path.as_deref().unwrap_or("-")
        ));
        parts.push(format!(
            "cpu_topology={}",
            normalized.cpu_topology_signature.as_deref().unwrap_or("-")
        ));
        parts.push(format!("profile={}", normalized.profile_name));
        parts.push(format!("situation={:?}", normalized.situation));

        for count in &normalized.active_task_class_distribution {
            parts.push(format!("class={}:{}", count.class.as_str(), count.count));
        }

        stable_hash_hex(&parts)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateMemoryRecord {
    pub action_id: ActionId,
    pub candidate_name: String,
    pub last_tried_unix_nanos: u128,
    pub result: CandidateMemoryResult,
    pub context_hash: String,
    pub score_delta: i64,
    pub rollback_reason: Option<String>,
    pub cooldown_expires_unix_nanos: Option<u128>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WorkloadActionMemory {
    pub workload_hash: String,
    pub action_id: ActionId,
    pub action_kind: String,
    pub objective: ObjectiveKind,
    pub situation: SituationKind,
    pub last_result: CandidateMemoryResult,
    pub score_delta: Option<f64>,
    pub last_seen_unix_nanos: u128,
    pub cooldown_until_unix_nanos: Option<u128>,
    pub exe_dev: Option<u64>,
    pub exe_ino: Option<u64>,
    pub cgroup_path: Option<String>,
}

impl WorkloadActionMemory {
    pub fn matches_context(&self, context: &CandidateContextHashInput) -> bool {
        let normalized = context.clone().normalized();
        normalized
            .workload_hash
            .as_ref()
            .is_some_and(|hash| hash == &self.workload_hash)
            && normalized.target_exe_dev == self.exe_dev
            && normalized.target_exe_ino == self.exe_ino
            && normalized.cgroup_path == self.cgroup_path
            && normalized.situation == self.situation
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct CandidateMemory {
    pub records: Vec<CandidateMemoryRecord>,
    #[serde(default)]
    pub workload_actions: Vec<WorkloadActionMemory>,
}

impl CandidateMemory {
    pub fn record_attempt(
        &mut self,
        candidate: &CandidateAction,
        context: &CandidateContextHashInput,
        now_unix_nanos: u128,
        cooldown_expires_unix_nanos: Option<u128>,
    ) -> CandidateMemoryRecord {
        self.record_result(
            candidate,
            context,
            now_unix_nanos,
            CandidateMemoryResult::Tried,
            None,
            None,
            None,
            cooldown_expires_unix_nanos,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_result(
        &mut self,
        candidate: &CandidateAction,
        context: &CandidateContextHashInput,
        now_unix_nanos: u128,
        result: CandidateMemoryResult,
        baseline_score_total: Option<u64>,
        current_score_total: Option<u64>,
        rollback_reason: Option<String>,
        cooldown_expires_unix_nanos: Option<u128>,
    ) -> CandidateMemoryRecord {
        let context_hash = context.context_hash();
        let action_id = candidate.action_id();
        let record = CandidateMemoryRecord {
            action_id: action_id.clone(),
            candidate_name: candidate.candidate_name().to_owned(),
            last_tried_unix_nanos: now_unix_nanos,
            result,
            context_hash: context_hash.clone(),
            score_delta: score_delta_i64(baseline_score_total, current_score_total),
            rollback_reason: normalize_owned_reason(rollback_reason),
            cooldown_expires_unix_nanos,
        };

        if let Some(existing) = self.records.iter_mut().find(|existing| {
            existing.action_id == action_id && existing.context_hash == context_hash
        }) {
            *existing = record.clone();
        } else {
            self.records.push(record.clone());
        }

        self.record_workload_action(
            candidate,
            context,
            &record,
            baseline_score_total,
            current_score_total,
        );
        self.enforce_bounds();

        record
    }

    pub fn latest(&self) -> Option<&CandidateMemoryRecord> {
        self.records
            .iter()
            .max_by_key(|record| record.last_tried_unix_nanos)
    }

    pub fn last_for_action(&self, action_id: &ActionId) -> Option<&CandidateMemoryRecord> {
        self.records
            .iter()
            .filter(|record| &record.action_id == action_id)
            .max_by_key(|record| record.last_tried_unix_nanos)
    }

    pub fn cooldown_remaining_for_action(
        &self,
        action_id: &ActionId,
        now_unix_nanos: u128,
    ) -> Option<Duration> {
        let record = self.last_for_action(action_id)?;
        let cooldown_expires = record.cooldown_expires_unix_nanos?;

        if now_unix_nanos >= cooldown_expires {
            return None;
        }

        Some(duration_from_remaining_nanos(
            cooldown_expires.saturating_sub(now_unix_nanos),
        ))
    }

    pub fn last_for_workload_action(
        &self,
        candidate: &CandidateAction,
        context: &CandidateContextHashInput,
    ) -> Option<&WorkloadActionMemory> {
        let action_id = candidate.action_id();
        self.workload_actions
            .iter()
            .filter(|memory| memory.action_id == action_id && memory.matches_context(context))
            .max_by_key(|memory| memory.last_seen_unix_nanos)
    }

    pub fn cooldown_remaining_for_workload_action(
        &self,
        candidate: &CandidateAction,
        context: &CandidateContextHashInput,
        now_unix_nanos: u128,
    ) -> Option<Duration> {
        let memory = self.last_for_workload_action(candidate, context)?;
        let cooldown_until = memory.cooldown_until_unix_nanos?;
        if now_unix_nanos >= cooldown_until {
            return None;
        }

        Some(duration_from_remaining_nanos(
            cooldown_until.saturating_sub(now_unix_nanos),
        ))
    }

    pub fn workload_rank_hint(
        &self,
        candidate: &CandidateAction,
        context: &CandidateContextHashInput,
    ) -> Option<i32> {
        let memory = self.last_for_workload_action(candidate, context)?;
        match memory.last_result {
            CandidateMemoryResult::Kept => Some(-100),
            CandidateMemoryResult::Reverted
            | CandidateMemoryResult::Faulted
            | CandidateMemoryResult::Rejected => Some(100),
            CandidateMemoryResult::Inconclusive => Some(20),
            CandidateMemoryResult::Tried => Some(0),
        }
    }

    fn record_workload_action(
        &mut self,
        candidate: &CandidateAction,
        context: &CandidateContextHashInput,
        record: &CandidateMemoryRecord,
        baseline_score_total: Option<u64>,
        current_score_total: Option<u64>,
    ) {
        let context = context.clone().normalized();
        let Some(workload_hash) = context.workload_hash.clone() else {
            return;
        };
        let action_id = candidate.action_id();
        let action_kind = candidate.action_kind().to_owned();
        let score_delta = score_delta_f64(baseline_score_total, current_score_total).or(Some(0.0));
        let memory = WorkloadActionMemory {
            workload_hash,
            action_id: action_id.clone(),
            action_kind,
            objective: candidate.objective(),
            situation: context.situation,
            last_result: record.result.clone(),
            score_delta,
            last_seen_unix_nanos: record.last_tried_unix_nanos,
            cooldown_until_unix_nanos: record.cooldown_expires_unix_nanos,
            exe_dev: context.target_exe_dev,
            exe_ino: context.target_exe_ino,
            cgroup_path: context.cgroup_path.clone(),
        };

        if let Some(existing) = self
            .workload_actions
            .iter_mut()
            .find(|existing| existing.action_id == action_id && existing.matches_context(&context))
        {
            *existing = memory;
        } else {
            self.workload_actions.push(memory);
        }
    }

    fn enforce_bounds(&mut self) {
        const MAX_RECORDS: usize = 512;
        const MAX_WORKLOAD_ACTIONS: usize = 512;

        if self.records.len() > MAX_RECORDS {
            self.records
                .sort_by_key(|record| std::cmp::Reverse(record.last_tried_unix_nanos));
            self.records.truncate(MAX_RECORDS);
        }
        if self.workload_actions.len() > MAX_WORKLOAD_ACTIONS {
            self.workload_actions
                .sort_by_key(|memory| std::cmp::Reverse(memory.last_seen_unix_nanos));
            self.workload_actions.truncate(MAX_WORKLOAD_ACTIONS);
        }
    }
}

fn class_distribution_from_workload_identity(
    observation: &AutotuneObservation,
) -> Vec<CandidateClassCount> {
    observation
        .workload_identity
        .as_ref()
        .map(|identity| {
            identity
                .class_distribution
                .iter()
                .filter_map(|(class, count)| {
                    TaskClass::from_str_opt(class).map(|class| CandidateClassCount {
                        class,
                        count: *count,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn target_executable_inode_from_snapshot(
    observation: &AutotuneObservation,
    snapshot: &TargetSnapshot,
) -> (Option<u64>, Option<u64>) {
    let Some(target_root_pid) = observation.target_root_pid else {
        return (None, None);
    };

    for task in snapshot.tasks.values() {
        if task.process_pid == target_root_pid || task.tid == target_root_pid {
            return (task.exe_dev, task.exe_ino);
        }
    }

    (None, None)
}

fn class_distribution_from_snapshot(snapshot: &TargetSnapshot) -> Vec<CandidateClassCount> {
    let mut counts = BTreeMap::<TaskClass, usize>::new();

    for task in snapshot.tasks.values() {
        *counts.entry(task.class).or_default() += 1;
    }

    counts
        .into_iter()
        .map(|(class, count)| CandidateClassCount { class, count })
        .collect()
}

fn normalize_class_distribution(
    distribution: Vec<CandidateClassCount>,
) -> Vec<CandidateClassCount> {
    let mut counts = BTreeMap::<TaskClass, usize>::new();

    for item in distribution {
        if item.count == 0 {
            continue;
        }

        *counts.entry(item.class).or_default() += item.count;
    }

    counts
        .into_iter()
        .map(|(class, count)| CandidateClassCount { class, count })
        .collect()
}

fn normalize_optional_string(value: Option<&str>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn normalize_owned_reason(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn score_delta_i64(baseline_score_total: Option<u64>, current_score_total: Option<u64>) -> i64 {
    let (Some(baseline), Some(current)) = (baseline_score_total, current_score_total) else {
        return 0;
    };

    let delta = current as i128 - baseline as i128;
    if delta > i64::MAX as i128 {
        i64::MAX
    } else if delta < i64::MIN as i128 {
        i64::MIN
    } else {
        delta as i64
    }
}

fn score_delta_f64(
    baseline_score_total: Option<u64>,
    current_score_total: Option<u64>,
) -> Option<f64> {
    let (Some(baseline), Some(current)) = (baseline_score_total, current_score_total) else {
        return None;
    };

    Some(current as f64 - baseline as f64)
}

fn duration_from_remaining_nanos(remaining_nanos: u128) -> Duration {
    Duration::from_nanos(remaining_nanos.min(u64::MAX as u128) as u64)
}

fn stable_hash_hex(parts: &[String]) -> String {
    let mut hash = 0xcbf29ce484222325_u64;

    for part in parts {
        for byte in part.len().to_string().as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }

        hash ^= b':' as u64;
        hash = hash.wrapping_mul(0x100000001b3);

        for byte in part.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }

        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }

    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        actions::{ActionId, SafetyClass},
        autotune::{
            observation::{AutotuneObservation, WorkloadIdentity},
            state::SituationKind,
        },
        focus::FocusGroupKind,
    };

    fn fake_candidate() -> CandidateAction {
        CandidateAction::fake(
            ActionId("cpu-affinity-profile:test".to_owned()),
            SafetyClass::ReversibleLowRisk,
        )
    }

    fn context_with_distribution(
        distribution: Vec<CandidateClassCount>,
    ) -> CandidateContextHashInput {
        CandidateContextHashInput {
            workload_hash: Some("workload-a".to_owned()),
            target_exe_dev: Some(8),
            target_exe_ino: Some(99),
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
    fn memory_records_candidate_result_context_hash_score_delta_reason_and_cooldown() {
        let candidate = fake_candidate();
        let context = context_with_distribution(vec![CandidateClassCount {
            class: TaskClass::Game,
            count: 4,
        }]);
        let mut memory = CandidateMemory::default();

        let record = memory.record_result(
            &candidate,
            &context,
            1_000,
            CandidateMemoryResult::Reverted,
            Some(1_000),
            Some(1_125),
            Some(" candidate regressed ".to_owned()),
            Some(301_000_000_000),
        );

        assert_eq!(
            record.action_id,
            ActionId("cpu-affinity-profile:test".to_owned())
        );
        assert_eq!(record.candidate_name, "fake-profile");
        assert_eq!(record.last_tried_unix_nanos, 1_000);
        assert_eq!(record.result, CandidateMemoryResult::Reverted);
        assert_eq!(record.context_hash, context.context_hash());
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
        let record = memory.record_result(
            &candidate,
            &context,
            2_000,
            CandidateMemoryResult::Kept,
            Some(1_000),
            Some(800),
            None,
            Some(62_000_000_000),
        );

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
        other_context.workload_hash = Some("workload-b".to_owned());
        other_context.target_exe_ino = Some(100);
        let mut memory = CandidateMemory::default();

        memory.record_result(
            &candidate,
            &context,
            1_000,
            CandidateMemoryResult::Reverted,
            Some(100),
            Some(120),
            None,
            Some(10_000),
        );

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
    fn observation_context_uses_workload_identity_and_inventory_signature() {
        let candidate = fake_candidate();
        let observation = AutotuneObservation {
            primary_situation: SituationKind::BrowserCpuPressure,
            topology_signature: Some("inventory-a".to_owned()),
            workload_identity: Some(WorkloadIdentity {
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

        assert_eq!(context.target_exe_dev, Some(8));
        assert_eq!(context.target_exe_ino, Some(99));
        assert_eq!(context.workload_hash.as_deref(), Some("workload-a"));
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
}
