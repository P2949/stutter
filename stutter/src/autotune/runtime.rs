use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Context;
use serde::Serialize;
use tokio::sync::{mpsc, oneshot};

use crate::{
    actions::{RollbackToken, SafetyClass, runner::ActionRunPolicy},
    autotune::{
        active_config::{ActiveConfigCollectorInput, collect_active_config},
        apply::executor_for_candidate,
        apply_low_risk::apply_candidate_with_audit,
        candidate::{CandidateAction, CandidateDryRunRecord},
        candidate_memory::CandidateMemoryResult,
        comparison::{ExperimentDataQuality, ExperimentResult},
        controller::{
            ActiveExperiment as ControllerActiveExperiment, ControllerPolicy,
            ControllerRuntimeState, decide_autotune_transition,
        },
        controller_journal::{
            ControllerJournalActionMetadata, ControllerJournalRecord, ControllerJournalState,
            default_controller_journal_path, journal_process_identity,
            write_controller_journal_applied_with_metadata,
            write_controller_journal_applying_with_metadata, write_controller_journal_record,
        },
        decision::AutotuneDecision,
        experiment::{ExperimentId, WindowScore},
        history::{
            AutotuneDecisionSummary, AutotuneHistoryEvent, AutotuneMode as HistoryAutotuneMode,
            ControllerPhase as HistoryControllerPhase, ObservationSummary,
            SituationKind as HistorySituationKind, TargetIdentity, append_autotune_history_event,
            default_autotune_history_path,
        },
        kept::{ActiveProfileState, KeptCandidateState},
        objective::{ObjectiveComparisonInput, compare_for_objective},
        observation::{
            ActiveTaskSnapshot, AutotuneObservation, ProtectedTask, WorkloadIdentity,
            is_protected_task_class, protected_task_reason,
        },
        planner::{CandidatePlanner, PlanResult, PlannerInput},
        quality::{OnlineDataQuality, OnlineDataQualityPolicy},
        rolling_window::{RollingWindow, WindowScore as RuntimeWindowScore},
        state::{ControllerPhase, SituationKind},
        washout::WashoutWindowConfig,
    },
    config::model::MonitorConfig,
    daemon::{
        ActionSource, DAEMON_STATE_SCHEMA_VERSION, DaemonConfig, DaemonDecisionState,
        DaemonDegradedStatus, DaemonExperimentState, DaemonFaultState, DaemonMode, DaemonPhase,
        DaemonPolicy, DaemonPolicyBuildInput, DaemonRollbackState, DaemonState, DaemonTargetState,
        DaemonWorkloadProfile, SystemHealthInputs, SystemHealthSnapshot, SystemHealthThresholds,
        build_daemon_policy, evaluate_system_health,
        state::{
            DaemonProfileEnvironment, DaemonProfileMemory, DaemonProfilePartition,
            daemon_profile_stable_hash,
        },
    },
    diagnosis::LiveDiagnosisEntry,
    ebpf_loader::DropCountersSnapshot,
    focus::FocusGroupKind,
    process_tree::TaskInfo,
    profiles::Profile,
    scorer::StutterScore,
    session_events::MonitorEvent,
};

pub const DEFAULT_RUNTIME_WINDOW_SECONDS: u64 = 30;
pub const DEFAULT_RECENT_DIAGNOSIS_LIMIT: usize = 16;

const DAEMON_EMERGENCY_RESTORE_COMMAND: &str = "stutter daemon emergency-restore";

#[derive(Clone, Debug)]
pub struct AutotuneRuntimeConfig {
    pub daemon_config: DaemonConfig,
    pub daemon_policy: DaemonPolicy,
    pub controller_id: String,
    pub decision_log: Option<PathBuf>,
    pub history_log: Option<PathBuf>,
    pub window_seconds: u64,
    pub candidate_window_seconds: u64,
    pub profiles: Vec<Profile>,
    pub online_data_quality_policy: OnlineDataQualityPolicy,
    pub washout: WashoutWindowConfig,
    pub simulated_candidates: Vec<CandidateAction>,
    pub simulate_action_effects: bool,
}

pub fn daemon_config_for_runtime_mode(
    mode: DaemonMode,
    source: ActionSource,
    tree_pid: Option<u32>,
    watch_process: Option<String>,
) -> DaemonConfig {
    let mut config = DaemonConfig {
        mode,
        source,
        ..DaemonConfig::default()
    };
    if let Some(tree_pid) = tree_pid {
        config.target.tree_pids.push(tree_pid);
    }
    config.target.watch_process = watch_process;
    config.target.require_explicit_target = mode.supports_apply();
    config.safety.min_confidence = crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE;
    config.autotune.candidate_window_seconds = DEFAULT_RUNTIME_WINDOW_SECONDS;
    config.autotune.washout_seconds = crate::autotune::washout::DEFAULT_WASHOUT_SECONDS;
    config
}

impl AutotuneRuntimeConfig {
    pub fn observe(
        decision_log: Option<PathBuf>,
        tree_pid: Option<u32>,
        watch_process: Option<String>,
    ) -> Self {
        Self::for_mode(
            DaemonMode::Observe,
            ActionSource::AutotuneRuntime,
            decision_log,
            tree_pid,
            watch_process,
        )
    }

    pub fn suggest(
        decision_log: Option<PathBuf>,
        tree_pid: Option<u32>,
        watch_process: Option<String>,
    ) -> Self {
        Self::for_mode(
            DaemonMode::Suggest,
            ActionSource::AutotuneRuntime,
            decision_log,
            tree_pid,
            watch_process,
        )
    }

    pub fn apply_low_risk(
        decision_log: Option<PathBuf>,
        tree_pid: Option<u32>,
        watch_process: Option<String>,
    ) -> Self {
        Self::for_mode(
            DaemonMode::ApplyLowRisk,
            ActionSource::AutotuneRuntime,
            decision_log,
            tree_pid,
            watch_process,
        )
    }

    pub fn from_daemon_config(daemon_config: DaemonConfig, decision_log: Option<PathBuf>) -> Self {
        let daemon_policy = build_daemon_policy(DaemonPolicyBuildInput {
            config: &daemon_config,
            remote_context: None,
        });
        Self::from_daemon_parts(daemon_config, daemon_policy, decision_log)
    }

    pub fn from_daemon_parts(
        daemon_config: DaemonConfig,
        daemon_policy: DaemonPolicy,
        decision_log: Option<PathBuf>,
    ) -> Self {
        let candidate_window_seconds = daemon_config.autotune.candidate_window_seconds.max(1);

        Self {
            daemon_config,
            daemon_policy,
            controller_id: "local-autotune".to_owned(),
            decision_log,
            history_log: Some(default_autotune_history_path()),
            window_seconds: DEFAULT_RUNTIME_WINDOW_SECONDS,
            candidate_window_seconds,
            profiles: Vec::new(),
            online_data_quality_policy: OnlineDataQualityPolicy::default(),
            washout: WashoutWindowConfig::default(),
            simulated_candidates: Vec::new(),
            simulate_action_effects: false,
        }
    }

    fn for_mode(
        mode: DaemonMode,
        source: ActionSource,
        decision_log: Option<PathBuf>,
        tree_pid: Option<u32>,
        watch_process: Option<String>,
    ) -> Self {
        let daemon_config = daemon_config_for_runtime_mode(mode, source, tree_pid, watch_process);
        Self::from_daemon_config(daemon_config, decision_log)
    }

    pub fn with_profiles(mut self, profiles: Vec<Profile>) -> Self {
        self.profiles = profiles;
        self
    }

    pub fn with_candidate_window_seconds(mut self, seconds: u64) -> Self {
        let seconds = seconds.max(1);
        self.candidate_window_seconds = seconds;
        self.daemon_config.autotune.candidate_window_seconds = seconds;
        self
    }

    pub fn with_online_data_quality_policy(mut self, policy: OnlineDataQualityPolicy) -> Self {
        self.online_data_quality_policy = policy;
        self
    }

    pub fn with_min_focus_confidence(mut self, value: f32) -> Self {
        self.daemon_config.safety.min_confidence = value.clamp(0.0, 1.0);
        self.refresh_daemon_policy();
        self
    }

    pub fn with_washout(mut self, seconds: u64, verify_interval_ms: u64) -> Self {
        self.washout = WashoutWindowConfig::default().with_washout(seconds, verify_interval_ms);
        self.daemon_config.autotune.washout_seconds = seconds;
        self
    }

    pub fn with_simulated_candidates(mut self, candidates: Vec<CandidateAction>) -> Self {
        self.simulated_candidates = candidates;
        self
    }

    pub fn with_simulated_action_effects(mut self) -> Self {
        self.simulate_action_effects = true;
        self
    }

    pub fn mode(&self) -> DaemonMode {
        self.daemon_config.mode
    }

    pub fn tree_pid(&self) -> Option<u32> {
        self.daemon_config.target.tree_pids.first().copied()
    }

    pub fn watch_process(&self) -> Option<&str> {
        self.daemon_config.target.watch_process.as_deref()
    }

