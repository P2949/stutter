#![cfg(feature = "autotune-controller")]

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
    autotune::{
        candidate::CandidateAction,
        controller::{ControllerPolicy, ControllerRuntimeState, decide_autotune_transition},
        decision::AutotuneDecision,
        history::{
            AutotuneDecisionSummary, AutotuneHistoryEvent, AutotuneMode as HistoryAutotuneMode,
            ControllerPhase as HistoryControllerPhase, ObservationSummary,
            SituationKind as HistorySituationKind, TargetIdentity, append_autotune_history_event,
            default_autotune_history_path,
        },
        observation::AutotuneObservation,
        quality::OnlineDataQuality,
        rolling_window::{RollingWindow, WindowScore as RuntimeWindowScore},
        state::{AutotuneMode, ControllerPhase, SituationKind},
    },
    cli::Config,
    diagnosis::LiveDiagnosisEntry,
    ebpf_loader::DropCountersSnapshot,
    focus::FocusGroupKind,
    process_tree::TaskInfo,
    scorer::StutterScore,
    session_events::MonitorEvent,
};

pub const DEFAULT_RUNTIME_WINDOW_SECONDS: u64 = 30;
pub const DEFAULT_RECENT_DIAGNOSIS_LIMIT: usize = 16;

#[derive(Clone, Debug)]
pub struct AutotuneRuntimeConfig {
    pub mode: AutotuneMode,
    pub controller_id: String,
    pub decision_log: Option<PathBuf>,
    pub history_log: Option<PathBuf>,
    pub window_seconds: u64,
    pub tree_pid: Option<u32>,
    pub watch_process: Option<String>,
    pub allow_system_wide_actions: bool,
}

impl AutotuneRuntimeConfig {
    pub fn observe(
        decision_log: Option<PathBuf>,
        tree_pid: Option<u32>,
        watch_process: Option<String>,
    ) -> Self {
        Self {
            mode: AutotuneMode::Observe,
            controller_id: "local-autotune".to_owned(),
            decision_log,
            history_log: Some(default_autotune_history_path()),
            window_seconds: DEFAULT_RUNTIME_WINDOW_SECONDS,
            tree_pid,
            watch_process,
            allow_system_wide_actions: false,
        }
    }

    pub fn suggest(
        decision_log: Option<PathBuf>,
        tree_pid: Option<u32>,
        watch_process: Option<String>,
    ) -> Self {
        Self {
            mode: AutotuneMode::Suggest,
            controller_id: "local-autotune".to_owned(),
            decision_log,
            history_log: Some(default_autotune_history_path()),
            window_seconds: DEFAULT_RUNTIME_WINDOW_SECONDS,
            tree_pid,
            watch_process,
            allow_system_wide_actions: false,
        }
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
    policy: ControllerPolicy,
    state: ControllerRuntimeState,
    window: RollingWindow,
    latest_focus: Option<RuntimeFocusState>,
    target_state: RuntimeTargetState,
    latest_drop_counters: DropCountersSnapshot,
    recent_diagnoses: VecDeque<LiveDiagnosisEntry>,
    last_observation: AutotuneObservation,
    last_decision: Option<AutotuneDecisionStreamEntry>,
}

impl AutotuneRuntime {
    pub fn new(config: AutotuneRuntimeConfig) -> Self {
        let mode = config.mode;
        Self {
            target_state: RuntimeTargetState::new(config.tree_pid),
            policy: ControllerPolicy::for_mode(mode),
            state: ControllerRuntimeState::default(),
            window: RollingWindow::new(Duration::from_secs(config.window_seconds)),
            latest_focus: None,
            latest_drop_counters: DropCountersSnapshot::default(),
            recent_diagnoses: VecDeque::new(),
            last_observation: AutotuneObservation::default(),
            last_decision: None,
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
                self.window.push_intervals(records);
                return self.evaluate_and_emit(None);
            }
            MonitorEvent::Frame { event } => {
                self.window.push_frame(*event);
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
                self.window.clear();
                self.latest_focus = Some(RuntimeFocusState {
                    kind: new_kind,
                    root_pids: root_pids.clone(),
                    member_pids,
                    confidence,
                    score,
                    situation,
                    reasons,
                });
                self.target_state.root_pid = root_pids.first().copied().or(self.config.tree_pid);
                return self.evaluate_and_emit(Some("focus changed; measurement window reset"));
            }
            MonitorEvent::FocusCleared { reason, .. } => {
                self.window.clear();
                self.latest_focus = None;
                self.target_state.root_pid = self.config.tree_pid;
                return self.evaluate_and_emit(Some(&reason));
            }
            MonitorEvent::DataQualityWarning { message } => {
                return self.evaluate_and_emit(Some(&message));
            }
            MonitorEvent::Finished { reason } => {
                return self.evaluate_and_emit(Some(&reason));
            }
            MonitorEvent::Spike { .. }
            | MonitorEvent::GpuSample { .. }
            | MonitorEvent::IrqEvent { .. }
            | MonitorEvent::IoEvent { .. } => {}
        }

        Ok(None)
    }

    pub fn observation(&self) -> AutotuneObservation {
        self.last_observation.clone()
    }

    pub fn last_decision(&self) -> Option<&AutotuneDecisionStreamEntry> {
        self.last_decision.as_ref()
    }

