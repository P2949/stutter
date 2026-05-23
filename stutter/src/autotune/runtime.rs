#[cfg(test)]
use std::path::PathBuf;
use std::{
    collections::{BTreeMap, VecDeque},
    path::Path,
    time::Duration,
};

#[path = "runtime/config.rs"]
pub(crate) mod config;

#[path = "runtime/daemon_state.rs"]
pub(crate) mod daemon_state;
#[path = "runtime/decision_view.rs"]
pub(crate) mod decision_view;
#[path = "runtime/emission.rs"]
pub(crate) mod emission;

#[path = "runtime/history.rs"]
pub(crate) mod history;

#[path = "runtime/planning.rs"]
pub(crate) mod planning;
#[path = "runtime/session.rs"]
pub(crate) mod session;

#[path = "runtime/stream.rs"]
pub(crate) mod stream;
#[path = "runtime/target_state.rs"]
pub(crate) mod target_state;
#[path = "runtime/worker.rs"]
pub(crate) mod worker;

pub(crate) use config::validate_runtime_config;
pub use config::{AutotuneRuntimeConfig, daemon_config_for_runtime_mode};
pub use daemon_state::daemon_phase_from_controller_phase;
pub(crate) use decision_view::data_quality_label;
#[cfg(test)]
pub(crate) use session::finish_autotune_controller_session;
pub use session::{AutotuneControllerExit, run_autotune_controller_session};
pub(crate) use stream::emit_decision_stream_entry;
pub use stream::{AutotuneDecisionStreamEntry, AutotuneDryRunPlanFileSummary};
pub use target_state::RuntimeTargetState;

#[cfg(test)]
use self::planning::top_denied_reason_for_plan;
use self::{
    decision_view::decision_reason,
    history::RuntimeHistoryContext,
    planning::{
        plan_has_deny_reason, select_best_candidate_for_situation, simulated_dry_run_records,
    },
};
#[cfg(test)]
use crate::autotune::state::SituationKind;
use crate::{
    autotune::{
        active_config::{ActiveConfigMatch, ActiveConfigMatchInput},
        activity::ActivityClassifier,
        controller::{ControllerPolicy, ControllerRuntimeState, decide_autotune_transition},
        decision::AutotuneDecision,
        experiment::ExperimentId,
        external_mutation::{
            ExternalMutationRecoveryDecision, recovery_decision_for_active_experiment,
            recovery_decision_for_kept_action,
        },
        kept::ActiveProfileState,
        live_experiment::{LiveExperimentManager, LiveExperimentManagerInput},
        observation::AutotuneObservation,
        observation_builder::{
            AutotuneObservationBuilder, AutotuneObservationBuilderInput, AutotuneObservationFocus,
        },
        planner::{CandidateDenyReason, CandidatePlanner, PlanResult, PlannerInput},
        planning::{candidate::CandidateAction, plan_io},
        rolling_window::RollingWindow,
        state::ControllerPhase,
    },
    daemon::{
        DaemonPolicy,
        policy::DaemonMode,
        privilege::{
            InProcessPrivilegedActionService, PrivilegedActionService,
            UnixSocketPrivilegedActionService, default_privileged_worker_socket_path,
        },
        state::DaemonState,
    },
    diagnosis::LiveDiagnosisEntry,
    ebpf_loader::DropCountersSnapshot,
    process_tree::TaskInfo,
    session_events::MonitorEvent,
};

pub const DEFAULT_RUNTIME_WINDOW_SECONDS: u64 = 30;
pub const DEFAULT_RECENT_DIAGNOSIS_LIMIT: usize = 16;

const DAEMON_EMERGENCY_RESTORE_COMMAND: &str = "stutter daemon emergency-restore";

struct ResolvedPrivilegedActionService {
    socket_service: Option<UnixSocketPrivilegedActionService>,
    in_process_service: Option<InProcessPrivilegedActionService>,
}