    fn refresh_daemon_policy(&mut self) {
        self.daemon_policy = build_daemon_policy(DaemonPolicyBuildInput {
            config: &self.daemon_config,
            remote_context: None,
        });
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct AutotuneDecisionStreamEntry {
    pub unix_nanos: u128,
    pub phase: String,
    pub mode: String,
    pub focus_kind: Option<String>,
    pub focus_confidence: f32,
    pub target_root_pid: Option<u32>,
    pub active_target_count: usize,
    pub situation: String,
    pub situation_confidence: f32,
    pub situation_evidence: Vec<String>,
    pub situation_blockers: Vec<String>,
    pub protected_tasks_count: usize,
    pub candidate_count: usize,
    pub top_denied_reason: Option<String>,
    pub score_total: u64,
    pub data_quality: String,
    pub data_quality_reason_codes: Vec<String>,
    pub decision: String,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct RuntimeFocusState {
    pub kind: FocusGroupKind,
    pub root_pids: Vec<u32>,
    pub member_pids: Vec<u32>,
    pub confidence: f32,
    pub score: f32,
    pub situation: SituationKind,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct OnlineAutotuneController {
    pub policy: ControllerPolicy,
    pub state: ControllerRuntimeState,
    pub window: RollingWindow,
    pub active_profile_state: ActiveProfileState,
}

impl OnlineAutotuneController {
    pub fn new(daemon_policy: DaemonPolicy, window_seconds: u64) -> Self {
        Self {
            policy: ControllerPolicy::from_daemon_policy(&daemon_policy),
            state: ControllerRuntimeState::default(),
            window: RollingWindow::new(Duration::from_secs(window_seconds)),
            active_profile_state: ActiveProfileState::default(),
        }
    }
}

#[derive(Clone, Debug)]
struct RuntimeHistoryContext {
    experiment_id: String,
    action_id: String,
    candidate_name: String,
    action_kind: String,
    score_before: Option<WindowScore>,
    score_after: Option<WindowScore>,
    rollback_performed: bool,
    rollback_policy: String,
    cooldown_until_unix_nanos: Option<u128>,
    manual_restore_command: Option<String>,
}

impl RuntimeHistoryContext {
    fn rollback_policy_with_metadata(&self) -> String {
        let mut parts = vec![self.rollback_policy.clone()];

        if let Some(cooldown_until_unix_nanos) = self.cooldown_until_unix_nanos {
            parts.push(format!(
                "cooldown_until_unix_nanos={cooldown_until_unix_nanos}"
            ));
        }

        if let Some(manual_restore_command) = self.manual_restore_command.as_deref() {
            parts.push(format!(
                "manual_restore_command={}",
                manual_restore_command.replace(' ', "_")
            ));
        }

        parts.join(";")
    }
}

#[derive(Clone, Debug)]
pub struct LiveLowRiskExperiment {
    pub experiment_id: ExperimentId,
    pub candidate: CandidateAction,
    pub baseline_score: WindowScore,
    pub applied_unix_nanos: u128,
    pub washout_until_unix_nanos: u128,
    pub measure_until_unix_nanos: u128,
    pub rollback: RollbackToken,
}

impl LiveLowRiskExperiment {
    pub fn candidate_name(&self) -> &str {
        self.candidate.profile_name()
    }

    pub fn action_id(&self) -> String {
        self.candidate.action_id().0
    }
}

#[derive(Clone, Debug)]
pub struct RuntimeTargetState {
    pub root_pid: Option<u32>,
    pub active_targets: usize,
    pub target_comm: Option<String>,
    pub active_tasks: BTreeMap<u32, TaskInfo>,
}

impl RuntimeTargetState {
    fn new(root_pid: Option<u32>) -> Self {
        Self {
            root_pid,
            active_targets: 0,
            target_comm: None,
            active_tasks: BTreeMap::new(),
        }
    }
}

#[derive(Debug)]
pub struct AutotuneRuntime {
    config: AutotuneRuntimeConfig,
    controller: OnlineAutotuneController,
    latest_focus: Option<RuntimeFocusState>,
    target_state: RuntimeTargetState,
    latest_drop_counters: DropCountersSnapshot,
    recent_diagnoses: VecDeque<LiveDiagnosisEntry>,
    live_low_risk_experiment: Option<LiveLowRiskExperiment>,
    last_observation: AutotuneObservation,
    last_decision: Option<AutotuneDecisionStreamEntry>,
    last_plan_result: Option<PlanResult>,
    pending_history_context: Option<RuntimeHistoryContext>,
}

fn top_denied_reason_for_plan(plan: &PlanResult) -> Option<String> {
    plan.evaluations
        .iter()
        .find(|evaluation| !evaluation.eligible)
        .and_then(|evaluation| {
            evaluation
                .deny_reasons
                .first()
                .map(|reason| format!("{reason:?}"))
                .or_else(|| evaluation.deny_messages.first().cloned())
        })
        .or_else(|| plan.no_action_reason.clone())
}

impl AutotuneRuntime {
    pub fn new(config: AutotuneRuntimeConfig) -> Self {
        let daemon_policy = config.daemon_policy.clone();
        let window_seconds = config.window_seconds;
        let controller = OnlineAutotuneController::new(daemon_policy, window_seconds);

        Self {
            target_state: RuntimeTargetState::new(config.tree_pid()),
            controller,
            latest_focus: None,
            latest_drop_counters: DropCountersSnapshot::default(),
            recent_diagnoses: VecDeque::new(),
            live_low_risk_experiment: None,
            last_observation: AutotuneObservation::default(),
            last_decision: None,
            last_plan_result: None,
            pending_history_context: None,
            config,
        }
    }

    pub fn on_event(
        &mut self,
        event: MonitorEvent,
    ) -> anyhow::Result<Option<AutotuneDecisionStreamEntry>> {
        match event {
            MonitorEvent::TargetSnapshot { active_targets, .. } => {
                self.update_target_snapshot(&active_targets);
            }
            MonitorEvent::Interval {
                records,
                drop_counters,
                ..
            } => {
                self.latest_drop_counters = drop_counters;
                self.controller.window.push_intervals(records);
                return self.evaluate_and_emit(None);
            }
            MonitorEvent::Frame { event } => {
                self.controller.window.push_frame(*event);
            }
            MonitorEvent::LiveDiagnosis { entry } => {
                self.push_diagnosis(*entry);
            }
            MonitorEvent::FocusChanged {
                new_kind,
                root_pids,
                member_pids,
                confidence,
                score,
                situation,
                reasons,
                ..
            } => {
                self.controller.window.clear();
                if self.live_low_risk_experiment.is_some() {
                    self.rollback_live_experiment(
                        crate::audit::unix_nanos_now(),
                        "focus changed during active low-risk experiment",
                    )?;
                }
                self.latest_focus = Some(RuntimeFocusState {
                    kind: new_kind,
                    root_pids: root_pids.clone(),
                    member_pids,
                    confidence,
                    score,
                    situation,
                    reasons,
                });
                self.target_state.root_pid = root_pids.first().copied().or(self.config.tree_pid());
                return self.evaluate_and_emit(Some("focus changed; measurement window reset"));
            }
            MonitorEvent::FocusCleared { reason, .. } => {
                self.controller.window.clear();
                if self.live_low_risk_experiment.is_some() {
                    self.rollback_live_experiment(
                        crate::audit::unix_nanos_now(),
                        "focus cleared during active low-risk experiment",
                    )?;
                }
                self.latest_focus = None;
                self.target_state.root_pid = self.config.tree_pid();
                return self.evaluate_and_emit(Some(&reason));
            }
            MonitorEvent::DataQualityWarning { message } => {
                return self.evaluate_and_emit(Some(&message));
            }
            MonitorEvent::Finished { reason } => {
                return self.evaluate_and_emit(Some(&reason));
            }
            MonitorEvent::Alert { .. }
            | MonitorEvent::MigrationEvent { .. }
            | MonitorEvent::CpuFreqSample { .. }
            | MonitorEvent::ForegroundEvent { .. }
            | MonitorEvent::SchedulerSample { .. }
            | MonitorEvent::Spike { .. }
            | MonitorEvent::GpuSample { .. }
            | MonitorEvent::IrqEvent { .. }
            | MonitorEvent::IoEvent { .. }
            | MonitorEvent::ScxEvent { .. }
            | MonitorEvent::Exec { .. } => {}
        }

        Ok(None)
    }

    pub fn observation(&self) -> AutotuneObservation {
        self.last_observation.clone()
    }

    pub fn last_decision(&self) -> Option<&AutotuneDecisionStreamEntry> {
        self.last_decision.as_ref()
    }

    pub fn has_active_experiment(&self) -> bool {
        self.live_low_risk_experiment.is_some()
    }

    pub fn rollback_on_stop(&mut self, reason: &str) -> anyhow::Result<Option<DaemonState>> {
        if !self.has_active_experiment() {
            return Ok(None);
        }

        self.rollback_live_experiment(crate::audit::unix_nanos_now(), reason)?;

        Ok(Some(self.daemon_state_snapshot()))
    }

    pub fn daemon_state_snapshot(&self) -> DaemonState {
        let active_experiment =
            self.live_low_risk_experiment
                .as_ref()
                .map(|experiment| DaemonExperimentState {
                    experiment_id: experiment.experiment_id.as_str().to_owned(),
                    action_id: experiment.action_id(),
                    candidate_name: Some(experiment.candidate_name().to_owned()),
                    safety_class: experiment.candidate.safety_class(),
                    started_unix_nanos: Some(experiment.applied_unix_nanos),
                });

        let active_rollback =
            self.live_low_risk_experiment
                .as_ref()
                .map(|experiment| DaemonRollbackState {
                    action_id: experiment.action_id(),
                    rollback_available: true,
                    token: Some(experiment.rollback.clone()),
                    manual_restore_command: Some(DAEMON_EMERGENCY_RESTORE_COMMAND.to_owned()),
                });

        let active_target = self.daemon_active_target_snapshot();
        let profile_memory = self.daemon_profile_memory_snapshot(active_target.as_ref());

        DaemonState {
            schema_version: DAEMON_STATE_SCHEMA_VERSION,
            mode: self.config.mode(),
            phase: daemon_phase_from_controller_phase(self.controller.state.phase),
            cooldown_until_unix_nanos: self.controller.state.cooldown_until_unix_nanos,
            active_target,
            active_experiment,
            active_rollback,
            last_decision: self
                .last_decision
                .as_ref()
                .map(|decision| DaemonDecisionState {
                    decision: decision.decision.clone(),
                    reason: decision.reason.clone(),
                    unix_nanos: Some(decision.unix_nanos),
                    score_total: Some(decision.score_total),
                    candidate_count: Some(decision.candidate_count),
                    top_denied_reason: decision.top_denied_reason.clone(),
                    situation: Some(decision.situation.clone()),
                    focus_kind: decision.focus_kind.clone(),
                }),
            health: self.daemon_health_snapshot(),
            degraded: self.daemon_degraded_statuses(),
            faulted: self.daemon_fault_state(),
            profile_memory,
        }
    }

    fn daemon_profile_memory_snapshot(
        &self,
        active_target: Option<&DaemonTargetState>,
    ) -> DaemonProfileMemory {
        let environment = DaemonProfileEnvironment::current();
        let workload_identity_hash = daemon_profile_workload_identity_hash(active_target);
        let workload_label = active_target
            .and_then(|target| target.comm.clone())
            .or_else(|| self.target_state.target_comm.clone());
        let profiles = self
            .controller
            .state
            .candidate_memory
            .records
            .iter()
            .filter(|record| record.result == CandidateMemoryResult::Kept)
            .map(|record| {
                let (action_kind, safety_class) =
                    daemon_profile_action_kind_and_safety_class(&record.action_id.0);
                DaemonWorkloadProfile {
                    workload_identity_hash: workload_identity_hash.clone(),
                    workload_label: workload_label.clone(),
                    candidate_name: record.candidate_name.clone(),
                    action_id: record.action_id.0.clone(),
                    action_kind,
                    safety_class,
                    kept_unix_nanos: record.last_tried_unix_nanos,
                    last_validated_unix_nanos: Some(record.last_tried_unix_nanos),
                    baseline_score_total: None,
                    candidate_score_total: None,
                    score_delta: record.score_delta,
                    confidence_milli: daemon_profile_confidence_milli(record.score_delta),
                    environment: environment.clone(),
                    partition: DaemonProfilePartition {
                        scheduler_label: environment.scheduler_label.clone(),
                        ..DaemonProfilePartition::default()
                    },
                }
            })
            .collect();

        DaemonProfileMemory { profiles }
    }

    fn daemon_health_snapshot(&self) -> SystemHealthSnapshot {
        let thresholds = SystemHealthThresholds {
            max_ebpf_dropped_events: self
                .config
                .online_data_quality_policy
                .max_drop_counter_total,
            ..SystemHealthThresholds::default()
        };

        evaluate_system_health(
            SystemHealthInputs {
                ebpf_dropped_events: self.latest_drop_counters.total(),
                ..SystemHealthInputs::default()
            },
            &thresholds,
        )
    }

    fn daemon_active_target_snapshot(&self) -> Option<DaemonTargetState> {
        let focus = self.latest_focus.as_ref();
        let root_pid = self
            .target_state
            .root_pid
            .or_else(|| focus.and_then(|focus| focus.root_pids.first().copied()));
        let active_targets = if self.target_state.active_targets > 0 {
            self.target_state.active_targets
        } else {
            focus.map(|focus| focus.member_pids.len()).unwrap_or(0)
        };
        let comm = self.target_state.target_comm.clone();

        if root_pid.is_none() && active_targets == 0 && comm.is_none() {
            return None;
        }

        Some(DaemonTargetState {
            root_pid,
            active_targets,
            comm,
        })
    }

    fn daemon_degraded_statuses(&self) -> Vec<DaemonDegradedStatus> {
        let mut degraded = Vec::new();

        if self.last_observation.data_quality.is_low() {
            let data_quality_reason_codes =
                self.last_observation.data_quality.reason_code_strings();
            let message = if data_quality_reason_codes.is_empty() {
                data_quality_label(&self.last_observation.data_quality)
            } else {
                format!(
                    "{} reason_codes={}",
                    data_quality_label(&self.last_observation.data_quality),
                    data_quality_reason_codes.join(",")
                )
            };
            degraded.push(DaemonDegradedStatus {
                category: "data_quality".to_owned(),
                message,
            });
        }

        let drop_counter_total = self.latest_drop_counters.total();
        if drop_counter_total > 0 {
            degraded.push(DaemonDegradedStatus {
                category: "drop_counters".to_owned(),
                message: format!("drop counters reported {drop_counter_total} lost events"),
            });
        }

        if let Some(cooldown_until_unix_nanos) = self.controller.state.cooldown_until_unix_nanos {
            degraded.push(DaemonDegradedStatus {
                category: "cooldown".to_owned(),
                message: format!("cooldown_until_unix_nanos={cooldown_until_unix_nanos}"),
            });
        }

        for diagnosis in &self.recent_diagnoses {
            degraded.push(DaemonDegradedStatus {
                category: "recent_diagnosis".to_owned(),
                message: format!(
                    "{:?} {:?} anchored on {}",
                    diagnosis.confidence, diagnosis.cause, diagnosis.anchor_comm
                ),
            });
        }

        degraded
    }

    fn daemon_fault_state(&self) -> Option<DaemonFaultState> {
        if self.controller.state.phase != ControllerPhase::Faulted {
            return None;
        }

        Some(DaemonFaultState {
            reason: self
                .last_decision
                .as_ref()
                .map(|decision| decision.reason.clone())
                .filter(|reason| !reason.trim().is_empty())
                .unwrap_or_else(|| "controller is faulted".to_owned()),
            manual_restore_command: Some(DAEMON_EMERGENCY_RESTORE_COMMAND.to_owned()),
        })
    }

    fn update_target_snapshot(&mut self, active_targets: &BTreeMap<u32, TaskInfo>) {
        self.target_state.active_targets = active_targets.len();
        self.target_state.active_tasks = active_targets.clone();

        if self.target_state.root_pid.is_none() {
            self.target_state.root_pid = self
                .latest_focus
                .as_ref()
                .and_then(|focus| focus.root_pids.first().copied())
                .or(self.config.tree_pid());
        }

        if let Some(root_pid) = self.target_state.root_pid {
            self.target_state.target_comm = active_targets
                .get(&root_pid)
                .map(|task| task.comm.to_string())
                .or_else(|| {
                    active_targets
                        .values()
                        .next()
                        .map(|task| task.comm.to_string())
                });
        } else {
            self.target_state.target_comm = active_targets
                .values()
                .next()
                .map(|task| task.comm.to_string());
        }
    }

    fn push_diagnosis(&mut self, diagnosis: LiveDiagnosisEntry) {
        self.controller.window.push_diagnosis(diagnosis.clone());
        self.recent_diagnoses.push_back(diagnosis);
        while self.recent_diagnoses.len() > DEFAULT_RECENT_DIAGNOSIS_LIMIT {
            self.recent_diagnoses.pop_front();
        }
    }

    fn evaluate_and_emit(
        &mut self,
        forced_reason: Option<&str>,
    ) -> anyhow::Result<Option<AutotuneDecisionStreamEntry>> {
        let observation = self.build_observation();
        self.last_observation = observation.clone();

        let decision = if let Some(experiment) = &self.live_low_risk_experiment {
            if observation.now_unix_nanos < experiment.washout_until_unix_nanos {
                AutotuneDecision::Noop {
                    reason: format!(
                        "candidate '{}' washout window is still stabilizing",
                        experiment.candidate_name()
                    ),
                }
            } else if observation.now_unix_nanos < experiment.measure_until_unix_nanos {
                AutotuneDecision::Noop {
                    reason: format!(
                        "candidate '{}' measurement window is still collecting",
                        experiment.candidate_name()
                    ),
                }
            } else {
                decide_autotune_transition(
                    &self.controller.policy,
                    &self.controller.state,
                    &observation,
                    None,
                )
            }
        } else {
            let candidate = self.select_candidate_for_observation(&observation);
            decide_autotune_transition(
                &self.controller.policy,
                &self.controller.state,
                &observation,
                candidate,
            )
        };

        let reason = forced_reason
            .map(str::to_owned)
            .unwrap_or_else(|| decision_reason(&decision));

        self.apply_decision_side_effects(&observation, &decision, &reason)?;

        let stream_entry = self.stream_entry_from_decision(&observation, &decision, reason.clone());

        self.append_decision_log(&stream_entry)?;
        self.append_history(&observation, &decision, &reason)?;

        println!("{}", serde_json::to_string(&stream_entry)?);

        self.last_decision = Some(stream_entry.clone());

        Ok(Some(stream_entry))
    }

    fn build_observation(&self) -> AutotuneObservation {
        let window_score = self
            .controller
            .window
            .score_with_quality_policy(&self.config.online_data_quality_policy);
        let focus = self.latest_focus.as_ref();
        let focus_kind = focus.map(|focus| focus.kind);
        let focus_confidence = focus.map(|focus| focus.confidence).unwrap_or(0.0);
        let focus_roots = focus
            .map(|focus| focus.root_pids.clone())
            .unwrap_or_default();
        let focus_reasons = focus.map(|focus| focus.reasons.clone()).unwrap_or_default();
        let primary_situation = focus
            .map(|focus| focus.situation)
            .unwrap_or(SituationKind::Unknown);
        let system_health = self.daemon_health_snapshot();
        let capabilities = crate::daemon::capabilities::CapabilityProbe::default().probe();
        let inventory = crate::system_inventory::SystemInventory::probe();
        let active_tasks = active_task_snapshots_from_active_tasks(&self.target_state.active_tasks);
        let protected_tasks = protected_tasks_from_active_tasks(&self.target_state.active_tasks);
        let workload_identity = workload_identity_from_runtime_context(
            self.target_state
                .root_pid
                .or_else(|| focus_roots.first().copied()),
            focus_kind,
            &self.target_state.active_tasks,
        );
        let active_config_snapshot = collect_active_config(ActiveConfigCollectorInput {
            proc_root: Path::new("/proc"),
            sys_root: Path::new("/sys"),
            active_tasks: &active_tasks,
            capabilities: &capabilities,
            inventory: &inventory,
        });

        let mut observation = AutotuneObservation {
            now_unix_nanos: crate::audit::unix_nanos_now(),
            elapsed_ms: self.controller.window.latest_elapsed_ms().unwrap_or(0),
            target_present: self.target_present(&window_score),
            target_root_pid: self
                .target_state
                .root_pid
                .or_else(|| focus_roots.first().copied()),
            active_target_count: self.target_state.active_targets,
            scored_task_count: window_score.scored_task_count,
            interval_count: window_score.interval_count,
            scored_samples: window_score.scored_samples,
            score: stutter_score_from_runtime_window_score(&window_score),
            data_quality: window_score.data_quality.clone(),
            primary_situation,
            situation: Default::default(),
            focus_kind,
            focus_confidence,
            focus_roots,
            focus_reasons,
            recent_diagnoses: self.recent_diagnoses.iter().cloned().collect(),
            system_health,
            capabilities,
            topology_signature: Some(inventory.inventory_hash),
            workload_identity,
            active_tasks,
            protected_tasks,
            active_config_snapshot: Some(active_config_snapshot),
            frame_count: window_score.frame_count,
            frame_p99_ms: window_score.frame_p99_ms,
            frame_max_ms: window_score.frame_max_ms,
            drop_counter_total: self.latest_drop_counters.total(),
        };
        observation.refresh_situation_classification();
        observation
    }

    fn target_present(&self, score: &RuntimeWindowScore) -> bool {
        self.target_state.active_targets > 0 || score.scored_samples > 0
    }

    fn select_candidate_for_observation(
        &mut self,
        observation: &AutotuneObservation,
    ) -> Option<CandidateAction> {
        if self.config.mode() == DaemonMode::Observe {
            self.last_plan_result = Some(PlanResult {
                selected: None,
                evaluations: Vec::new(),
                no_action_reason: Some("observe mode does not suggest or apply".to_owned()),
            });
            return None;
        }

        if observation.data_quality.blocks_action()
            || observation.focus_is_idle_or_unknown()
            || observation.focus_has_critical_realtime_warning()
            || observation.focus_confidence < self.controller.policy.min_focus_confidence
        {
            self.last_plan_result = Some(PlanResult {
                selected: None,
                evaluations: Vec::new(),
                no_action_reason: Some(
                    "quality, focus, realtime, or confidence gate blocked planning".to_owned(),
                ),
            });
            return None;
        }

        let tree_pid = observation.target_root_pid.or(self.config.tree_pid())?;

        if !self.config.simulated_candidates.is_empty() {
            let candidates = self.config.simulated_candidates.clone();
            let records = simulated_dry_run_records(&candidates, observation.active_target_count);
            let selected = select_best_candidate_for_situation(
                &candidates,
                &records,
                observation,
                self.controller.policy.max_safety_class.clone(),
                &self.controller.state,
            );
            self.last_plan_result = Some(PlanResult {
                selected: selected.clone(),
                evaluations: Vec::new(),
                no_action_reason: selected
                    .is_none()
                    .then(|| "no simulated candidate selected".to_owned()),
            });
            return selected;
        }

        let mut observation = observation.clone();
        if observation.target_root_pid.is_none() {
            observation.target_root_pid = Some(tree_pid);
        }
        let planner = CandidatePlanner::default_for_policy(&self.config.daemon_policy);
        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &self.config.daemon_policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            controller_state: &self.controller.state,
            active_profile_state: Some(&self.controller.active_profile_state),
            profiles: &self.config.profiles,
        });
        let selected = result.selected.clone();
        self.last_plan_result = Some(result);
        selected
    }

    fn apply_decision_side_effects(
        &mut self,
        observation: &AutotuneObservation,
        decision: &AutotuneDecision,
        reason: &str,
    ) -> anyhow::Result<()> {
        match decision {
            AutotuneDecision::StartExperiment { candidate, .. } => {
                self.start_live_experiment(observation, candidate.clone(), reason)?;
            }
            AutotuneDecision::KeepCurrent { .. } => {
                self.keep_live_experiment(observation, reason)?;
            }
            AutotuneDecision::Revert { .. } => {
                self.rollback_live_experiment(observation.now_unix_nanos, reason)?;
            }
            AutotuneDecision::EnterCooldown { duration, .. } => {
                self.controller.state.phase = ControllerPhase::Cooldown;
                self.controller.state.cooldown_until_unix_nanos = Some(
                    observation
                        .now_unix_nanos
                        .saturating_add(duration.as_nanos()),
                );
            }
            AutotuneDecision::Fault { .. } => {
                if self.live_low_risk_experiment.is_some() {
                    self.rollback_live_experiment(observation.now_unix_nanos, reason)?;
                }
                self.controller.state.enter_cooldown_after_fault(
                    &self.controller.policy,
                    observation.now_unix_nanos,
                );
            }
            AutotuneDecision::Noop { .. } | AutotuneDecision::Suggest { .. } => {}
        }

        Ok(())
    }

    fn experiment_deadlines_from_now(&self, applied_unix_nanos: u128) -> (u128, u128) {
        if self.config.simulate_action_effects {
            return (applied_unix_nanos, applied_unix_nanos);
        }

        let washout_until_unix_nanos =
            applied_unix_nanos.saturating_add(self.config.washout.washout_duration().as_nanos());
        let measure_until_unix_nanos = washout_until_unix_nanos
            .saturating_add(Duration::from_secs(self.config.candidate_window_seconds).as_nanos());

        (washout_until_unix_nanos, measure_until_unix_nanos)
    }

    fn start_live_experiment(
        &mut self,
        observation: &AutotuneObservation,
        candidate: CandidateAction,
        reason: &str,
    ) -> anyhow::Result<()> {
        match self.config.mode() {
            DaemonMode::ApplyLowRisk => {
                if candidate.safety_class() != SafetyClass::ReversibleLowRisk {
                    anyhow::bail!(
                        "live apply-low-risk rejected non-low-risk candidate {} safety={:?}",
                        candidate.profile_name(),
                        candidate.safety_class()
                    );
                }
            }
            DaemonMode::ApplyMediumRisk => {
                if candidate.safety_class() > SafetyClass::ReversibleMediumRisk {
                    anyhow::bail!(
                        "live apply-medium-risk rejected non-medium-risk candidate {} safety={:?}",
                        candidate.profile_name(),
                        candidate.safety_class()
                    );
                }
            }
            _ => return Ok(()),
        }

        let baseline_score = window_score_from_observation(observation);
        validate_window_score_for_apply("baseline", &baseline_score)?;

        let experiment_id = ExperimentId::new(format!(
            "live-{}:{}:{}",
            self.config.mode().as_str(),
            candidate.profile_name(),
            observation.now_unix_nanos
        ));
        let action_id = candidate.action_id().0;
        let action_kind = candidate.action_kind().to_owned();
        let rollback = self.apply_candidate_for_runtime(
            &candidate,
            experiment_id.as_str(),
            &action_id,
            observation,
        )?;

        self.controller.state.record_candidate_attempt(
            &candidate,
            observation,
            None,
            Some(
                observation.now_unix_nanos.saturating_add(
                    self.controller
                        .policy
                        .minimum_time_between_same_action
                        .as_nanos(),
                ),
            ),
        );

        self.controller.state.phase = ControllerPhase::Measuring;
        self.controller.state.active_experiment = Some(ControllerActiveExperiment {
            experiment_id: experiment_id.clone(),
            candidate: candidate.clone(),
            baseline_score_total: baseline_score.score.total,
        });

        let (washout_until_unix_nanos, measure_until_unix_nanos) =
            self.experiment_deadlines_from_now(observation.now_unix_nanos);

        self.live_low_risk_experiment = Some(LiveLowRiskExperiment {
            experiment_id,
            candidate,
            baseline_score: baseline_score.clone(),
            applied_unix_nanos: observation.now_unix_nanos,
            washout_until_unix_nanos,
            measure_until_unix_nanos,
            rollback,
        });
        if let Some(experiment) = self.live_low_risk_experiment.as_ref() {
            self.write_controller_journal_phase_for_live_experiment(
                experiment,
                observation,
                ControllerJournalState::Measuring,
                "measurement_window_collecting",
            )?;
        }

        self.controller.window.clear();

        self.pending_history_context = Some(RuntimeHistoryContext {
            experiment_id: self
                .live_low_risk_experiment
                .as_ref()
                .map(|experiment| experiment.experiment_id.as_str().to_owned())
                .unwrap_or_else(|| "unknown-experiment".to_owned()),
            action_id: action_id.clone(),
            candidate_name: self
                .live_low_risk_experiment
                .as_ref()
                .map(|experiment| experiment.candidate.profile_name().to_owned())
                .unwrap_or_else(|| "unknown-candidate".to_owned()),
            action_kind,
            score_before: Some(baseline_score.clone()),
            score_after: None,
            rollback_performed: false,
            rollback_policy: "rollback-on-restore".to_owned(),
            cooldown_until_unix_nanos: None,
            manual_restore_command: Some(DAEMON_EMERGENCY_RESTORE_COMMAND.to_owned()),
        });

        log::info!(
            "autotune_live_experiment_started mode={} action_kind={} reason={reason}",
            self.config.mode(),
            action_id
        );

        Ok(())
    }

    fn apply_candidate_for_runtime(
        &self,
        candidate: &CandidateAction,
        experiment_id: &str,
        action_id: &str,
        observation: &AutotuneObservation,
    ) -> anyhow::Result<RollbackToken> {
        if self.config.simulate_action_effects {
            return Ok(RollbackToken::CpuAffinityRestoreFile {
                path: PathBuf::from(format!(
                    "/tmp/stutter-simulated-rollback-{experiment_id}.json"
                )),
                affected_tasks: observation.active_target_count.max(1),
            });
        }

        let journal_path = default_controller_journal_path();

        write_controller_journal_applying_with_metadata(
            &journal_path,
            experiment_id,
            action_id,
            self.controller_journal_metadata_for_candidate(
                candidate,
                observation,
                None,
                "pending_apply",
            ),
        )?;
        let applied_rollback = if self.config.mode() == DaemonMode::ApplyMediumRisk {
            let executor = executor_for_candidate(candidate.clone())?;
            let result = executor
                .apply_with_audit(ActionRunPolicy::apply_medium_risk(
                    ActionSource::AutotuneRuntime,
                    false,
                ))
                .with_context(|| {
                    format!(
                        "audited apply failed for autotune candidate '{}'",
                        candidate.candidate_name()
                    )
                })?;
            result.rollback.with_context(|| {
                format!(
                    "audited apply for autotune candidate '{}' succeeded without rollback token",
                    candidate.candidate_name()
                )
            })?
        } else {
            apply_candidate_with_audit(candidate.clone())?.rollback
        };
        write_controller_journal_applied_with_metadata(
            &journal_path,
            experiment_id,
            action_id,
            applied_rollback.clone(),
            self.controller_journal_metadata_for_candidate(
                candidate,
                observation,
                Some(applied_rollback.affected_tasks()),
                "applied_pending_verify",
            ),
        )?;

        Ok(applied_rollback)
    }

    fn controller_journal_metadata_for_candidate(
        &self,
        candidate: &CandidateAction,
        observation: &AutotuneObservation,
        affected_tasks: Option<usize>,
        verify_result: &'static str,
    ) -> ControllerJournalActionMetadata {
        let pid = observation
            .target_root_pid
            .filter(|pid| *pid != 0)
            .unwrap_or_else(|| candidate.tree_pid());
        let starttime_ticks = (pid != 0)
            .then(|| crate::process_tree::process_starttime_at(Path::new("/proc"), pid))
            .flatten();
        let active_task_count = affected_tasks.or(Some(observation.active_target_count));

        ControllerJournalActionMetadata::default()
            .with_candidate(candidate.profile_name().to_owned())
            .with_workload_identity(journal_process_identity(pid, starttime_ticks, None))
            .with_target_identity(journal_process_identity(
                pid,
                starttime_ticks,
                active_task_count,
            ))
            .with_restore_command(DAEMON_EMERGENCY_RESTORE_COMMAND)
            .with_verify_result(verify_result)
            .with_safety_class(candidate.safety_class())
    }

    fn keep_live_experiment(
        &mut self,
        observation: &AutotuneObservation,
        reason: &str,
    ) -> anyhow::Result<()> {
        let Some(experiment) = self.live_low_risk_experiment.take() else {
            return Ok(());
        };

        let candidate_score = window_score_from_observation(observation);
        validate_window_score_for_apply("candidate", &candidate_score)?;

        let result = compare_for_objective(ObjectiveComparisonInput {
            objective: experiment.candidate.objective(),
            baseline: &experiment.baseline_score,
            candidate: &candidate_score,
            data_quality: experiment_data_quality(&observation.data_quality),
            target_disappeared: !observation.target_present,
        });

        match result {
            ExperimentResult::Improved { .. } => {
                if let Err(err) = self.write_controller_journal_phase_for_live_experiment(
                    &experiment,
                    observation,
                    ControllerJournalState::Keeping,
                    "kept_pending_manual_restore",
                ) {
                    self.live_low_risk_experiment = Some(experiment);
                    return Err(err);
                }

                let kept = KeptCandidateState::new(
                    experiment.experiment_id.clone(),
                    experiment.candidate.clone(),
                    experiment.baseline_score.clone(),
                    candidate_score.clone(),
                    experiment.rollback.clone(),
                    observation.now_unix_nanos,
                    reason.to_owned(),
                );

                self.controller
                    .active_profile_state
                    .record_kept_candidate(kept, result.clone());

                self.controller.state.record_candidate_result(
                    &experiment.candidate,
                    observation,
                    None,
                    CandidateMemoryResult::Kept,
                    Some(experiment.baseline_score.score.total),
                    Some(candidate_score.score.total),
                    None,
                    Some(
                        observation
                            .now_unix_nanos
                            .saturating_add(self.controller.policy.cooldown_after_keep.as_nanos()),
                    ),
                );

                let cooldown_until_unix_nanos = observation
                    .now_unix_nanos
                    .saturating_add(self.controller.policy.cooldown_after_keep.as_nanos());

                self.pending_history_context = Some(RuntimeHistoryContext {
                    experiment_id: experiment.experiment_id.as_str().to_owned(),
                    action_id: experiment.action_id(),
                    candidate_name: experiment.candidate.profile_name().to_owned(),
                    action_kind: experiment.candidate.action_kind().to_owned(),
                    score_before: Some(experiment.baseline_score.clone()),
                    score_after: Some(candidate_score.clone()),
                    rollback_performed: false,
                    rollback_policy: "rollback-on-restore".to_owned(),
                    cooldown_until_unix_nanos: Some(cooldown_until_unix_nanos),
                    manual_restore_command: Some(DAEMON_EMERGENCY_RESTORE_COMMAND.to_owned()),
                });

                self.controller
                    .state
                    .enter_cooldown_after_keep(&self.controller.policy, observation.now_unix_nanos);
            }
            other => {
                self.live_low_risk_experiment = Some(experiment);
                self.rollback_live_experiment(
                    observation.now_unix_nanos,
                    &format!("candidate was not improved at keep point: {other:?}"),
                )?;
            }
        }

        Ok(())
    }

    fn rollback_live_experiment(
        &mut self,
        now_unix_nanos: u128,
        reason: &str,
    ) -> anyhow::Result<()> {
        let Some(experiment) = self.live_low_risk_experiment.take() else {
            return Ok(());
        };

        if let Err(err) = self.write_controller_journal_phase_for_live_experiment(
            &experiment,
            &self.last_observation,
            ControllerJournalState::Reverting,
            "rollback_in_progress",
        ) {
            self.live_low_risk_experiment = Some(experiment);
            return Err(err);
        }

        self.rollback_experiment_for_runtime(&experiment)?;
        self.write_controller_journal_phase_for_live_experiment(
            &experiment,
            &self.last_observation,
            ControllerJournalState::Reverted,
            "rollback_verified",
        )?;

        self.controller.state.record_candidate_result(
            &experiment.candidate,
            &self.last_observation,
            None,
            CandidateMemoryResult::Reverted,
            Some(experiment.baseline_score.score.total),
            Some(self.last_observation.score.total),
            Some(reason.to_owned()),
            Some(
                now_unix_nanos
                    .saturating_add(self.controller.policy.cooldown_after_revert.as_nanos()),
            ),
        );

        let cooldown_until_unix_nanos =
            now_unix_nanos.saturating_add(self.controller.policy.cooldown_after_revert.as_nanos());

        self.pending_history_context = Some(RuntimeHistoryContext {
            experiment_id: experiment.experiment_id.as_str().to_owned(),
            action_id: experiment.action_id(),
            candidate_name: experiment.candidate.profile_name().to_owned(),
            action_kind: experiment.candidate.action_kind().to_owned(),
            score_before: Some(experiment.baseline_score.clone()),
            score_after: Some(window_score_from_observation(&self.last_observation)),
            rollback_performed: true,
            rollback_policy: "rollback-performed".to_owned(),
            cooldown_until_unix_nanos: Some(cooldown_until_unix_nanos),
            manual_restore_command: Some(DAEMON_EMERGENCY_RESTORE_COMMAND.to_owned()),
        });

        self.controller
            .state
            .enter_cooldown_after_revert(&self.controller.policy, now_unix_nanos);

        Ok(())
    }

    fn controller_journal_record_for_live_experiment(
        &self,
        experiment: &LiveLowRiskExperiment,
        observation: &AutotuneObservation,
        state: ControllerJournalState,
        verify_result: &'static str,
    ) -> ControllerJournalRecord {
        let action_id = experiment.action_id();
        ControllerJournalRecord::for_phase(
            state,
            experiment.experiment_id.as_str(),
            action_id,
            Some(experiment.rollback.clone()),
        )
        .with_metadata(self.controller_journal_metadata_for_candidate(
            &experiment.candidate,
            observation,
            Some(experiment.rollback.affected_tasks()),
            verify_result,
        ))
    }

    fn write_controller_journal_phase_for_live_experiment(
        &self,
        experiment: &LiveLowRiskExperiment,
        observation: &AutotuneObservation,
        state: ControllerJournalState,
        verify_result: &'static str,
    ) -> anyhow::Result<()> {
        if self.config.simulate_action_effects {
            return Ok(());
        }

        let record = self.controller_journal_record_for_live_experiment(
            experiment,
            observation,
            state,
            verify_result,
        );
        write_controller_journal_record(&default_controller_journal_path(), &record)
    }

    fn rollback_experiment_for_runtime(
        &self,
        experiment: &LiveLowRiskExperiment,
    ) -> anyhow::Result<()> {
        if self.config.simulate_action_effects {
            return Ok(());
        }

        let executor = executor_for_candidate(experiment.candidate.clone())?;
        executor.rollback(&experiment.rollback)?;

        Ok(())
    }

    fn stream_entry_from_decision(
        &self,
        observation: &AutotuneObservation,
        decision: &AutotuneDecision,
        reason: String,
    ) -> AutotuneDecisionStreamEntry {
        AutotuneDecisionStreamEntry {
            unix_nanos: observation.now_unix_nanos,
            phase: format!("{:?}", self.controller.state.phase),
            mode: format!("{:?}", self.config.mode()),
            focus_kind: observation.focus_kind.map(|kind| format!("{kind:?}")),
            focus_confidence: observation.focus_confidence,
            target_root_pid: observation.target_root_pid,
            active_target_count: observation.active_target_count,
            situation: format!("{:?}", observation.primary_situation),
            situation_confidence: observation.situation.confidence,
            situation_evidence: observation
                .situation
                .evidence
                .iter()
                .take(5)
                .map(|evidence| {
                    format!(
                        "{}={} weight={:.2}",
                        evidence.signal, evidence.value, evidence.weight
                    )
                })
                .collect(),
            situation_blockers: observation
                .situation
                .blockers
                .iter()
                .map(|blocker| format!("{blocker:?}"))
                .collect(),
            protected_tasks_count: observation.protected_tasks.len(),
            candidate_count: self
                .last_plan_result
                .as_ref()
                .map(|plan| plan.evaluations.len())
                .unwrap_or(0),
            top_denied_reason: self
                .last_plan_result
                .as_ref()
                .and_then(top_denied_reason_for_plan),
            score_total: observation.score.total,
            data_quality: data_quality_label(&observation.data_quality),
            data_quality_reason_codes: observation.data_quality.reason_code_strings(),
            decision: decision_label(decision),
            reason,
        }
    }

    fn append_decision_log(&self, entry: &AutotuneDecisionStreamEntry) -> anyhow::Result<()> {
        let Some(path) = &self.config.decision_log else {
            return Ok(());
        };

        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }

        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        serde_json::to_writer(&mut file, entry)?;
        file.write_all(b"\n")?;

        Ok(())
    }

    fn append_history(
        &mut self,
        observation: &AutotuneObservation,
        decision: &AutotuneDecision,
        reason: &str,
    ) -> anyhow::Result<()> {
        let Some(path) = self.config.history_log.clone() else {
            return Ok(());
        };

        let score = crate::autotune::experiment::WindowScore {
            started_unix_nanos: observation.now_unix_nanos,
            finished_unix_nanos: observation.now_unix_nanos,
            interval_count: observation.interval_count,
            scored_samples: observation.scored_samples,
            scored_task_count: observation.scored_task_count,
            score: observation.score.clone(),
        };

        let target = self.target_identity_from_observation(observation);
        let context = self.pending_history_context.take();

        let candidate_name = context
            .as_ref()
            .map(|context| context.candidate_name.clone())
            .or_else(|| decision_candidate_name(decision));

        let action_kind = context
            .as_ref()
            .map(|context| context.action_kind.clone())
            .or_else(|| decision_action_kind(decision));

        let rollback_policy = context
            .as_ref()
            .map(RuntimeHistoryContext::rollback_policy_with_metadata)
            .unwrap_or_else(|| rollback_policy_for_decision(decision).to_owned());

        let mut history = AutotuneHistoryEvent::new(
            self.config.controller_id.clone(),
            history_phase(self.controller.state.phase),
            history_mode(self.config.mode()),
            target,
            history_situation(observation.primary_situation),
            ObservationSummary {
                target_present: observation.target_present,
                active_target_count: observation.active_target_count,
                scored_task_count: observation.scored_task_count,
                interval_count: observation.interval_count,
                scored_samples: observation.scored_samples,
                score_total: observation.score.total,
                over_1ms: observation.score.over_1ms,
                over_2ms: observation.score.over_2ms,
                over_5ms: observation.score.over_5ms,
                frame_p99_ms: observation.frame_p99_ms,
                frame_max_ms: observation.frame_max_ms,
                drop_counter_total: observation.drop_counter_total,
                data_quality: data_quality_label(&observation.data_quality),
            },
            AutotuneDecisionSummary {
                decision: decision_label(decision),
                candidate_name,
                action_kind,
                eligible: decision_is_eligible(decision),
                rollback_policy,
            },
            reason.to_owned(),
        );

        if let Some(context) = context.as_ref() {
            history = history
                .with_experiment_id(context.experiment_id.clone())
                .with_action_id(context.action_id.clone())
                .with_scores(context.score_before.clone(), context.score_after.clone())
                .with_rollback_performed(context.rollback_performed);
        } else {
            history = history.with_scores(None, Some(score));
        }

        append_autotune_history_event(&path, &history)?;

        self.append_followup_history_events(
            &path,
            observation,
            decision,
            reason,
            context.as_ref(),
        )?;

        Ok(())
    }

    fn target_identity_from_observation(
        &self,
        observation: &AutotuneObservation,
    ) -> Option<TargetIdentity> {
        observation.target_root_pid.map(|root_pid| TargetIdentity {
            root_pid,
            process_comm: self
                .target_state
                .target_comm
                .clone()
                .or_else(|| self.config.watch_process().map(str::to_owned))
                .unwrap_or_else(|| "unknown".to_owned()),
            process_starttime_ticks: None,
            exe_dev: None,
            exe_ino: None,
            active_task_count: observation.active_target_count,
        })
    }

    fn append_followup_history_events(
        &self,
        path: &std::path::Path,
        observation: &AutotuneObservation,
        decision: &AutotuneDecision,
        reason: &str,
        context: Option<&RuntimeHistoryContext>,
    ) -> anyhow::Result<()> {
        if let (AutotuneDecision::StartExperiment { .. }, Some(context)) = (decision, context) {
            let applied = self.lifecycle_history_event(
                observation,
                context,
                ControllerPhase::Measuring,
                "candidate_applied",
                true,
                false,
                "candidate was applied and rollback token was written to controller journal",
            );
            append_autotune_history_event(path, &applied)?;
        }

        if matches!(
            decision,
            AutotuneDecision::KeepCurrent { .. }
                | AutotuneDecision::Revert { .. }
                | AutotuneDecision::EnterCooldown { .. }
        ) && let Some(context) = context
        {
            let cooldown = self.lifecycle_history_event(
                observation,
                context,
                ControllerPhase::Cooldown,
                "cooldown_entered",
                true,
                context.rollback_performed,
                reason,
            );
            append_autotune_history_event(path, &cooldown)?;
        }

        if matches!(decision, AutotuneDecision::Fault { .. })
            && let Some(context) = context
        {
            let faulted = self.lifecycle_history_event(
                observation,
                context,
                ControllerPhase::Faulted,
                "faulted",
                false,
                context.rollback_performed,
                reason,
            );
            append_autotune_history_event(path, &faulted)?;
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn lifecycle_history_event(
        &self,
        observation: &AutotuneObservation,
        context: &RuntimeHistoryContext,
        phase: ControllerPhase,
        decision: &str,
        eligible: bool,
        rollback_performed: bool,
        reason: &str,
    ) -> AutotuneHistoryEvent {
        AutotuneHistoryEvent::new(
            self.config.controller_id.clone(),
            history_phase(phase),
            history_mode(self.config.mode()),
            self.target_identity_from_observation(observation),
            history_situation(observation.primary_situation),
            ObservationSummary {
                target_present: observation.target_present,
                active_target_count: observation.active_target_count,
                scored_task_count: observation.scored_task_count,
                interval_count: observation.interval_count,
                scored_samples: observation.scored_samples,
                score_total: observation.score.total,
                over_1ms: observation.score.over_1ms,
                over_2ms: observation.score.over_2ms,
                over_5ms: observation.score.over_5ms,
                frame_p99_ms: observation.frame_p99_ms,
                frame_max_ms: observation.frame_max_ms,
                drop_counter_total: observation.drop_counter_total,
                data_quality: data_quality_label(&observation.data_quality),
            },
            AutotuneDecisionSummary {
                decision: decision.to_owned(),
                candidate_name: Some(context.candidate_name.clone()),
                action_kind: Some(context.action_kind.clone()),
                eligible,
                rollback_policy: context.rollback_policy_with_metadata(),
            },
            reason.to_owned(),
        )
        .with_experiment_id(context.experiment_id.clone())
        .with_action_id(context.action_id.clone())
        .with_scores(context.score_before.clone(), context.score_after.clone())
        .with_rollback_performed(rollback_performed)
    }
}

#[derive(Debug)]
pub struct AutotuneControllerExit {
    pub reason: String,
    pub last_decision: Option<AutotuneDecisionStreamEntry>,
}

pub async fn run_autotune_controller_session(
    monitor_config: std::sync::Arc<MonitorConfig>,
    runtime_config: AutotuneRuntimeConfig,
    external_stop: Option<oneshot::Receiver<()>>,
    duration: Option<Duration>,
) -> anyhow::Result<AutotuneControllerExit> {
    let (event_tx, mut event_rx) = mpsc::channel::<MonitorEvent>(1024);
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let mut runtime = AutotuneRuntime::new(runtime_config);

    let (stop_task, _stop_tx_guard) = if duration.is_some() || external_stop.is_some() {
        (
            Some(tokio::spawn(async move {
                match (duration, external_stop) {
                    (Some(duration), Some(mut external_stop)) => {
                        tokio::select! {
                            _ = tokio::time::sleep(duration) => {}
                            _ = &mut external_stop => {}
                        }
                    }
                    (Some(duration), None) => {
                        tokio::time::sleep(duration).await;
                    }
                    (None, Some(mut external_stop)) => {
                        let _ = (&mut external_stop).await;
                    }
                    (None, None) => {}
                }
                let _ = stop_tx.send(());
            })),
            None,
        )
    } else {
        (None, Some(stop_tx))
    };

    let monitor_task = tokio::spawn(async move {
        crate::session::run_monitor(monitor_config, None, Some(event_tx), Some(stop_rx)).await
    });

    while let Some(event) = event_rx.recv().await {
        runtime.on_event(event)?;
    }

    if let Some(stop_task) = stop_task {
        stop_task.abort();
        let _ = stop_task.await;
    }

    let monitor_result = monitor_task.await?;
    let reason = monitor_result?;

    Ok(AutotuneControllerExit {
        reason,
        last_decision: runtime.last_decision().cloned(),
    })
}

fn select_best_candidate_for_situation(
    candidates: &[CandidateAction],
    records: &[CandidateDryRunRecord],
    observation: &AutotuneObservation,
    max_safety_class: SafetyClass,
    state: &ControllerRuntimeState,
) -> Option<CandidateAction> {
    let mut ranked = candidates
        .iter()
        .filter_map(|candidate| {
            let record = records
                .iter()
                .find(|record| record.candidate_name == candidate.profile_name())?;

            if !record.eligible || record.safety_class > max_safety_class {
                return None;
            }

            if state
                .candidate_memory
                .cooldown_remaining_for_action(&candidate.action_id(), observation.now_unix_nanos)
                .is_some()
            {
                return None;
            }

            let rank = if matches!(candidate, CandidateAction::Fake { .. }) {
                0
            } else {
                candidate_situation_rank(candidate.profile_name(), observation.primary_situation)?
            };
            Some((rank, candidate.clone()))
        })
        .collect::<Vec<_>>();

    ranked.sort_by_key(|(rank, candidate)| (*rank, candidate.profile_name().to_owned()));
    ranked.into_iter().map(|(_, candidate)| candidate).next()
}

fn candidate_situation_rank(profile_name: &str, situation: SituationKind) -> Option<u8> {
    let name = profile_name.to_ascii_lowercase();

    match situation {
        SituationKind::GameCpuSchedulerPressure | SituationKind::GameFocused => {
            if name.contains("game-isolate-render") {
                Some(0)
            } else if name.contains("avoid-smt-contention") {
                Some(1)
            } else if name.contains("wine-server-dedicated") {
                Some(2)
            } else if name.contains("helper-spread") {
                Some(3)
            } else if name.contains("game") || name.contains("wine") || name.contains("helper") {
                Some(10)
            } else {
                None
            }
        }
        SituationKind::CompositorPressure => {
            if name.contains("game-compositor-separate") {
                Some(0)
            } else if name.contains("compositor") {
                Some(1)
            } else {
                None
            }
        }
        SituationKind::CpuPressure => {
            if name.contains("avoid-smt-contention") {
                Some(0)
            } else if name.contains("helper-spread") {
                Some(1)
            } else {
                None
            }
        }
        SituationKind::GameGpuBound
        | SituationKind::ThermalOrPowerLimit
        | SituationKind::IoPressure
        | SituationKind::IrqPressure
        | SituationKind::Idle
        | SituationKind::Unknown => None,
        SituationKind::CompileLoad
        | SituationKind::CompileCpuBound
        | SituationKind::CompileLinkerPressure
        | SituationKind::BrowserFocused
        | SituationKind::BrowserCpuPressure
        | SituationKind::BrowserGpuVideo
        | SituationKind::BrowserIoPressure
        | SituationKind::MediaPlayback
        | SituationKind::Recording
        | SituationKind::VirtualMachineLoad => None,
    }
}

fn window_score_from_observation(observation: &AutotuneObservation) -> WindowScore {
    WindowScore {
        started_unix_nanos: observation.now_unix_nanos,
        finished_unix_nanos: observation.now_unix_nanos,
        interval_count: observation.interval_count,
        scored_samples: observation.scored_samples,
        scored_task_count: observation.scored_task_count,
        score: observation.score.clone(),
    }
}

fn validate_window_score_for_apply(label: &str, score: &WindowScore) -> anyhow::Result<()> {
    if score.interval_count == 0 {
        anyhow::bail!("{label} window has zero intervals");
    }

    if score.scored_samples == 0 {
        anyhow::bail!("{label} window has zero scored samples");
    }

    if score.scored_task_count == 0 {
        anyhow::bail!("{label} window has zero scored tasks");
    }

    Ok(())
}

fn experiment_data_quality(quality: &OnlineDataQuality) -> ExperimentDataQuality {
    match quality {
        OnlineDataQuality::High => ExperimentDataQuality::High,
        OnlineDataQuality::Medium { .. } => ExperimentDataQuality::Medium,
        OnlineDataQuality::Low { .. } => ExperimentDataQuality::Low,
    }
}

fn stutter_score_from_runtime_window_score(score: &RuntimeWindowScore) -> StutterScore {
    StutterScore {
        total: score.score_total,
        over_1ms: score.over_1ms,
        over_2ms: score.over_2ms,
        over_5ms: score.over_5ms,
        max_latency_ns: score.max_latency_ns,
        frame_max_ms: score.frame_max_ms,
        frame_p99_ms: score.frame_p99_ms,
        frame_over_16ms: 0,
        frame_over_33ms: 0,
        frame_over_50ms: 0,
    }
}

fn simulated_dry_run_records(
    candidates: &[CandidateAction],
    active_target_count: usize,
) -> Vec<CandidateDryRunRecord> {
    candidates
        .iter()
        .map(|candidate| CandidateDryRunRecord {
            candidate_name: candidate.profile_name().to_owned(),
            affected_tasks: active_target_count.max(1),
            warnings: Vec::new(),
            safety_class: candidate.safety_class(),
            eligible: true,
            reason: None,
        })
        .collect()
}

fn decision_reason(decision: &AutotuneDecision) -> String {
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

fn rollback_policy_for_decision(decision: &AutotuneDecision) -> &'static str {
    match decision {
        AutotuneDecision::StartExperiment { .. }
        | AutotuneDecision::KeepCurrent { .. }
        | AutotuneDecision::Revert { .. } => "rollback-on-restore",
        AutotuneDecision::Suggest { .. }
        | AutotuneDecision::Noop { .. }
        | AutotuneDecision::EnterCooldown { .. }
        | AutotuneDecision::Fault { .. } => "none",
    }
}

fn decision_is_eligible(decision: &AutotuneDecision) -> bool {
    matches!(
        decision,
        AutotuneDecision::Suggest { .. }
            | AutotuneDecision::StartExperiment { .. }
            | AutotuneDecision::KeepCurrent { .. }
            | AutotuneDecision::Revert { .. }
    )
}

fn decision_label(decision: &AutotuneDecision) -> String {
    match decision {
        AutotuneDecision::Noop { .. } => "observed".to_owned(),
        AutotuneDecision::Suggest { .. } => "suggested".to_owned(),
        AutotuneDecision::StartExperiment { .. } => "candidate_started".to_owned(),
        AutotuneDecision::KeepCurrent { .. } => "candidate_kept".to_owned(),
        AutotuneDecision::Revert { .. } => "candidate_reverted".to_owned(),
        AutotuneDecision::EnterCooldown { .. } => "cooldown_entered".to_owned(),
        AutotuneDecision::Fault { .. } => "faulted".to_owned(),
    }
}

fn decision_candidate_name(decision: &AutotuneDecision) -> Option<String> {
    match decision {
        AutotuneDecision::Suggest { candidate, .. }
        | AutotuneDecision::StartExperiment { candidate, .. } => {
            Some(candidate.profile_name().to_owned())
        }
        AutotuneDecision::Noop { .. }
        | AutotuneDecision::KeepCurrent { .. }
        | AutotuneDecision::Revert { .. }
        | AutotuneDecision::EnterCooldown { .. }
        | AutotuneDecision::Fault { .. } => None,
    }
}

fn decision_action_kind(decision: &AutotuneDecision) -> Option<String> {
    match decision {
        AutotuneDecision::Suggest { candidate, .. }
        | AutotuneDecision::StartExperiment { candidate, .. } => {
            Some(candidate.action_kind().to_owned())
        }
        AutotuneDecision::Noop { .. }
        | AutotuneDecision::KeepCurrent { .. }
        | AutotuneDecision::Revert { .. }
        | AutotuneDecision::EnterCooldown { .. }
        | AutotuneDecision::Fault { .. } => None,
    }
}

fn data_quality_label(quality: &OnlineDataQuality) -> String {
    match quality {
        OnlineDataQuality::High => "High".to_owned(),
        OnlineDataQuality::Medium { reasons } => format!("Medium: {}", reasons.join("; ")),
        OnlineDataQuality::Low { reasons } => format!("Low: {}", reasons.join("; ")),
    }
}

pub fn daemon_phase_from_controller_phase(phase: ControllerPhase) -> DaemonPhase {
    match phase {
        ControllerPhase::Disabled => DaemonPhase::Disabled,
        ControllerPhase::Observing => DaemonPhase::Observe,
        ControllerPhase::Planning => DaemonPhase::Decide,
        ControllerPhase::Applying => DaemonPhase::Apply,
        ControllerPhase::Measuring => DaemonPhase::Measure,
        ControllerPhase::Keeping => DaemonPhase::Keep,
        ControllerPhase::Reverting => DaemonPhase::Rollback,
        ControllerPhase::Cooldown => DaemonPhase::Cooldown,
        ControllerPhase::Faulted => DaemonPhase::Faulted,
    }
}

fn daemon_profile_workload_identity_hash(active_target: Option<&DaemonTargetState>) -> String {
    let root_pid = active_target
        .and_then(|target| target.root_pid)
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| "unknown".to_owned());
    let active_targets = active_target
        .map(|target| target.active_targets.to_string())
        .unwrap_or_else(|| "0".to_owned());
    let comm = active_target
        .and_then(|target| target.comm.as_deref())
        .unwrap_or("unknown");

    daemon_profile_stable_hash([root_pid.as_str(), active_targets.as_str(), comm])
}

fn daemon_profile_action_kind_and_safety_class(action_id: &str) -> (String, SafetyClass) {
    if action_id.starts_with("cpu-affinity-profile:") {
        (
            "cpu_affinity_profile".to_owned(),
            SafetyClass::ReversibleLowRisk,
        )
    } else {
        ("unknown".to_owned(), SafetyClass::ObserveOnly)
    }
}

fn daemon_profile_confidence_milli(score_delta: i64) -> u16 {
    if score_delta < 0 {
        900
    } else if score_delta == 0 {
        600
    } else {
        350
    }
}

fn history_phase(phase: ControllerPhase) -> HistoryControllerPhase {
    match phase {
        ControllerPhase::Disabled => HistoryControllerPhase::Disabled,
        ControllerPhase::Observing => HistoryControllerPhase::Observing,
        ControllerPhase::Planning => HistoryControllerPhase::Planning,
        ControllerPhase::Applying => HistoryControllerPhase::Applying,
        ControllerPhase::Measuring => HistoryControllerPhase::Measuring,
        ControllerPhase::Keeping => HistoryControllerPhase::Keeping,
        ControllerPhase::Reverting => HistoryControllerPhase::Reverting,
        ControllerPhase::Cooldown => HistoryControllerPhase::Cooldown,
        ControllerPhase::Faulted => HistoryControllerPhase::Faulted,
    }
}

fn history_mode(mode: DaemonMode) -> HistoryAutotuneMode {
    match mode {
        DaemonMode::Observe => HistoryAutotuneMode::Observe,
        DaemonMode::Suggest => HistoryAutotuneMode::Suggest,
        DaemonMode::ApplyLowRisk => HistoryAutotuneMode::ApplyLowRisk,
        DaemonMode::ApplyMediumRisk => HistoryAutotuneMode::ApplyMediumRisk,
        DaemonMode::ApplyHighRisk => HistoryAutotuneMode::ApplyHighRisk,
    }
}

fn history_situation(situation: SituationKind) -> HistorySituationKind {
    situation
}

fn protected_tasks_from_active_tasks(active_tasks: &BTreeMap<u32, TaskInfo>) -> Vec<ProtectedTask> {
    active_tasks
        .values()
        .filter(|task| is_protected_task_class(task.class))
        .map(|task| ProtectedTask {
            tid: task.tid,
            process_pid: task.process_pid,
            comm: task.comm.clone(),
            class: task.class,
            reason: protected_task_reason(task.class).to_owned(),
        })
        .collect()
}

fn active_task_snapshots_from_active_tasks(
    active_tasks: &BTreeMap<u32, TaskInfo>,
) -> Vec<ActiveTaskSnapshot> {
    active_tasks
        .values()
        .map(|task| ActiveTaskSnapshot {
            tid: task.tid,
            process_pid: task.process_pid,
            comm: task.comm.clone(),
            class: task.class,
            process_starttime_ticks: task.process_starttime_ticks,
            task_starttime_ticks: task.task_starttime_ticks,
            cgroup_path: read_proc_cgroup_path(task.tid),
        })
        .collect()
}

fn workload_identity_from_runtime_context(
    root_pid: Option<u32>,
    focus_kind: Option<FocusGroupKind>,
    active_tasks: &BTreeMap<u32, TaskInfo>,
) -> Option<WorkloadIdentity> {
    let root_pid = root_pid?;
    let root_task = active_tasks
        .values()
        .find(|task| task.process_pid == root_pid || task.tid == root_pid);
    let process_starttime_ticks = root_task
        .and_then(|task| task.process_starttime_ticks)
        .or_else(|| crate::process_tree::process_starttime_at(Path::new("/proc"), root_pid));
    let (exe_dev, exe_ino) = root_task
        .and_then(|task| Some((task.exe_dev?, task.exe_ino?)))
        .map(|(dev, ino)| (Some(dev), Some(ino)))
        .unwrap_or_else(|| exe_identity_for_pid(root_pid));
    let cgroup_path = read_proc_cgroup_path(root_pid);
    let class_distribution = class_distribution_from_tasks(active_tasks);
    let stable_hash = crate::daemon::state::daemon_profile_stable_hash([
        root_pid.to_string().as_str(),
        process_starttime_ticks
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_owned())
            .as_str(),
        exe_dev
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_owned())
            .as_str(),
        exe_ino
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_owned())
            .as_str(),
        cgroup_path.as_deref().unwrap_or(""),
        &format!("{focus_kind:?}"),
    ]);

    Some(WorkloadIdentity {
        root_pid,
        process_starttime_ticks,
        exe_dev,
        exe_ino,
        cgroup_path,
        focus_kind,
        class_distribution,
        stable_hash,
    })
}

