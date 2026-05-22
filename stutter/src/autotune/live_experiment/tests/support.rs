#![allow(unused_imports)]
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

use anyhow::Context;



use super::super::*;
pub(super) use crate::{
    actions::{ActionId, ActionState, RollbackToken, SafetyClass, TaskIdentity},
    autotune::{
        active_config::{RollbackVerification, verify_rollback_restored_baseline},
        apply_low_risk::apply_candidate_with_audit,
        candidate::{CandidateAction, CandidateDryRunRecord, CandidateEvidence, NiceActionPlan},
        candidate_memory::CandidateMemoryResult,
        comparison::{ExperimentDataQuality, ExperimentResult},
        controller::{
            ActiveExperiment as ControllerActiveExperiment, ControllerCandidateResultInput,
            ControllerPolicy, ControllerRuntimeState,
        },
        controller_journal::{
            ControllerJournalActionMetadata, ControllerJournalRecord, ControllerJournalState,
            default_controller_journal_path, journal_process_identity,
            write_controller_journal_applied_with_metadata,
            write_controller_journal_applying_with_metadata, write_controller_journal_record,
        },
        decision::AutotuneDecision,
        experiment::{ExperimentId, WindowScore},
        kept::{ActiveProfileState, KeptCandidateState},
        objective::{
            ObjectiveComparisonInput, ObjectiveKind, ObjectiveSignals, compare_for_objective,
        },
        observation::{ActiveConfigSnapshot, AutotuneObservation},
        quality::OnlineDataQuality,
        state::ControllerPhase,
        system_context::{SystemContextSnapshotInput, collect_system_context},
        washout::WashoutWindowConfig,
    },
    daemon::{
        DaemonPolicy,
        policy::{DaemonMode, DaemonPolicyContext},
        privilege::{
            CandidateApplyRequest, CandidatePlanRequest, PrivilegedActionService, RollbackRequest,
        },
        state::{DaemonExperimentState, DaemonRollbackState},
    },
    daemon_policy::ActionSource,
    scorer::StutterScore,
};

#[derive(Default)]
pub(super) struct FakeLiveExecutor {
    pub(super) apply_calls: usize,
    pub(super) rollback_calls: usize,
    pub(super) fail_apply: bool,
    pub(super) fail_rollback: bool,
    pub(super) post_rollback_active_config: Option<ActiveConfigSnapshot>,
}

impl LiveExperimentActionExecutor for FakeLiveExecutor {
    fn apply_candidate(
        &mut self,
        _input: &LiveExperimentManagerInput<'_>,
        _candidate: &CandidateAction,
        _experiment_id: &str,
        _observation: &AutotuneObservation,
    ) -> anyhow::Result<RollbackToken> {
        self.apply_calls += 1;

        if self.fail_apply {
            anyhow::bail!("intentional apply failure");
        }

        Ok(fake_rollback())
    }

    fn rollback_candidate(
        &mut self,
        _input: &LiveExperimentManagerInput<'_>,
        _experiment: &LiveExperiment,
        _observation: &AutotuneObservation,
    ) -> anyhow::Result<Option<ActiveConfigSnapshot>> {
        self.rollback_calls += 1;

        if self.fail_rollback {
            anyhow::bail!("intentional rollback failure");
        }

        Ok(self.post_rollback_active_config.clone())
    }
}

#[derive(Debug, Default)]
pub(super) struct FakePrivilegedService {
    pub(super) apply_calls: Mutex<usize>,
    pub(super) rollback_calls: Mutex<usize>,
}

impl FakePrivilegedService {
    pub(super) fn apply_calls(&self) -> usize {
        *self.apply_calls.lock().unwrap()
    }
}

impl PrivilegedActionService for FakePrivilegedService {
    fn dry_run_candidate(
        &self,
        request: CandidateApplyRequest,
    ) -> anyhow::Result<CandidateDryRunRecord> {
        Ok(CandidateDryRunRecord {
            candidate_name: request.plan.candidate.candidate_name().to_owned(),
            affected_tasks: 1,
            warnings: Vec::new(),
            safety_class: request.plan.candidate.safety_class(),
            eligible: true,
            reason: None,
        })
    }

    fn apply_candidate(
        &self,
        _request: CandidateApplyRequest,
    ) -> anyhow::Result<crate::daemon::privilege::ApplyResult> {
        *self.apply_calls.lock().unwrap() += 1;
        Ok(crate::daemon::privilege::ApplyResult {
            state: ActionState {
                applied: true,
                affected_tasks: 1,
                checked_tasks: 1,
                pending_changes: 1,
                warnings: Vec::new(),
            },
            rollback: RollbackToken::NiceRestore {
                records: Vec::new(),
            },
        })
    }

