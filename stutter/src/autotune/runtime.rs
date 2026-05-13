use std::{
    collections::{BTreeMap, VecDeque},
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    time::Duration,
};

use serde::Serialize;
use tokio::sync::{mpsc, oneshot};

use crate::{
    actions::{RollbackToken, SafetyClass, TuningAction},
    autotune::{
        apply_low_risk::{action_from_candidate, apply_candidate_with_audit},
        candidate::{
            CandidateAction, CandidateDryRunRecord, dry_run_candidates,
            generate_profile_candidates, generate_topology_aware_profile_candidates,
        },
        candidate_memory::CandidateMemoryResult,
        comparison::{ExperimentDataQuality, ExperimentResult, compare_experiment},
        controller::{
            ActiveExperiment as ControllerActiveExperiment, ControllerPolicy,
            ControllerRuntimeState, decide_autotune_transition,
        },
        controller_journal::{
            default_controller_journal_path, write_controller_journal_applied,
            write_controller_journal_applying, write_controller_journal_clean,
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
        observation::AutotuneObservation,
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
        build_daemon_policy,
    },
    diagnosis::LiveDiagnosisEntry,
    ebpf_loader::DropCountersSnapshot,
    focus::FocusGroupKind,
    process_tree::TaskInfo,
    profiles::Profile,
    scorer::StutterScore,
    session_events::MonitorEvent,
    topology::TopologyModel,
};

pub const DEFAULT_RUNTIME_WINDOW_SECONDS: u64 = 30;
pub const DEFAULT_RECENT_DIAGNOSIS_LIMIT: usize = 16;

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
    pub score_total: u64,
    pub data_quality: String,
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
}

impl RuntimeTargetState {
    fn new(root_pid: Option<u32>) -> Self {
        Self {
            root_pid,
            active_targets: 0,
            target_comm: None,
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
    pending_history_context: Option<RuntimeHistoryContext>,
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
                    manual_restore_command: Some("stutter autotune restore".to_owned()),
                });