fn exe_identity_for_pid(pid: u32) -> (Option<u64>, Option<u64>) {
    fs::metadata(Path::new("/proc").join(pid.to_string()).join("exe"))
        .map(|metadata| (Some(metadata.dev()), Some(metadata.ino())))
        .unwrap_or((None, None))
}

fn read_proc_cgroup_path(pid: u32) -> Option<String> {
    let text = fs::read_to_string(Path::new("/proc").join(pid.to_string()).join("cgroup")).ok()?;
    text.lines()
        .filter_map(|line| line.rsplit_once(':').map(|(_, path)| path.trim()))
        .filter(|path| !path.is_empty())
        .max_by_key(|path| path.len())
        .map(str::to_owned)
}

fn class_distribution_from_tasks(
    active_tasks: &BTreeMap<u32, TaskInfo>,
) -> BTreeMap<String, usize> {
    let mut classes = BTreeMap::new();
    for task in active_tasks.values() {
        *classes.entry(task.class.as_str().to_owned()).or_insert(0) += 1;
    }
    classes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        diagnosis::{Confidence, StutterCause},
        ebpf_loader::DropCountersSnapshot,
        process_tree::TaskClass,
        recorder::IntervalRecord,
    };

    fn runtime() -> AutotuneRuntime {
        let mut config = AutotuneRuntimeConfig::observe(None, Some(1234), None);
        config.history_log = None;
        AutotuneRuntime::new(config)
    }

    #[test]
    fn runtime_starts_with_default_observation() {
        let runtime = runtime();
        let observation = runtime.observation();

        assert!(!observation.target_present);
        assert_eq!(observation.target_root_pid, None);
        assert_eq!(observation.score.total, 0);
        assert!(observation.data_quality.blocks_action());
    }

    #[test]
    fn apply_medium_runtime_can_start_reversible_medium_experiment_in_simulation() {
        let daemon_config = daemon_config_for_runtime_mode(
            DaemonMode::ApplyMediumRisk,
            ActionSource::AutotuneRuntime,
            Some(1234),
            None,
        );
        let mut config = AutotuneRuntimeConfig::from_daemon_config(daemon_config, None)
            .with_simulated_action_effects();
        config.history_log = None;
        let mut runtime = AutotuneRuntime::new(config);
        let observation = high_quality_game_observation_with_focus_confidence(0.95);
        let candidate = CandidateAction::Fake {
            action_id: crate::actions::ActionId("fake-medium".to_owned()),
            safety_class: SafetyClass::ReversibleMediumRisk,
        };

        runtime
            .start_live_experiment(&observation, candidate, "test medium start")
            .unwrap();

        assert!(runtime.has_active_experiment());
        assert_eq!(runtime.controller.state.phase, ControllerPhase::Measuring);
        assert_eq!(
            runtime
                .pending_history_context
                .as_ref()
                .map(|context| context.action_kind.as_str()),
            Some("fake")
        );
    }

    fn low_risk_profile() -> crate::profiles::Profile {
        crate::profiles::Profile {
            name: "game-low-risk".to_owned(),
            rules: vec![crate::profiles::ProfileRule {
                affinity: Some(crate::affinity::CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![crate::process_tree::TaskClass::Game],
                match_comm: Vec::new(),
            }],
        }
    }

    fn high_quality_game_observation_with_focus_confidence(
        focus_confidence: f32,
    ) -> AutotuneObservation {
        AutotuneObservation {
            now_unix_nanos: 1_000_000_000,
            elapsed_ms: 30_000,
            target_present: true,
            target_root_pid: Some(1234),
            active_target_count: 1,
            scored_task_count: 1,
            interval_count: 5,
            scored_samples: 100,
            score: StutterScore {
                total: 100,
                over_1ms: 10,
                over_2ms: 5,
                over_5ms: 1,
                ..StutterScore::default()
            },
            data_quality: OnlineDataQuality::High,
            primary_situation: SituationKind::GameCpuSchedulerPressure,
            situation: Default::default(),
            focus_kind: Some(FocusGroupKind::Game),
            focus_confidence,
            focus_roots: vec![1234],
            focus_reasons: vec!["game focus selected".to_owned()],
            recent_diagnoses: Vec::new(),
            system_health: Default::default(),
            capabilities: Default::default(),
            topology_signature: None,
            workload_identity: None,
            active_tasks: Vec::new(),
            protected_tasks: Vec::new(),
            active_config_snapshot: None,
            frame_count: 100,
            frame_p99_ms: 12.0,
            frame_max_ms: 20.0,
            drop_counter_total: 0,
        }
    }

    #[test]
    fn daemon_phase_from_controller_phase_maps_all_controller_phases() {
        let cases = [
            (ControllerPhase::Disabled, DaemonPhase::Disabled),
            (ControllerPhase::Observing, DaemonPhase::Observe),
            (ControllerPhase::Planning, DaemonPhase::Decide),
            (ControllerPhase::Applying, DaemonPhase::Apply),
            (ControllerPhase::Measuring, DaemonPhase::Measure),
            (ControllerPhase::Keeping, DaemonPhase::Keep),
            (ControllerPhase::Reverting, DaemonPhase::Rollback),
            (ControllerPhase::Cooldown, DaemonPhase::Cooldown),
            (ControllerPhase::Faulted, DaemonPhase::Faulted),
        ];

        for (controller_phase, expected_daemon_phase) in cases {
            assert_eq!(
                daemon_phase_from_controller_phase(controller_phase),
                expected_daemon_phase
            );
        }
    }

    #[test]
    fn top_denied_reason_for_plan_prefers_deny_reason_enum() {
        let candidate = CandidateAction::Fake {
            action_id: crate::actions::ActionId("fake-noop".to_owned()),
            safety_class: SafetyClass::ObserveOnly,
        };
        let descriptor = candidate.descriptor();
        let evaluation = crate::autotune::planner::CandidateEvaluation {
            candidate_name: "fake-noop".to_owned(),
            action_kind: "fake".to_owned(),
            descriptor,
            provider: "test".to_owned(),
            confidence: 1.0,
            eligible: false,
            deny_reasons: vec![crate::autotune::planner::CandidateDenyReason::NoEffectiveChange],
            deny_messages: vec!["candidate would not change active configuration".to_owned()],
            evidence: Vec::new(),
            objective: crate::autotune::objective::ObjectiveKind::DesktopInteractivity,
            rank: Some(1),
            dry_run: None,
            candidate,
        };
        let plan = PlanResult {
            selected: None,
            evaluations: vec![evaluation],
            no_action_reason: None,
        };

        assert_eq!(
            top_denied_reason_for_plan(&plan).as_deref(),
            Some("NoEffectiveChange")
        );
    }

    #[test]
    fn runtime_config_stores_intent_and_permissions_in_daemon_fields() {
        let config =
            AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), Some("Game.exe".to_owned()))
                .with_min_focus_confidence(0.81)
                .with_candidate_window_seconds(45);

        assert_eq!(config.daemon_config.mode, DaemonMode::ApplyLowRisk);
        assert_eq!(config.daemon_config.source, ActionSource::AutotuneRuntime);
        assert_eq!(config.daemon_config.target.tree_pids, vec![1234]);
        assert_eq!(
            config.daemon_config.target.watch_process.as_deref(),
            Some("Game.exe")
        );
        assert!(config.daemon_config.target.require_explicit_target);
        assert_eq!(config.daemon_config.safety.min_confidence, 0.81);
        assert_eq!(config.daemon_config.autotune.candidate_window_seconds, 45);
        assert_eq!(config.daemon_policy.mode, DaemonMode::ApplyLowRisk);
        assert_eq!(
            config.daemon_policy.max_safety_class,
            SafetyClass::ReversibleLowRisk
        );
        assert_eq!(config.daemon_policy.min_confidence, 0.81);
    }

    #[test]
    fn runtime_reports_active_low_risk_experiment_state() {
        let mut runtime = runtime();

        assert!(!runtime.has_active_experiment());

        let candidate = CandidateAction::cpu_affinity_profile(low_risk_profile(), 1234);
        let baseline_score = WindowScore {
            started_unix_nanos: 100,
            finished_unix_nanos: 200,
            interval_count: 1,
            scored_samples: 100,
            scored_task_count: 1,
            score: StutterScore {
                total: 500,
                over_1ms: 10,
                over_2ms: 5,
                over_5ms: 1,
                ..StutterScore::default()
            },
        };

        runtime.live_low_risk_experiment = Some(LiveLowRiskExperiment {
            experiment_id: ExperimentId::new("experiment-active"),
            candidate,
            baseline_score,
            applied_unix_nanos: 1_000,
            washout_until_unix_nanos: 2_000,
            measure_until_unix_nanos: 3_000,
            rollback: RollbackToken::CpuAffinityRestoreFile {
                path: PathBuf::from("/tmp/stutter-active-restore.json"),
                affected_tasks: 1,
            },
        });

        assert!(runtime.has_active_experiment());
    }

    #[test]
    fn live_experiment_journal_record_carries_phase_metadata_and_rollback() {
        let runtime = runtime();
        let observation = high_quality_game_observation_with_focus_confidence(0.95);
        let candidate = CandidateAction::cpu_affinity_profile(low_risk_profile(), 1234);
        let baseline_score = WindowScore {
            started_unix_nanos: 100,
            finished_unix_nanos: 200,
            interval_count: 1,
            scored_samples: 100,
            scored_task_count: 1,
            score: StutterScore {
                total: 500,
                over_1ms: 10,
                over_2ms: 5,
                over_5ms: 1,
                ..StutterScore::default()
            },
        };
        let experiment = LiveLowRiskExperiment {
            experiment_id: ExperimentId::new("experiment-active"),
            candidate,
            baseline_score,
            applied_unix_nanos: 1_000,
            washout_until_unix_nanos: 2_000,
            measure_until_unix_nanos: 3_000,
            rollback: RollbackToken::CpuAffinityRestoreFile {
                path: PathBuf::from("/tmp/stutter-active-restore.json"),
                affected_tasks: 2,
            },
        };

        let record = runtime.controller_journal_record_for_live_experiment(
            &experiment,
            &observation,
            ControllerJournalState::Reverting,
            "rollback_in_progress",
        );

        assert_eq!(record.state(), ControllerJournalState::Reverting);
        assert_eq!(
            record.experiment_action(),
            Some(("experiment-active", "cpu-affinity-profile:game-low-risk"))
        );
        assert_eq!(record.candidate.as_deref(), Some("game-low-risk"));
        assert_eq!(
            record.target_identity.as_deref(),
            Some("pid:1234:starttime:unknown:active_tasks:2")
        );
        assert_eq!(
            record.verify_result.as_deref(),
            Some("rollback_in_progress")
        );
        assert_eq!(record.safety_class, Some(SafetyClass::ReversibleLowRisk));
        assert!(record.rollback_token().is_some());
        assert!(record.may_have_mutated_system());
    }

    #[test]
    fn runtime_rollback_on_stop_noops_without_active_experiment() {
        let mut runtime = runtime();

        let snapshot = runtime.rollback_on_stop("daemon stop").unwrap();

        assert!(snapshot.is_none());
        assert!(!runtime.has_active_experiment());
    }

    #[test]
    fn daemon_state_snapshot_serializes_live_runtime_state() {
        let mut runtime = AutotuneRuntime::new(
            AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), None)
                .with_profiles(vec![low_risk_profile()]),
        );
        let candidate = CandidateAction::cpu_affinity_profile(low_risk_profile(), 1234);
        let baseline_score = WindowScore {
            started_unix_nanos: 100,
            finished_unix_nanos: 200,
            interval_count: 1,
            scored_samples: 100,
            scored_task_count: 1,
            score: StutterScore {
                total: 500,
                over_1ms: 10,
                over_2ms: 5,
                over_5ms: 1,
                ..StutterScore::default()
            },
        };
        let rollback = RollbackToken::CpuAffinityRestoreFile {
            path: PathBuf::from("/tmp/stutter-restore.json"),
            affected_tasks: 2,
        };

        runtime.target_state = RuntimeTargetState {
            root_pid: Some(1234),
            active_targets: 2,
            target_comm: Some("game".to_owned()),
            active_tasks: BTreeMap::new(),
        };
        runtime.latest_focus = Some(RuntimeFocusState {
            kind: FocusGroupKind::Game,
            root_pids: vec![1234],
            member_pids: vec![1234, 1235],
            confidence: 0.95,
            score: 0.99,
            situation: SituationKind::GameCpuSchedulerPressure,
            reasons: vec!["game focus selected".to_owned()],
        });
        runtime.latest_drop_counters = DropCountersSnapshot {
            ringbuf_reserve_failed: 7,
            ..DropCountersSnapshot::default()
        };
        runtime.recent_diagnoses.push_back(LiveDiagnosisEntry {
            elapsed_ms: 12_345,
            cause: StutterCause::GpuBoundCandidate,
            confidence: Confidence::High,
            anchor_class: TaskClass::Game,
            anchor_comm: "game".to_owned(),
            evidence: vec!["gpu busy".to_owned()],
        });
        runtime.live_low_risk_experiment = Some(LiveLowRiskExperiment {
            experiment_id: ExperimentId::new("experiment-1"),
            candidate,
            baseline_score,
            applied_unix_nanos: 1_000,
            washout_until_unix_nanos: 2_000,
            measure_until_unix_nanos: 3_000,
            rollback,
        });
        runtime.controller.state.phase = ControllerPhase::Faulted;
        runtime.controller.state.cooldown_until_unix_nanos = Some(9_000);

        let mut observation = high_quality_game_observation_with_focus_confidence(0.95);
        observation.data_quality = OnlineDataQuality::Low {
            reasons: vec!["low scored samples".to_owned()],
        };
        observation.drop_counter_total = 7;
        observation.score.total = 999;
        runtime.last_observation = observation;
        runtime.last_decision = Some(AutotuneDecisionStreamEntry {
            unix_nanos: 8_000,
            phase: "Faulted".to_owned(),
            mode: "ApplyLowRisk".to_owned(),
            focus_kind: Some("Game".to_owned()),
            focus_confidence: 0.95,
            target_root_pid: Some(1234),
            active_target_count: 2,
            situation: "GameCpuSchedulerPressure".to_owned(),
            situation_confidence: 0.95,
            situation_evidence: Vec::new(),
            situation_blockers: Vec::new(),
            protected_tasks_count: 0,
            candidate_count: 0,
            top_denied_reason: None,
            score_total: 999,
            data_quality: "Low: low scored samples".to_owned(),
            data_quality_reason_codes: vec!["measurement_uncertain".to_owned()],
            decision: "faulted".to_owned(),
            reason: "rollback failed".to_owned(),
        });
        let kept_candidate = CandidateAction::cpu_affinity_profile(low_risk_profile(), 1234);
        runtime.controller.state.record_candidate_result(
            &kept_candidate,
            &runtime.last_observation,
            None,
            CandidateMemoryResult::Kept,
            Some(500),
            Some(400),
            None,
            None,
        );

        let snapshot = runtime.daemon_state_snapshot();
        let value = serde_json::to_value(&snapshot).unwrap();

        assert_eq!(snapshot.mode, DaemonMode::ApplyLowRisk);
        assert_eq!(snapshot.phase, DaemonPhase::Faulted);
        assert_eq!(snapshot.cooldown_until_unix_nanos, Some(9_000));
        assert_eq!(
            snapshot
                .active_target
                .as_ref()
                .and_then(|target| target.root_pid),
            Some(1234)
        );
        assert_eq!(
            snapshot
                .active_experiment
                .as_ref()
                .map(|experiment| experiment.experiment_id.as_str()),
            Some("experiment-1")
        );
        assert_eq!(
            snapshot
                .active_rollback
                .as_ref()
                .map(|rollback| rollback.rollback_available),
            Some(true)
        );
        assert_eq!(
            snapshot
                .last_decision
                .as_ref()
                .map(|decision| decision.decision.as_str()),
            Some("faulted")
        );
        assert_eq!(
            snapshot.faulted.as_ref().map(|fault| fault.reason.as_str()),
            Some("rollback failed")
        );
        assert!(
            snapshot
                .degraded
                .iter()
                .any(|status| status.category == "data_quality")
        );
        assert!(snapshot.degraded.iter().any(|status| {
            status.category == "data_quality"
                && status
                    .message
                    .contains("reason_codes=measurement_uncertain")
        }));
        assert!(
            snapshot
                .degraded
                .iter()
                .any(|status| status.category == "drop_counters")
        );
        assert!(!snapshot.health.ok_for_apply);
        assert_eq!(
            snapshot.health.reason_code.as_deref(),
            Some("drop_counters_high")
        );
        assert!(
            snapshot
                .degraded
                .iter()
                .any(|status| status.category == "cooldown")
        );
        assert!(
            snapshot
                .degraded
                .iter()
                .any(|status| status.category == "recent_diagnosis")
        );

        assert_eq!(value["schema_version"], DAEMON_STATE_SCHEMA_VERSION);
        assert_eq!(value["mode"], "apply-low-risk");
        assert_eq!(value["phase"], "faulted");
        assert_eq!(value["cooldown_until_unix_nanos"].as_u64(), Some(9_000));
        assert_eq!(value["active_target"]["comm"], "game");
        assert_eq!(
            value["active_experiment"]["action_id"],
            "cpu-affinity-profile:game-low-risk"
        );
        assert_eq!(
            value["active_rollback"]["token"]["kind"],
            "cpu-affinity-restore-file"
        );
        assert_eq!(value["last_decision"]["score_total"].as_u64(), Some(999));
        assert_eq!(
            value["active_rollback"]["manual_restore_command"],
            "stutter daemon emergency-restore"
        );
        assert_eq!(
            value["faulted"]["manual_restore_command"],
            "stutter daemon emergency-restore"
        );
        assert_eq!(snapshot.profile_memory.profiles.len(), 1);
        assert_eq!(
            snapshot.profile_memory.profiles[0].candidate_name,
            "game-low-risk"
        );
        assert_eq!(
            snapshot.profile_memory.profiles[0]
                .workload_label
                .as_deref(),
            Some("game")
        );
        assert_eq!(
            value["profile_memory"]["profiles"][0]["action_kind"],
            "cpu_affinity_profile"
        );
    }

    #[test]
    fn candidate_selection_blocks_focus_confidence_below_policy_threshold() {
        let mut runtime = AutotuneRuntime::new(
            AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), None)
                .with_profiles(vec![low_risk_profile()]),
        );
        let observation = high_quality_game_observation_with_focus_confidence(
            runtime.controller.policy.min_focus_confidence - 0.01,
        );

        let candidate = runtime.select_candidate_for_observation(&observation);

        assert!(candidate.is_none());
    }

    #[test]
    fn runtime_observation_uses_configured_online_data_quality_policy() {
        let mut runtime = AutotuneRuntime::new(
            AutotuneRuntimeConfig::observe(None, Some(1234), None).with_online_data_quality_policy(
                OnlineDataQualityPolicy {
                    min_scored_samples: 200,
                    ..OnlineDataQualityPolicy::default()
                },
            ),
        );

        for elapsed_ms in [1000, 2000, 3000, 4000, 5000] {
            runtime
                .controller
                .window
                .push_interval(crate::recorder::IntervalRecord {
                    elapsed_ms,
                    task: 42,
                    samples: 20,
                    over_1ms: 1,
                    max_ns: 2_000_000,
                    ..Default::default()
                });
        }

        let observation = runtime.build_observation();

        assert_eq!(observation.scored_samples, 100);
        assert!(observation.data_quality.is_low());
        assert!(
            observation
                .data_quality
                .reasons()
                .iter()
                .any(|reason| reason.contains("fewer than min_scored_samples"))
        );
    }

    #[test]
    fn runtime_config_defaults_to_default_online_data_quality_policy() {
        let config = AutotuneRuntimeConfig::observe(None, Some(1234), None);
        let default_policy = OnlineDataQualityPolicy::default();

        assert_eq!(
            config.online_data_quality_policy.min_scored_intervals,
            default_policy.min_scored_intervals
        );
        assert_eq!(
            config.online_data_quality_policy.min_scored_samples,
            default_policy.min_scored_samples
        );
        assert_eq!(
            config.online_data_quality_policy.max_drop_counter_total,
            default_policy.max_drop_counter_total
        );
        assert_eq!(
            config.online_data_quality_policy.frame_data_policy,
            default_policy.frame_data_policy
        );
    }

    #[test]
    fn runtime_config_defaults_and_overrides_washout_policy() {
        let default_config = AutotuneRuntimeConfig::observe(None, Some(1234), None);

        assert_eq!(
            default_config.washout.washout_seconds,
            crate::autotune::washout::DEFAULT_WASHOUT_SECONDS
        );
        assert_eq!(
            default_config.washout.verify_interval_ms,
            crate::autotune::washout::DEFAULT_WASHOUT_VERIFY_INTERVAL_MS
        );

        let custom_config =
            AutotuneRuntimeConfig::observe(None, Some(1234), None).with_washout(30, 2_000);

        assert_eq!(custom_config.washout.washout_seconds, 30);
        assert_eq!(custom_config.washout.verify_interval_ms, 2_000);

        let clamped_config =
            AutotuneRuntimeConfig::observe(None, Some(1234), None).with_washout(0, 50);

        assert_eq!(
            clamped_config.washout.washout_seconds,
            crate::autotune::washout::MIN_WASHOUT_SECONDS
        );
        assert_eq!(
            clamped_config.washout.verify_interval_ms,
            crate::autotune::washout::MIN_WASHOUT_VERIFY_INTERVAL_MS
        );
    }

    #[test]
    fn runtime_config_default_min_focus_confidence_matches_controller_default() {
        let config = AutotuneRuntimeConfig::suggest(None, None, None);
        let runtime = AutotuneRuntime::new(config.clone());

        assert_eq!(
            config.daemon_config.safety.min_confidence,
            crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE
        );
        assert_eq!(
            runtime.controller.policy.min_focus_confidence,
            crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE
        );
    }

    #[test]
    fn runtime_config_custom_min_focus_confidence_updates_controller_policy() {
        let config =
            AutotuneRuntimeConfig::suggest(None, None, None).with_min_focus_confidence(0.42);
        let runtime = AutotuneRuntime::new(config.clone());

        assert_eq!(config.daemon_config.safety.min_confidence, 0.42);
        assert_eq!(runtime.controller.policy.min_focus_confidence, 0.42);
    }

    #[test]
    fn runtime_config_min_focus_confidence_is_clamped() {
        let low = AutotuneRuntimeConfig::suggest(None, None, None).with_min_focus_confidence(-1.0);
        let high = AutotuneRuntimeConfig::suggest(None, None, None).with_min_focus_confidence(2.0);

        assert_eq!(low.daemon_config.safety.min_confidence, 0.0);
        assert_eq!(high.daemon_config.safety.min_confidence, 1.0);
    }

    #[test]
    fn runtime_washout_policy_delays_live_measurement_deadline() {
        let runtime = AutotuneRuntime::new(
            AutotuneRuntimeConfig::observe(None, Some(1234), None)
                .with_candidate_window_seconds(30)
                .with_washout(20, 2_000),
        );
        let applied_unix_nanos = 1_000_000_000_u128;

        let (washout_until_unix_nanos, measure_until_unix_nanos) =
            runtime.experiment_deadlines_from_now(applied_unix_nanos);

        let expected_washout_until =
            applied_unix_nanos.saturating_add(Duration::from_secs(20).as_nanos());
        let expected_measure_until =
            expected_washout_until.saturating_add(Duration::from_secs(30).as_nanos());

        assert_eq!(washout_until_unix_nanos, expected_washout_until);
        assert_eq!(measure_until_unix_nanos, expected_measure_until);
    }

    #[test]
    fn interval_event_updates_window_and_emits_noop_decision() {
        let mut runtime = runtime();
        let record = IntervalRecord {
            elapsed_ms: 1_000,
            task: 42,
            samples: 100,
            over_1ms: 3,
            over_2ms: 2,
            over_5ms: 1,
            max_ns: 7_000_000,
            ..IntervalRecord::default()
        };

        let emitted = runtime
            .on_event(MonitorEvent::Interval {
                elapsed_ms: 1_000,
                records: vec![record],
                drop_counters: DropCountersSnapshot::default(),
            })
            .unwrap()
            .unwrap();

        assert_eq!(emitted.decision, "observed");
        assert_eq!(emitted.score_total, 143);
        assert_eq!(runtime.observation().score.total, 143);
        assert_eq!(runtime.observation().scored_task_count, 1);
    }

    #[test]
    fn focus_change_resets_window_and_sets_focus_context() {
        let mut runtime = runtime();

        let emitted = runtime
            .on_event(MonitorEvent::FocusChanged {
                elapsed_ms: 1_000,
                old_kind: None,
                new_kind: FocusGroupKind::Game,
                root_pids: vec![2222],
                member_pids: vec![2222, 2223],
                confidence: 0.90,
                score: 0.95,
                situation: SituationKind::GameFocused,
                reasons: vec!["test focus".to_owned()],
            })
            .unwrap()
            .unwrap();

        assert_eq!(emitted.focus_kind.as_deref(), Some("Game"));
        assert_eq!(emitted.target_root_pid, Some(2222));
        assert_eq!(runtime.observation().focus_kind, Some(FocusGroupKind::Game));
        assert_eq!(
            runtime.observation().primary_situation,
            SituationKind::GameFocused
        );
    }

    #[test]
    fn low_quality_is_reported_in_decision_stream() {
        let mut runtime = runtime();

        let emitted = runtime
            .on_event(MonitorEvent::DataQualityWarning {
                message: "synthetic warning".to_owned(),
            })
            .unwrap()
            .unwrap();

        assert_eq!(emitted.decision, "observed");
        assert!(emitted.data_quality.starts_with("Low"));
        assert_eq!(
            emitted.data_quality_reason_codes,
            vec![
                "insufficient_samples".to_owned(),
                "target_missing".to_owned()
            ]
        );
    }

    #[test]
    fn data_quality_label_names_high_medium_low() {
        assert_eq!(data_quality_label(&OnlineDataQuality::High), "High");
        assert!(
            data_quality_label(&OnlineDataQuality::Medium {
                reasons: vec!["reason".to_owned()]
            })
            .starts_with("Medium")
        );
        assert!(
            data_quality_label(&OnlineDataQuality::Low {
                reasons: vec!["reason".to_owned()]
            })
            .starts_with("Low")
        );
    }
}