    fn rollback(
        &self,
        request: RollbackRequest,
    ) -> anyhow::Result<crate::daemon::privilege::RollbackResult> {
        *self.rollback_calls.lock().unwrap() += 1;
        Ok(crate::daemon::privilege::RollbackResult {
            affected_tasks: request.token.affected_tasks(),
        })
    }
}

pub(super) fn low_risk_candidate() -> CandidateAction {
    CandidateAction::fake(
        ActionId::new("fake-low-risk".to_owned()),
        SafetyClass::ReversibleLowRisk,
    )
}

pub(super) fn medium_risk_candidate() -> CandidateAction {
    CandidateAction::Nice {
        plan: NiceActionPlan {
            name: "medium-nice".to_owned(),
            action: crate::actions::nice::NiceAction {
                targets: vec![TaskIdentity {
                    tid: 42,
                    process_pid: Some(42),
                    comm: Some("game".to_owned()),
                    starttime_ticks: Some(1),
                }],
                nice: 5,
                policy: crate::actions::nice::NicePolicy::default(),
            },
            target_root_pid: Some(42),
            evidence: vec![CandidateEvidence::new("test", "medium risk", 1.0)],
            objective: ObjectiveKind::DesktopInteractivity,
        },
    }
}

pub(super) fn score(total: u64) -> WindowScore {
    WindowScore {
        started_unix_nanos: 1,
        finished_unix_nanos: 2,
        interval_count: 10,
        scored_samples: 100,
        scored_task_count: 1,
        score: StutterScore {
            total,
            frame_p99_ms: 12.0,
            frame_max_ms: 12.0,
            over_5ms: 1,
            ..StutterScore::default()
        },
    }
}

pub(super) fn observation(total: u64, now_unix_nanos: u128) -> AutotuneObservation {
    AutotuneObservation {
        now_unix_nanos,
        target_present: true,
        target_root_pid: Some(99999),
        active_target_count: 1,
        interval_count: 10,
        scored_samples: 100,
        scored_task_count: 1,
        score: StutterScore {
            total,
            frame_p99_ms: 12.0,
            frame_max_ms: 12.0,
            over_5ms: 1,
            ..StutterScore::default()
        },
        data_quality: OnlineDataQuality::High,
        objective_signals: ObjectiveSignals::from_window_score(&score(total)),
        ..AutotuneObservation::default()
    }
}

pub(super) fn live_experiment() -> LiveExperiment {
    LiveExperiment {
        experiment_id: ExperimentId::new("experiment-active"),
        candidate: low_risk_candidate(),
        safety_class: SafetyClass::ReversibleLowRisk,
        mode: DaemonMode::ApplyLowRisk,
        baseline_score: score(1_000),
        baseline_signals: ObjectiveSignals::from_window_score(&score(1_000)),
        baseline_active_config: None,
        applied_unix_nanos: 100,
        washout_until_unix_nanos: 200,
        measure_until_unix_nanos: 300,
        rollback: fake_rollback(),
    }
}

pub(super) fn nice_live_experiment_with_baseline_config() -> LiveExperiment {
    LiveExperiment {
        experiment_id: ExperimentId::new("experiment-nice"),
        candidate: medium_risk_candidate(),
        safety_class: SafetyClass::ReversibleMediumRisk,
        mode: DaemonMode::ApplyMediumRisk,
        baseline_score: score(1_000),
        baseline_signals: ObjectiveSignals::from_window_score(&score(1_000)),
        baseline_active_config: Some(active_nice_config(42, 0)),
        applied_unix_nanos: 100,
        washout_until_unix_nanos: 200,
        measure_until_unix_nanos: 300,
        rollback: RollbackToken::NiceRestore {
            records: Vec::new(),
        },
    }
}

pub(super) fn active_nice_config(tid: u32, nice: i32) -> ActiveConfigSnapshot {
    ActiveConfigSnapshot {
        nice: crate::autotune::observation::ActiveNiceSnapshot {
            per_tid: std::collections::BTreeMap::from([(tid, nice)]),
        },
        ..ActiveConfigSnapshot::default()
    }
}