        DaemonState {
            schema_version: DAEMON_STATE_SCHEMA_VERSION,
            mode: self.config.mode(),
            phase: daemon_phase_from_controller_phase(self.controller.state.phase),
            cooldown_until_unix_nanos: self.controller.state.cooldown_until_unix_nanos,
            active_target: self.daemon_active_target_snapshot(),
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
                }),
            degraded: self.daemon_degraded_statuses(),
            faulted: self.daemon_fault_state(),
        }
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
            degraded.push(DaemonDegradedStatus {
                category: "data_quality".to_owned(),
                message: data_quality_label(&self.last_observation.data_quality),
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
            manual_restore_command: Some("stutter autotune restore".to_owned()),
        })
    }

    fn update_target_snapshot(&mut self, active_targets: &BTreeMap<u32, TaskInfo>) {
        self.target_state.active_targets = active_targets.len();

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

        AutotuneObservation {
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
            focus_kind,
            focus_confidence,
            focus_roots,
            focus_reasons,
            recent_diagnoses: self.recent_diagnoses.iter().cloned().collect(),
            frame_count: window_score.frame_count,
            frame_p99_ms: window_score.frame_p99_ms,
            frame_max_ms: window_score.frame_max_ms,
            drop_counter_total: self.latest_drop_counters.total(),
        }
    }

    fn target_present(&self, score: &RuntimeWindowScore) -> bool {
        self.target_state.active_targets > 0 || score.scored_samples > 0
    }

    fn select_candidate_for_observation(
        &self,
        observation: &AutotuneObservation,
    ) -> Option<CandidateAction> {
        if self.config.mode() == DaemonMode::Observe {
            return None;
        }

        if self.config.daemon_policy.allow_system_wide_actions {
            return None;
        }

        if observation.data_quality.blocks_action()
            || observation.focus_is_idle_or_unknown()
            || observation.focus_has_critical_realtime_warning()
            || observation.focus_confidence < self.controller.policy.min_focus_confidence
        {
            return None;
        }

        let tree_pid = observation.target_root_pid.or(self.config.tree_pid())?;

        let candidates = self.candidates_for_tree(tree_pid);
        let records = dry_run_candidates(&candidates);

        select_best_candidate_for_situation(
            &candidates,
            &records,
            observation,
            self.controller.policy.max_safety_class.clone(),
            &self.controller.state,
        )
    }

    fn candidates_for_tree(&self, tree_pid: u32) -> Vec<CandidateAction> {
        if !self.config.profiles.is_empty() {
            return generate_profile_candidates(&self.config.profiles, tree_pid, None);
        }

        let Ok(topology) = TopologyModel::read() else {
            return Vec::new();
        };

        generate_topology_aware_profile_candidates(&topology, tree_pid)
    }

    fn apply_decision_side_effects(
        &mut self,
        observation: &AutotuneObservation,
        decision: &AutotuneDecision,
        reason: &str,
    ) -> anyhow::Result<()> {
        match decision {
            AutotuneDecision::StartExperiment { candidate, .. } => {
                self.start_low_risk_experiment(observation, candidate.clone(), reason)?;
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
        let washout_until_unix_nanos =
            applied_unix_nanos.saturating_add(self.config.washout.washout_duration().as_nanos());
        let measure_until_unix_nanos = washout_until_unix_nanos
            .saturating_add(Duration::from_secs(self.config.candidate_window_seconds).as_nanos());

        (washout_until_unix_nanos, measure_until_unix_nanos)
    }

    fn start_low_risk_experiment(
        &mut self,
        observation: &AutotuneObservation,
        candidate: CandidateAction,
        reason: &str,
    ) -> anyhow::Result<()> {
        if self.config.mode() != DaemonMode::ApplyLowRisk {
            return Ok(());
        }

        if candidate.safety_class() != SafetyClass::ReversibleLowRisk {
            anyhow::bail!(
                "live apply-low-risk rejected non-low-risk candidate {} safety={:?}",
                candidate.profile_name(),
                candidate.safety_class()
            );
        }

        let baseline_score = window_score_from_observation(observation);
        validate_window_score_for_apply("baseline", &baseline_score)?;

        let experiment_id = ExperimentId::new(format!(
            "live-low-risk:{}:{}",
            candidate.profile_name(),
            observation.now_unix_nanos
        ));
        let action_id = candidate.action_id().0;
        let journal_path = default_controller_journal_path();

        write_controller_journal_applying(&journal_path, experiment_id.as_str(), &action_id)?;
        let applied = apply_candidate_with_audit(candidate.clone())?;
        write_controller_journal_applied(
            &journal_path,
            experiment_id.as_str(),
            &action_id,
            applied.rollback.clone(),
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
            rollback: applied.rollback,
        });

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
            action_kind: "cpu_affinity_profile".to_owned(),
            score_before: Some(baseline_score.clone()),
            score_after: None,
            rollback_performed: false,
            rollback_policy: "rollback-on-restore".to_owned(),
            cooldown_until_unix_nanos: None,
            manual_restore_command: Some("stutter autotune restore".to_owned()),
        });

        log::info!("autotune_live_low_risk_started reason={reason}");

        Ok(())
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

        let result = compare_experiment(crate::autotune::comparison::ExperimentComparisonInput {
            baseline: &experiment.baseline_score,
            candidate: &candidate_score,
            data_quality: experiment_data_quality(&observation.data_quality),
            target_disappeared: !observation.target_present,
        });

        match result {
            ExperimentResult::Improved { .. } => {
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
                    manual_restore_command: Some("stutter autotune restore".to_owned()),
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

        let (_, action) = action_from_candidate(experiment.candidate.clone())?;
        action.rollback(&experiment.rollback)?;

        write_controller_journal_clean(&default_controller_journal_path())?;

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
            manual_restore_command: Some("stutter autotune restore".to_owned()),
        });

        self.controller
            .state
            .enter_cooldown_after_revert(&self.controller.policy, now_unix_nanos);

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
            score_total: observation.score.total,
            data_quality: data_quality_label(&observation.data_quality),
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

            let rank =
                candidate_situation_rank(candidate.profile_name(), observation.primary_situation)?;
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
    match situation {
        SituationKind::Unknown => HistorySituationKind::Unknown,
        SituationKind::Idle => HistorySituationKind::Idle,
        SituationKind::GameFocused => HistorySituationKind::GameFocused,
        SituationKind::GameCpuSchedulerPressure => HistorySituationKind::GameCpuSchedulerPressure,
        SituationKind::GameGpuBound => HistorySituationKind::GameGpuBound,
        SituationKind::CompositorPressure => HistorySituationKind::CompositorPressure,
        SituationKind::CpuPressure => HistorySituationKind::CpuPressure,
        SituationKind::IoPressure => HistorySituationKind::IoPressure,
        SituationKind::IrqPressure => HistorySituationKind::IrqPressure,
        SituationKind::ThermalOrPowerLimit => HistorySituationKind::ThermalOrPowerLimit,
        SituationKind::CompileLoad
        | SituationKind::BrowserFocused
        | SituationKind::BrowserCpuPressure
        | SituationKind::BrowserGpuVideo
        | SituationKind::BrowserIoPressure
        | SituationKind::CompileCpuBound
        | SituationKind::CompileLinkerPressure
        | SituationKind::MediaPlayback
        | SituationKind::Recording
        | SituationKind::VirtualMachineLoad => HistorySituationKind::CompileLoad,
    }
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
            focus_kind: Some(FocusGroupKind::Game),
            focus_confidence,
            focus_roots: vec![1234],
            focus_reasons: vec!["game focus selected".to_owned()],
            recent_diagnoses: Vec::new(),
            frame_count: 100,
            frame_p99_ms: 12.0,
            frame_max_ms: 20.0,
            drop_counter_total: 0,
        }
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
            score_total: 999,
            data_quality: "Low: low scored samples".to_owned(),
            decision: "faulted".to_owned(),
            reason: "rollback failed".to_owned(),
        });

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
        assert!(
            snapshot
                .degraded
                .iter()
                .any(|status| status.category == "drop_counters")
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
            value["faulted"]["manual_restore_command"],
            "stutter autotune restore"
        );
    }

    #[test]
    fn candidate_selection_blocks_focus_confidence_below_policy_threshold() {
        let runtime = AutotuneRuntime::new(
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