impl ResolvedPrivilegedActionService {
    fn as_service(&self) -> Option<&dyn PrivilegedActionService> {
        self.socket_service
            .as_ref()
            .map(|service| service as &dyn PrivilegedActionService)
            .or_else(|| {
                self.in_process_service
                    .as_ref()
                    .map(|service| service as &dyn PrivilegedActionService)
            })
    }
}

fn resolve_privileged_action_service(
    config: &AutotuneRuntimeConfig,
) -> anyhow::Result<ResolvedPrivilegedActionService> {
    let use_privileged_service =
        config.mode() == DaemonMode::ApplyMediumRisk && !config.simulate_action_effects;
    if !use_privileged_service {
        return Ok(ResolvedPrivilegedActionService {
            socket_service: None,
            in_process_service: None,
        });
    }

    if config
        .daemon_config
        .autotune
        .unsafe_in_process_privileged_worker
    {
        return Ok(ResolvedPrivilegedActionService {
            socket_service: None,
            in_process_service: Some(InProcessPrivilegedActionService::default()),
        });
    }

    let socket_path = config
        .daemon_config
        .autotune
        .privileged_worker_socket
        .clone()
        .map(Ok)
        .unwrap_or_else(default_privileged_worker_socket_path)?;
    Ok(ResolvedPrivilegedActionService {
        socket_service: Some(UnixSocketPrivilegedActionService::new(socket_path)),
        in_process_service: None,
    })
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

#[derive(Debug)]
pub struct AutotuneRuntime {
    config: AutotuneRuntimeConfig,
    controller: OnlineAutotuneController,
    latest_focus: Option<AutotuneObservationFocus>,
    target_state: RuntimeTargetState,
    latest_drop_counters: DropCountersSnapshot,
    recent_diagnoses: VecDeque<LiveDiagnosisEntry>,
    activity_classifier: ActivityClassifier,
    live_experiments: LiveExperimentManager,
    last_observation: AutotuneObservation,
    last_decision: Option<AutotuneDecisionStreamEntry>,
    last_plan_result: Option<PlanResult>,
    last_dry_run_plan_files: Vec<AutotuneDryRunPlanFileSummary>,
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
            activity_classifier: ActivityClassifier::new(5),
            live_experiments: LiveExperimentManager::new(),
            last_observation: AutotuneObservation::default(),
            last_decision: None,
            last_plan_result: None,
            last_dry_run_plan_files: Vec::new(),
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
                let scored_samples = records
                    .iter()
                    .map(|record| record.samples)
                    .fold(0_u64, u64::saturating_add);
                self.activity_classifier.push_interval(scored_samples);
                self.latest_drop_counters = drop_counters;
                self.controller.window.push_intervals(records);
                return self.evaluate_and_emit(None);
            }
            MonitorEvent::Frame { event } => {
                self.controller.window.push_frame(*event);
            }
            MonitorEvent::IrqEvent { event } => {
                self.controller.window.push_irq_event(*event);
            }
            MonitorEvent::IoEvent { event } => {
                self.controller.window.push_block_io_event(*event);
            }
            MonitorEvent::GpuSample { sample } => {
                self.controller.window.push_gpu_sample(*sample);
            }
            MonitorEvent::GpuEngineSample { .. } => {}
            MonitorEvent::CpuFreqSample { event } => {
                self.controller.window.push_cpu_freq_event(*event);
            }
            MonitorEvent::ForegroundEvent { event } => {
                self.controller.window.push_foreground_event(*event);
            }
            MonitorEvent::LiveDiagnosis { entry } => {
                self.push_diagnosis(*entry);
            }
            MonitorEvent::FocusChanged {
                new_kind,
                root_pids,
                member_pids,
                confidence,
                situation,
                reasons,
                ..
            } => {
                self.controller.window.clear();
                if self.live_experiments.has_active_experiment() {
                    self.rollback_live_experiment(
                        crate::audit::unix_nanos_now(),
                        "focus changed during active experiment",
                    )?;
                }
                self.latest_focus = Some(AutotuneObservationFocus {
                    kind: new_kind,
                    root_pids: root_pids.clone(),
                    member_pids,
                    confidence,
                    situation,
                    reasons,
                });
                self.target_state.root_pid = root_pids.first().copied().or(self.config.tree_pid());
                return self.evaluate_and_emit(Some("focus changed; measurement window reset"));
            }
            MonitorEvent::FocusCleared { reason, .. } => {
                self.controller.window.clear();
                if self.live_experiments.has_active_experiment() {
                    self.rollback_live_experiment(
                        crate::audit::unix_nanos_now(),
                        "focus cleared during active experiment",
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
            | MonitorEvent::SchedulerSample { .. }
            | MonitorEvent::Spike { .. }
            | MonitorEvent::ScxEvent { .. }
            | MonitorEvent::KmsFlipEvent { .. }
            | MonitorEvent::DrmFenceEvent { .. }
            | MonitorEvent::WaylandPresentationEvent { .. }
            | MonitorEvent::DmaBufEvent { .. }
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

    pub(crate) fn config(&self) -> &AutotuneRuntimeConfig {
        &self.config
    }

    pub fn controller_state(&self) -> &ControllerRuntimeState {
        &self.controller.state
    }

    pub fn active_profile_state(&self) -> &ActiveProfileState {
        &self.controller.active_profile_state
    }

    pub fn has_active_experiment(&self) -> bool {
        self.live_experiments.has_active_experiment()
    }

    pub fn rollback_on_stop(&mut self, reason: &str) -> anyhow::Result<Option<DaemonState>> {
        if !self.has_active_experiment() {
            return Ok(None);
        }

        self.rollback_live_experiment(crate::audit::unix_nanos_now(), reason)?;

        Ok(Some(self.daemon_state_snapshot()))
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

        let decision =
            if let Some(decision) = self.live_experiments.active_window_decision(&observation) {
                decision
            } else if self.live_experiments.has_active_experiment() {
                self.active_experiment_external_mutation_decision(&observation)
                    .unwrap_or_else(|| {
                        decide_autotune_transition(
                            &self.controller.policy,
                            &self.controller.state,
                            &observation,
                            None,
                        )
                    })
            } else {
                let candidate = self.select_candidate_for_observation(&observation)?;
                self.plan_external_mutation_recovery_decision(&observation)
                    .unwrap_or_else(|| {
                        decide_autotune_transition(
                            &self.controller.policy,
                            &self.controller.state,
                            &observation,
                            candidate,
                        )
                    })
            };

        let reason = forced_reason
            .map(str::to_owned)
            .unwrap_or_else(|| decision_reason(&decision));

        self.apply_decision_side_effects(&observation, &decision, &reason)?;

        let stream_entry = self.stream_entry_from_decision(&observation, &decision, reason.clone());

        self.append_decision_log(&stream_entry)?;
        self.append_history(&observation, &decision, &reason)?;

        emit_decision_stream_entry(&stream_entry)?;

        self.last_decision = Some(stream_entry.clone());

        Ok(Some(stream_entry))
    }

    fn active_experiment_external_mutation_decision(
        &mut self,
        observation: &AutotuneObservation,
    ) -> Option<AutotuneDecision> {
        let snapshot = observation.active_config_snapshot.as_ref()?;

        let (experiment_id, candidate_name, active_match, simulated_fake_candidate) = {
            let experiment = self.live_experiments.current_experiment()?;
            let active_config_input = ActiveConfigMatchInput {
                snapshot,
                active_tasks: &observation.active_tasks,
            };

            (
                experiment.experiment_id.clone(),
                experiment.candidate.candidate_name().to_owned(),
                experiment
                    .candidate
                    .matches_active_config(active_config_input),
                self.config.simulate_action_effects
                    && matches!(&experiment.candidate, CandidateAction::Fake { .. }),
            )
        };

        let decision = recovery_decision_for_active_experiment(
            self.config.daemon_config.autotune.external_mutation_policy,
        );

        let reason = match active_match {
            ActiveConfigMatch::Matches { .. } => {
                return None;
            }
            ActiveConfigMatch::Differs { expected, actual } => {
                format!(
                    "external_mutation_detected: active experiment {candidate_name} no longer matches live state; expected {expected}; actual {actual}; recovery_decision={}",
                    decision.reason_code()
                )
            }
            ActiveConfigMatch::Unknown { summary } => {
                // Fake candidates have no concrete active-config footprint, so their
                // matches_active_config() result is always Unknown. In simulated-action
                // mode this is expected test/dev behavior, not an unverifiable live
                // system state. Real candidates must not take this bypass.
                if simulated_fake_candidate {
                    return None;
                }

                format!(
                    "active_config_unknown: active experiment {candidate_name} live state could not be verified; summary={summary}; recovery_decision={}",
                    decision.reason_code()
                )
            }
        };

        if reason.starts_with("active_config_unknown:") {
            log::warn!("{reason}");
        }

        Some(self.active_experiment_recovery_decision(experiment_id, reason, decision))
    }

    fn active_experiment_recovery_decision(
        &mut self,
        experiment_id: ExperimentId,
        reason: String,
        decision: ExternalMutationRecoveryDecision,
    ) -> AutotuneDecision {
        match decision {
            ExternalMutationRecoveryDecision::RestoreExpectedState => AutotuneDecision::Revert {
                experiment_id,
                reason,
            },
            ExternalMutationRecoveryDecision::AcceptExternalMutationAndResync => {
                let abandoned = self.live_experiments.abandon_current_for_external_resync();
                self.controller.state.active_experiment = None;
                self.controller.state.phase = ControllerPhase::Observing;
                AutotuneDecision::Noop {
                    reason: format!(
                        "{reason}; abandoned_active_experiment={}",
                        abandoned.is_some()
                    ),
                }
            }
            ExternalMutationRecoveryDecision::FaultRequireManualRestore
            | ExternalMutationRecoveryDecision::AbandonKeptAction => {
                AutotuneDecision::Fault { reason }
            }
        }
    }

    fn plan_external_mutation_recovery_decision(
        &mut self,
        _observation: &AutotuneObservation,
    ) -> Option<AutotuneDecision> {
        let plan = self.last_plan_result.as_ref()?;
        let has_kept_drift =
            plan_has_deny_reason(plan, CandidateDenyReason::KeptActionNoLongerActive);
        let has_active_drift =
            plan_has_deny_reason(plan, CandidateDenyReason::ExternalMutationDetected);

        if has_active_drift {
            let decision = recovery_decision_for_active_experiment(
                self.config.daemon_config.autotune.external_mutation_policy,
            );
            return Some(match decision {
                ExternalMutationRecoveryDecision::RestoreExpectedState => {
                    if let Some(experiment) = self.live_experiments.current_experiment() {
                        AutotuneDecision::Revert {
                            experiment_id: experiment.experiment_id.clone(),
                            reason: format!(
                                "external_mutation_detected: active experiment drifted; recovery_decision={}",
                                decision.reason_code()
                            ),
                        }
                    } else {
                        AutotuneDecision::Fault {
                            reason: "external_mutation_detected: active experiment drifted but no current experiment was available for rollback".to_owned(),
                        }
                    }
                }
                ExternalMutationRecoveryDecision::AcceptExternalMutationAndResync => {
                    let abandoned = self.live_experiments.abandon_current_for_external_resync();
                    self.controller.state.active_experiment = None;
                    self.controller.state.phase = ControllerPhase::Observing;
                    AutotuneDecision::Noop {
                        reason: format!(
                            "external_mutation_detected: accepted external active-experiment mutation and resynced controller state; abandoned_active_experiment={}",
                            abandoned.is_some()
                        ),
                    }
                }
                ExternalMutationRecoveryDecision::FaultRequireManualRestore
                | ExternalMutationRecoveryDecision::AbandonKeptAction => AutotuneDecision::Fault {
                    reason: format!(
                        "external_mutation_detected: active experiment drifted; recovery_decision={}",
                        decision.reason_code()
                    ),
                },
            });
        }

        if has_kept_drift {
            let decision = recovery_decision_for_kept_action(
                self.config.daemon_config.autotune.external_mutation_policy,
            );
            return Some(match decision {
                ExternalMutationRecoveryDecision::AbandonKeptAction => {
                    let abandoned = self
                        .controller
                        .active_profile_state
                        .abandon_kept_actions_for_external_resync();
                    AutotuneDecision::Noop {
                        reason: format!(
                            "kept_action_no_longer_active: accepted external mutation and abandoned kept action state; recovery_decision={} abandoned_kept_actions={abandoned}",
                            decision.reason_code()
                        ),
                    }
                }
                ExternalMutationRecoveryDecision::FaultRequireManualRestore
                | ExternalMutationRecoveryDecision::RestoreExpectedState
                | ExternalMutationRecoveryDecision::AcceptExternalMutationAndResync => {
                    AutotuneDecision::Fault {
                        reason: format!(
                            "kept_action_no_longer_active: kept action drifted; recovery_decision={}; run stutter daemon resync-state or restore manually",
                            decision.reason_code()
                        ),
                    }
                }
            });
        }

        None
    }

    fn build_observation(&self) -> AutotuneObservation {
        AutotuneObservationBuilder::build(AutotuneObservationBuilderInput {
            window: &self.controller.window,
            online_data_quality_policy: &self.config.online_data_quality_policy,
            focus: self.latest_focus.as_ref(),
            root_pid: self.target_state.root_pid,
            active_target_count: self.target_state.active_targets,
            active_tasks: &self.target_state.active_tasks,
            recent_diagnoses: self.recent_diagnoses.iter().cloned().collect(),
            drop_counters: self.latest_drop_counters.clone(),
            proc_root: Path::new("/proc"),
            sys_root: Path::new("/sys"),
            activity_level: self.activity_classifier.classify(),
        })
        .observation
    }

    fn select_candidate_for_observation(
        &mut self,
        observation: &AutotuneObservation,
    ) -> anyhow::Result<Option<CandidateAction>> {
        self.last_dry_run_plan_files.clear();

        if self.config.mode() == DaemonMode::Observe {
            self.last_plan_result = Some(PlanResult {
                selected: None,
                evaluations: Vec::new(),
                no_action_reason: Some("observe mode does not suggest or apply".to_owned()),
            });
            return Ok(None);
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
            return Ok(None);
        }

        let Some(tree_pid) = observation.target_root_pid.or(self.config.tree_pid()) else {
            return Ok(None);
        };

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
            return Ok(selected);
        }

        let mut observation = observation.clone();
        if observation.target_root_pid.is_none() {
            observation.target_root_pid = Some(tree_pid);
        }
        let planner = CandidatePlanner::default_for_policy(&self.config.daemon_policy);
        if let Some(err) = self.config.workload_policy_error.as_ref() {
            self.last_plan_result = Some(PlanResult {
                selected: None,
                evaluations: Vec::new(),
                no_action_reason: Some(format!("invalid workload policy configuration: {err}")),
            });
            return Ok(None);
        }
        let result = planner.plan(PlannerInput {
            observation: &observation,
            daemon_policy: &self.config.daemon_policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            controller_state: &self.controller.state,
            active_profile_state: Some(&self.controller.active_profile_state),
            workload_policy: &self.config.workload_policy,
            profiles: &self.config.profiles,
        });
        let selected = result.selected.clone();
        if self.config.dry_run_all_safe {
            self.last_dry_run_plan_files = self.write_dry_run_plan_files_for_plan(&result)?;
        }
        self.last_plan_result = Some(result);
        Ok(selected)
    }

    fn write_dry_run_plan_files_for_plan(
        &self,
        plan: &PlanResult,
    ) -> anyhow::Result<Vec<AutotuneDryRunPlanFileSummary>> {
        let plan_dir = self
            .config
            .dry_run_plan_dir
            .clone()
            .unwrap_or_else(plan_io::default_candidate_plan_dir);
        let mut written = Vec::new();

        for evaluation in &plan.evaluations {
            let Some(dry_run) = evaluation.dry_run.as_ref() else {
                continue;
            };
            let path = plan_io::candidate_plan_path(&evaluation.candidate, &plan_dir);
            plan_io::write_candidate_plan_file(
                &path,
                &evaluation.candidate,
                Some(dry_run.affected_tasks),
            )?;
            written.push(AutotuneDryRunPlanFileSummary {
                candidate_name: evaluation.candidate_name.clone(),
                action_kind: evaluation.action_kind.clone(),
                path,
                affected_tasks: dry_run.affected_tasks,
                safety_class: evaluation.descriptor.safety_class.clone(),
                eligible: evaluation.eligible,
                deny_reasons: evaluation.deny_reasons.clone(),
            });
        }

        Ok(written)
    }

    fn apply_decision_side_effects(
        &mut self,
        observation: &AutotuneObservation,
        decision: &AutotuneDecision,
        reason: &str,
    ) -> anyhow::Result<()> {
        if self.config.dry_run_all_safe
            && matches!(decision, AutotuneDecision::StartExperiment { .. })
        {
            anyhow::bail!("dry-run-all-safe mode refused to start a live experiment");
        }

        let privileged_action_service = resolve_privileged_action_service(&self.config)?;
        let input = LiveExperimentManagerInput {
            mode: self.config.mode(),
            daemon_policy: self.config.daemon_policy.clone(),
            controller_policy: self.controller.policy.clone(),
            simulate_action_effects: self.config.simulate_action_effects,
            washout: self.config.washout.clone(),
            candidate_window_seconds: self.config.candidate_window_seconds,
            manual_restore_command: DAEMON_EMERGENCY_RESTORE_COMMAND,
            controller_journal_path: self.config.controller_journal_path.clone(),
            #[cfg(test)]
            exit_rollback_registry: None,
            privileged_action_service: privileged_action_service.as_service(),
        };

        let outcome = self.live_experiments.apply_decision_side_effects(
            input,
            &mut self.controller.state,
            &mut self.controller.active_profile_state,
            observation,
            decision,
            reason,
        )?;

        log::debug!(
            "autotune_live_experiment_outcome event={} clear_measurement_window={} history_context={}",
            outcome.event.as_str(),
            outcome.clear_measurement_window,
            outcome.history_context.is_some()
        );

        if outcome.clear_measurement_window {
            self.controller.window.clear();
        }

        if let Some(context) = outcome.history_context {
            self.pending_history_context = Some(context.into());
        }

        Ok(())
    }

    fn rollback_live_experiment(
        &mut self,
        now_unix_nanos: u128,
        reason: &str,
    ) -> anyhow::Result<()> {
        let Some(experiment_id) = self.live_experiments.current_experiment_id() else {
            log::warn!("rollback_live_experiment requested without active experiment: {reason}");
            return Ok(());
        };
        let mut observation = self.last_observation.clone();
        observation.now_unix_nanos = now_unix_nanos;
        let decision = AutotuneDecision::Revert {
            experiment_id,
            reason: reason.to_owned(),
        };
        self.apply_decision_side_effects(&observation, &decision, reason)
    }
}

#[cfg(test)]
mod tests;