pub(super) fn input(journal_path: PathBuf) -> LiveExperimentManagerInput<'static> {
    let daemon_policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
    LiveExperimentManagerInput {
        mode: DaemonMode::ApplyLowRisk,
        controller_policy: ControllerPolicy::from_daemon_policy(&daemon_policy),
        daemon_policy,
        simulate_action_effects: false,
        washout: WashoutWindowConfig::default(),
        candidate_window_seconds: 30,
        manual_restore_command: "stutter daemon emergency-restore",
        controller_journal_path: Some(journal_path),
        exit_rollback_registry: None,
        privileged_action_service: None,
    }
}

pub(super) fn medium_input<'a>(
    journal_path: PathBuf,
    service: Option<&'a dyn PrivilegedActionService>,
) -> LiveExperimentManagerInput<'a> {
    let daemon_policy = DaemonPolicy::apply_medium_risk(ActionSource::Test);
    LiveExperimentManagerInput {
        mode: DaemonMode::ApplyMediumRisk,
        controller_policy: ControllerPolicy::from_daemon_policy(&daemon_policy),
        daemon_policy,
        simulate_action_effects: false,
        washout: WashoutWindowConfig::default(),
        candidate_window_seconds: 30,
        manual_restore_command: "stutter daemon emergency-restore",
        controller_journal_path: Some(journal_path),
        exit_rollback_registry: None,
        privileged_action_service: service,
    }
}

#[test]
fn medium_risk_experiment_state_uses_actual_mode_and_safety_class() {
    let journal_path = temp_journal_path("medium-state");
    let input = medium_input(journal_path.clone(), None);
    let mut manager = LiveExperimentManager::new();
    let mut controller_state = ControllerRuntimeState::default();
    let mut active_profile_state = ActiveProfileState::default();
    let mut executor = FakeLiveExecutor::default();
    let observation = observation(1_000, 1_000_000_000);

    let outcome = manager
        .apply_decision_side_effects_with_executor(
            input,
            LiveExperimentRuntimeState {
                controller_state: &mut controller_state,
                active_profile_state: &mut active_profile_state,
            },
            &observation,
            &AutotuneDecision::StartExperiment {
                candidate: medium_risk_candidate(),
                reason: "medium candidate passed gate".to_owned(),
            },
            "medium candidate passed gate",
            &mut executor,
        )
        .unwrap();

    let experiment = manager.current_experiment().unwrap();
    assert_eq!(experiment.mode, DaemonMode::ApplyMediumRisk);
    assert_eq!(experiment.safety_class, SafetyClass::ReversibleMediumRisk);
    assert_eq!(
        manager
            .daemon_experiment_state()
            .map(|state| (state.mode, state.safety_class)),
        Some((
            DaemonMode::ApplyMediumRisk,
            SafetyClass::ReversibleMediumRisk
        ))
    );
    assert_eq!(
        manager
            .daemon_rollback_state("stutter daemon emergency-restore")
            .map(|state| (state.mode, state.safety_class)),
        Some((
            DaemonMode::ApplyMediumRisk,
            SafetyClass::ReversibleMediumRisk
        ))
    );
    assert_eq!(
        outcome
            .history_context
            .as_ref()
            .map(|context| (context.mode, context.safety_class.clone())),
        Some((
            DaemonMode::ApplyMediumRisk,
            SafetyClass::ReversibleMediumRisk
        ))
    );

    let journal =
        crate::autotune::controller_journal::read_controller_journal(&journal_path).unwrap();
    assert_eq!(journal.mode, Some(DaemonMode::ApplyMediumRisk));
    assert_eq!(
        journal.safety_class,
        Some(SafetyClass::ReversibleMediumRisk)
    );
}

pub(super) fn decision_reason(decision: &AutotuneDecision) -> String {
    match decision {
        AutotuneDecision::Noop { reason }
        | AutotuneDecision::Suggest { reason, .. }
        | AutotuneDecision::StartExperiment { reason, .. }
        | AutotuneDecision::KeepCurrent { reason, .. }
        | AutotuneDecision::Revert { reason, .. }
        | AutotuneDecision::EnterCooldown { reason, .. }
        | AutotuneDecision::Fault { reason } => reason.clone(),
    }
}

pub(super) fn temp_journal_path(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-live-experiment-test-{name}-{}-{}",
        std::process::id(),
        crate::audit::unix_nanos_now()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("controller_journal.json")
}
pub(super) fn fake_rollback() -> RollbackToken {
    RollbackToken::CpuAffinityRestoreFile {
        path: PathBuf::from("/tmp/stutter-test-rollback.json"),
        affected_tasks: 1,
    }
}