    fn update_target_snapshot(&mut self, active_targets: &BTreeMap<u32, TaskInfo>) {
        self.target_state.active_targets = active_targets.len();

        if self.target_state.root_pid.is_none() {
            self.target_state.root_pid = self
                .latest_focus
                .as_ref()
                .and_then(|focus| focus.root_pids.first().copied())
                .or(self.config.tree_pid);
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
        self.window.push_diagnosis(diagnosis.clone());
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

        let candidate = self.select_candidate_for_observation(&observation);
        let decision =
            decide_autotune_transition(&self.policy, &self.state, &observation, candidate);
        let reason = forced_reason
            .map(str::to_owned)
            .unwrap_or_else(|| decision_reason(&decision));
        let stream_entry = self.stream_entry_from_decision(&observation, &decision, reason.clone());

        self.append_decision_log(&stream_entry)?;
        self.append_history(&observation, &decision, &reason)?;

        println!("{}", serde_json::to_string(&stream_entry)?);

        self.last_decision = Some(stream_entry.clone());

        Ok(Some(stream_entry))
    }

    fn build_observation(&self) -> AutotuneObservation {
        let window_score = self.window.score();
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
            elapsed_ms: self.window.latest_elapsed_ms().unwrap_or(0),
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
        if self.config.mode != AutotuneMode::Suggest {
            return None;
        }

        if self.config.allow_system_wide_actions {
            return None;
        }

        if observation.data_quality.blocks_action()
            || observation.focus_is_idle_or_unknown()
            || observation.focus_has_critical_realtime_warning()
        {
            return None;
        }

        None
    }

    fn stream_entry_from_decision(
        &self,
        observation: &AutotuneObservation,
        decision: &AutotuneDecision,
        reason: String,
    ) -> AutotuneDecisionStreamEntry {
        AutotuneDecisionStreamEntry {
            unix_nanos: observation.now_unix_nanos,
            phase: format!("{:?}", self.state.phase),
            mode: format!("{:?}", self.config.mode),
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
        &self,
        observation: &AutotuneObservation,
        decision: &AutotuneDecision,
        reason: &str,
    ) -> anyhow::Result<()> {
        let Some(path) = &self.config.history_log else {
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

        let target = observation.target_root_pid.map(|root_pid| TargetIdentity {
            root_pid,
            process_comm: self
                .target_state
                .target_comm
                .clone()
                .or_else(|| self.config.watch_process.clone())
                .unwrap_or_else(|| "unknown".to_owned()),
            process_starttime_ticks: None,
            exe_dev: None,
            exe_ino: None,
            active_task_count: observation.active_target_count,
        });

        let history = AutotuneHistoryEvent::new(
            self.config.controller_id.clone(),
            history_phase(self.state.phase),
            history_mode(self.config.mode),
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
                candidate_name: decision_candidate_name(decision),
                action_kind: decision_action_kind(decision),
                eligible: matches!(decision, AutotuneDecision::Suggest { .. }),
                rollback_policy: "none".to_owned(),
            },
            reason.to_owned(),
        )
        .with_scores(None, Some(score))
        .with_rollback_performed(false);

        append_autotune_history_event(path, &history)?;

        Ok(())
    }
}

#[derive(Debug)]
pub struct AutotuneControllerExit {
    pub reason: String,
    pub last_decision: Option<AutotuneDecisionStreamEntry>,
}

pub async fn run_autotune_controller_session(
    monitor_config: std::sync::Arc<Config>,
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

fn decision_label(decision: &AutotuneDecision) -> String {
    match decision {
        AutotuneDecision::Noop { .. } => "noop".to_owned(),
        AutotuneDecision::Suggest { .. } => "suggest".to_owned(),
        AutotuneDecision::StartExperiment { .. } => {
            "start_experiment_blocked_in_observe_runtime".to_owned()
        }
        AutotuneDecision::KeepCurrent { .. } => {
            "keep_current_blocked_in_observe_runtime".to_owned()
        }
        AutotuneDecision::Revert { .. } => "revert_blocked_in_observe_runtime".to_owned(),
        AutotuneDecision::EnterCooldown { .. } => "cooldown".to_owned(),
        AutotuneDecision::Fault { .. } => "fault".to_owned(),
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

fn history_mode(mode: AutotuneMode) -> HistoryAutotuneMode {
    match mode {
        AutotuneMode::Observe => HistoryAutotuneMode::Observe,
        AutotuneMode::Suggest => HistoryAutotuneMode::Suggest,
        AutotuneMode::ApplyLowRisk => HistoryAutotuneMode::ApplyLowRisk,
        AutotuneMode::ApplyMediumRisk => HistoryAutotuneMode::ApplyMediumRisk,
        AutotuneMode::ApplyHighRisk => HistoryAutotuneMode::ApplyHighRisk,
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
    use crate::{ebpf_loader::DropCountersSnapshot, recorder::IntervalRecord};

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
    fn interval_event_updates_window_and_emits_noop_decision() {
        let mut runtime = runtime();
        let mut record = IntervalRecord::default();
        record.elapsed_ms = 1_000;
        record.task = 42;
        record.samples = 100;
        record.over_1ms = 3;
        record.over_2ms = 2;
        record.over_5ms = 1;
        record.max_ns = 7_000_000;

        let emitted = runtime
            .on_event(MonitorEvent::Interval {
                elapsed_ms: 1_000,
                records: vec![record],
                drop_counters: DropCountersSnapshot::default(),
            })
            .unwrap()
            .unwrap();

        assert_eq!(emitted.decision, "noop");
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

        assert_eq!(emitted.decision, "noop");
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
