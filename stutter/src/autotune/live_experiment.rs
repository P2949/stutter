use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::Context;

use crate::{
    actions::{RollbackToken, SafetyClass},
    autotune::{
        apply_low_risk::apply_candidate_with_audit,
        candidate::CandidateAction,
        candidate_memory::CandidateMemoryResult,
        comparison::{ExperimentDataQuality, ExperimentResult},
        controller::{
            ActiveExperiment as ControllerActiveExperiment, ControllerPolicy,
            ControllerRuntimeState,
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
        objective::{ObjectiveComparisonInput, ObjectiveSignals, compare_for_objective},
        observation::AutotuneObservation,
        state::ControllerPhase,
        washout::WashoutWindowConfig,
    },
    daemon::{
        DaemonExperimentState, DaemonMode, DaemonPolicy, DaemonPolicyContext, DaemonRollbackState,
        privilege::{
            CandidateApplyRequest, CandidatePlanRequest, PrivilegedActionService, RollbackRequest,
        },
    },
};

#[derive(Clone, Debug)]
pub struct LiveExperiment {
    pub experiment_id: ExperimentId,
    pub candidate: CandidateAction,
    pub safety_class: SafetyClass,
    pub mode: DaemonMode,
    pub baseline_score: WindowScore,
    pub baseline_signals: ObjectiveSignals,
    pub applied_unix_nanos: u128,
    pub washout_until_unix_nanos: u128,
    pub measure_until_unix_nanos: u128,
    pub rollback: RollbackToken,
}

impl LiveExperiment {
    pub fn candidate_name(&self) -> &str {
        self.candidate.profile_name()
    }

    pub fn action_id(&self) -> String {
        self.candidate.action_id().0
    }
}

#[derive(Clone, Debug)]
pub struct LiveExperimentManagerInput<'a> {
    pub mode: DaemonMode,
    pub daemon_policy: DaemonPolicy,
    pub controller_policy: ControllerPolicy,
    pub simulate_action_effects: bool,
    pub washout: WashoutWindowConfig,
    pub candidate_window_seconds: u64,
    pub manual_restore_command: &'static str,
    pub controller_journal_path: Option<PathBuf>,
    pub exit_rollback_registry: Option<&'a crate::autotune::shutdown::ActiveAutotuneActionRegistry>,
    pub privileged_action_service: Option<&'a dyn PrivilegedActionService>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveExperimentEvent {
    Noop,
    Started,
    Kept,
    Reverted,
    CooldownEntered,
    Faulted,
}

#[derive(Clone, Debug)]
pub struct LiveExperimentHistoryContext {
    pub experiment_id: String,
    pub action_id: String,
    pub candidate_name: String,
    pub action_kind: String,
    pub mode: DaemonMode,
    pub safety_class: SafetyClass,
    pub score_before: Option<WindowScore>,
    pub score_after: Option<WindowScore>,
    pub rollback_performed: bool,
    pub rollback_policy: String,
    pub cooldown_until_unix_nanos: Option<u128>,
    pub manual_restore_command: Option<String>,
}

#[derive(Clone, Debug)]
pub struct LiveExperimentOutcome {
    pub event: LiveExperimentEvent,
    pub history_context: Option<LiveExperimentHistoryContext>,
    pub clear_measurement_window: bool,
}

impl LiveExperimentOutcome {
    fn noop() -> Self {
        Self {
            event: LiveExperimentEvent::Noop,
            history_context: None,
            clear_measurement_window: false,
        }
    }

    fn event(event: LiveExperimentEvent) -> Self {
        Self {
            event,
            history_context: None,
            clear_measurement_window: false,
        }
    }

    fn with_history(
        event: LiveExperimentEvent,
        history_context: LiveExperimentHistoryContext,
    ) -> Self {
        Self {
            event,
            history_context: Some(history_context),
            clear_measurement_window: false,
        }
    }

    fn with_clear_measurement_window(mut self) -> Self {
        self.clear_measurement_window = true;
        self
    }
}

#[derive(Debug, Default)]
pub struct LiveExperimentManager {
    current: Option<LiveExperiment>,
}

trait LiveExperimentActionExecutor {
    fn apply_candidate(
        &mut self,
        input: &LiveExperimentManagerInput<'_>,
        candidate: &CandidateAction,
        experiment_id: &str,
        observation: &AutotuneObservation,
    ) -> anyhow::Result<RollbackToken>;

    fn rollback_candidate(
        &mut self,
        input: &LiveExperimentManagerInput<'_>,
        experiment: &LiveExperiment,
        observation: &AutotuneObservation,
    ) -> anyhow::Result<()>;
}

struct RuntimeLiveExperimentActionExecutor;

impl LiveExperimentActionExecutor for RuntimeLiveExperimentActionExecutor {
    fn apply_candidate(
        &mut self,
        input: &LiveExperimentManagerInput<'_>,
        candidate: &CandidateAction,
        _experiment_id: &str,
        observation: &AutotuneObservation,
    ) -> anyhow::Result<RollbackToken> {
        if input.mode == DaemonMode::ApplyMediumRisk
            && candidate.safety_class() > SafetyClass::ReversibleLowRisk
        {
            let service = input
                .privileged_action_service
                .ok_or_else(|| anyhow::anyhow!("privileged_worker_required: apply-medium-risk requires a privileged action service"))?;
            let result = service
                .apply_candidate(CandidateApplyRequest {
                    plan: CandidatePlanRequest::from_candidate(
                        candidate.clone(),
                        observation.now_unix_nanos,
                    ),
                    policy: input.daemon_policy.clone(),
                    context: policy_context_for_runtime_apply(observation),
                    max_plan_age_nanos: Duration::from_secs(30).as_nanos(),
                })
                .with_context(|| {
                    format!(
                        "privileged apply failed for autotune candidate '{}'",
                        candidate.candidate_name()
                    )
                })?;
            Ok(result.rollback)
        } else {
            Ok(apply_candidate_with_audit(candidate.clone())?.rollback)
        }
    }

    fn rollback_candidate(
        &mut self,
        input: &LiveExperimentManagerInput<'_>,
        experiment: &LiveExperiment,
        observation: &AutotuneObservation,
    ) -> anyhow::Result<()> {
        if experiment.mode == DaemonMode::ApplyMediumRisk
            && experiment.safety_class > SafetyClass::ReversibleLowRisk
        {
            let service = input
                .privileged_action_service
                .ok_or_else(|| anyhow::anyhow!("privileged_worker_required: apply-medium-risk rollback requires a privileged action service"))?;
            service.rollback(RollbackRequest {
                candidate: experiment.candidate.clone(),
                token: experiment.rollback.clone(),
                policy: input.daemon_policy.clone(),
                context: policy_context_for_runtime_apply(observation),
            })?;
        } else {
            let executor =
                crate::autotune::apply::executor_for_candidate(experiment.candidate.clone())?;
            executor.rollback(&experiment.rollback)?;
        }

        Ok(())
    }
}

struct LiveExperimentRuntimeState<'a> {
    controller_state: &'a mut ControllerRuntimeState,
    active_profile_state: &'a mut ActiveProfileState,
}

impl LiveExperimentManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn has_active_experiment(&self) -> bool {
        self.current.is_some()
    }

    pub fn current_experiment(&self) -> Option<&LiveExperiment> {
        self.current.as_ref()
    }

    pub fn current_experiment_id(&self) -> Option<ExperimentId> {
        self.current
            .as_ref()
            .map(|experiment| experiment.experiment_id.clone())
    }

    #[cfg(test)]
    pub fn set_current_for_tests(&mut self, experiment: LiveExperiment) {
        self.current = Some(experiment);
    }

    pub fn daemon_experiment_state(&self) -> Option<DaemonExperimentState> {
        self.current
            .as_ref()
            .map(|experiment| DaemonExperimentState {
                experiment_id: experiment.experiment_id.as_str().to_owned(),
                action_id: experiment.action_id(),
                candidate_name: Some(experiment.candidate_name().to_owned()),
                mode: experiment.mode,
                safety_class: experiment.safety_class.clone(),
                started_unix_nanos: Some(experiment.applied_unix_nanos),
            })
    }

    pub fn daemon_rollback_state(
        &self,
        manual_restore_command: &'static str,
    ) -> Option<DaemonRollbackState> {
        self.current.as_ref().map(|experiment| DaemonRollbackState {
            action_id: experiment.action_id(),
            mode: experiment.mode,
            safety_class: experiment.safety_class.clone(),
            rollback_available: true,
            token: Some(experiment.rollback.clone()),
            manual_restore_command: Some(manual_restore_command.to_owned()),
        })
    }

    pub fn active_window_decision(
        &self,
        observation: &AutotuneObservation,
    ) -> Option<AutotuneDecision> {
        let experiment = self.current.as_ref()?;

        if observation.now_unix_nanos < experiment.washout_until_unix_nanos {
            return Some(AutotuneDecision::Noop {
                reason: format!(
                    "candidate '{}' washout window is still stabilizing",
                    experiment.candidate_name()
                ),
            });
        }

        if observation.now_unix_nanos < experiment.measure_until_unix_nanos {
            return Some(AutotuneDecision::Noop {
                reason: format!(
                    "candidate '{}' measurement window is still collecting",
                    experiment.candidate_name()
                ),
            });
        }

        None
    }

    pub fn validate_start_candidate(
        mode: DaemonMode,
        policy: &DaemonPolicy,
        candidate: &CandidateAction,
    ) -> anyhow::Result<bool> {
        match mode {
            DaemonMode::ApplyLowRisk => {
                if candidate.safety_class() != SafetyClass::ReversibleLowRisk {
                    anyhow::bail!(
                        "live apply-low-risk rejected non-low-risk candidate {} safety={:?}",
                        candidate.profile_name(),
                        candidate.safety_class()
                    );
                }
                Ok(true)
            }
            DaemonMode::ApplyMediumRisk => {
                if !policy.allow_medium_risk_apply {
                    anyhow::bail!("live apply-medium-risk requires explicit medium-risk unlock");
                }
                if candidate.safety_class() > SafetyClass::ReversibleMediumRisk {
                    anyhow::bail!(
                        "live apply-medium-risk rejected non-medium-risk candidate {} safety={:?}",
                        candidate.profile_name(),
                        candidate.safety_class()
                    );
                }
                if candidate.is_high_risk_system_adjacent() {
                    anyhow::bail!(
                        "live apply-medium-risk rejected manual-only high-risk candidate {} action_kind={}",
                        candidate.profile_name(),
                        candidate.action_kind()
                    );
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    pub fn deadlines_from_now(
        simulate_action_effects: bool,
        washout: &WashoutWindowConfig,
        candidate_window_seconds: u64,
        applied_unix_nanos: u128,
    ) -> (u128, u128) {
        if simulate_action_effects {
            return (applied_unix_nanos, applied_unix_nanos);
        }

        let washout_until_unix_nanos =
            applied_unix_nanos.saturating_add(washout.washout_duration().as_nanos());
        let measure_until_unix_nanos = washout_until_unix_nanos
            .saturating_add(Duration::from_secs(candidate_window_seconds).as_nanos());

        (washout_until_unix_nanos, measure_until_unix_nanos)
    }

    pub fn compare_keep_result(
        experiment: &LiveExperiment,
        candidate_score: &WindowScore,
        observation: &AutotuneObservation,
    ) -> ExperimentResult {
        compare_for_objective(ObjectiveComparisonInput {
            objective: experiment.candidate.objective(),
            baseline: &experiment.baseline_score,
            candidate: candidate_score,
            baseline_signals: &experiment.baseline_signals,
            candidate_signals: &observation.objective_signals,
            data_quality: experiment_data_quality(&observation.data_quality),
            target_disappeared: !observation.target_present,
        })
    }

    pub fn apply_decision_side_effects(
        &mut self,
        input: LiveExperimentManagerInput<'_>,
        controller_state: &mut ControllerRuntimeState,
        active_profile_state: &mut ActiveProfileState,
        observation: &AutotuneObservation,
        decision: &AutotuneDecision,
        reason: &str,
    ) -> anyhow::Result<LiveExperimentOutcome> {
        let mut executor = RuntimeLiveExperimentActionExecutor;
        self.apply_decision_side_effects_with_executor(
            input,
            LiveExperimentRuntimeState {
                controller_state,
                active_profile_state,
            },
            observation,
            decision,
            reason,
            &mut executor,
        )
    }

    fn apply_decision_side_effects_with_executor<E: LiveExperimentActionExecutor + ?Sized>(
        &mut self,
        input: LiveExperimentManagerInput<'_>,
        runtime_state: LiveExperimentRuntimeState<'_>,
        observation: &AutotuneObservation,
        decision: &AutotuneDecision,
        reason: &str,
        executor: &mut E,
    ) -> anyhow::Result<LiveExperimentOutcome> {
        match decision {
            AutotuneDecision::StartExperiment { candidate, .. } => self
                .start_candidate_experiment_with_executor(
                    &input,
                    &mut *runtime_state.controller_state,
                    observation,
                    candidate.clone(),
                    reason,
                    executor,
                ),
            AutotuneDecision::KeepCurrent { .. } => self.keep_current_experiment_with_executor(
                &input,
                &mut *runtime_state.controller_state,
                &mut *runtime_state.active_profile_state,
                observation,
                reason,
                executor,
            ),
            AutotuneDecision::Revert { .. } => self.rollback_active_experiment_with_executor(
                &input,
                &mut *runtime_state.controller_state,
                observation,
                observation.now_unix_nanos,
                reason,
                executor,
            ),
            AutotuneDecision::EnterCooldown { duration, .. } => {
                runtime_state.controller_state.phase = ControllerPhase::Cooldown;
                runtime_state.controller_state.cooldown_until_unix_nanos = Some(
                    observation
                        .now_unix_nanos
                        .saturating_add(duration.as_nanos()),
                );
                Ok(LiveExperimentOutcome::event(
                    LiveExperimentEvent::CooldownEntered,
                ))
            }
            AutotuneDecision::Fault { .. } => {
                let rollback_outcome = if self.has_active_experiment() {
                    Some(self.rollback_active_experiment_with_executor(
                        &input,
                        &mut *runtime_state.controller_state,
                        observation,
                        observation.now_unix_nanos,
                        reason,
                        executor,
                    )?)
                } else {
                    None
                };

                runtime_state.controller_state.enter_cooldown_after_fault(
                    &input.controller_policy,
                    observation.now_unix_nanos,
                );

                let mut outcome = LiveExperimentOutcome::event(LiveExperimentEvent::Faulted);
                if let Some(rollback_outcome) = rollback_outcome {
                    outcome.history_context = rollback_outcome.history_context;
                }
                Ok(outcome)
            }
            AutotuneDecision::Noop { .. } | AutotuneDecision::Suggest { .. } => {
                Ok(LiveExperimentOutcome::noop())
            }
        }
    }

    fn start_candidate_experiment_with_executor<E: LiveExperimentActionExecutor + ?Sized>(
        &mut self,
        input: &LiveExperimentManagerInput<'_>,
        controller_state: &mut ControllerRuntimeState,
        observation: &AutotuneObservation,
        candidate: CandidateAction,
        reason: &str,
        executor: &mut E,
    ) -> anyhow::Result<LiveExperimentOutcome> {
        if !Self::validate_start_candidate(input.mode, &input.daemon_policy, &candidate)? {
            return Ok(LiveExperimentOutcome::noop());
        }

        let baseline_score = window_score_from_observation(observation);
        validate_window_score_for_apply("baseline", &baseline_score)?;

        let experiment_id = ExperimentId::new(format!(
            "live-{}:{}:{}",
            input.mode.as_str(),
            candidate.profile_name(),
            observation.now_unix_nanos
        ));
        let action_id = candidate.action_id().0;
        let action_kind = candidate.action_kind().to_owned();
        let safety_class = candidate.safety_class();

        let rollback = self.apply_candidate_for_runtime_with_executor(
            input,
            executor,
            &candidate,
            experiment_id.as_str(),
            &action_id,
            observation,
        )?;

        controller_state.record_candidate_attempt(
            &candidate,
            observation,
            None,
            Some(
                observation.now_unix_nanos.saturating_add(
                    input
                        .controller_policy
                        .minimum_time_between_same_action
                        .as_nanos(),
                ),
            ),
        );

        controller_state.phase = ControllerPhase::Measuring;
        controller_state.active_experiment = Some(ControllerActiveExperiment {
            experiment_id: experiment_id.clone(),
            candidate: candidate.clone(),
            baseline_score_total: baseline_score.score.total,
        });

        let (washout_until_unix_nanos, measure_until_unix_nanos) = Self::deadlines_from_now(
            input.simulate_action_effects,
            &input.washout,
            input.candidate_window_seconds,
            observation.now_unix_nanos,
        );

        self.current = Some(LiveExperiment {
            experiment_id,
            candidate,
            safety_class: safety_class.clone(),
            mode: input.mode,
            baseline_score: baseline_score.clone(),
            baseline_signals: observation.objective_signals.clone(),
            applied_unix_nanos: observation.now_unix_nanos,
            washout_until_unix_nanos,
            measure_until_unix_nanos,
            rollback,
        });

        if let Some(experiment) = self.current.as_ref() {
            self.write_controller_journal_phase_for_live_experiment(
                input,
                experiment,
                observation,
                ControllerJournalState::Measuring,
                "measurement_window_collecting",
            )?;
        }

        let history_context = LiveExperimentHistoryContext {
            experiment_id: self
                .current
                .as_ref()
                .map(|experiment| experiment.experiment_id.as_str().to_owned())
                .unwrap_or_else(|| "unknown-experiment".to_owned()),
            action_id,
            candidate_name: self
                .current
                .as_ref()
                .map(|experiment| experiment.candidate.profile_name().to_owned())
                .unwrap_or_else(|| "unknown-candidate".to_owned()),
            action_kind,
            mode: input.mode,
            safety_class,
            score_before: Some(baseline_score),
            score_after: None,
            rollback_performed: false,
            rollback_policy: "rollback-on-restore".to_owned(),
            cooldown_until_unix_nanos: None,
            manual_restore_command: Some(input.manual_restore_command.to_owned()),
        };

        log::info!(
            "autotune_live_experiment_started mode={} action_kind={} reason={reason}",
            input.mode,
            history_context.action_id
        );

        Ok(
            LiveExperimentOutcome::with_history(LiveExperimentEvent::Started, history_context)
                .with_clear_measurement_window(),
        )
    }

    fn apply_candidate_for_runtime_with_executor<E: LiveExperimentActionExecutor + ?Sized>(
        &self,
        input: &LiveExperimentManagerInput<'_>,
        executor: &mut E,
        candidate: &CandidateAction,
        experiment_id: &str,
        action_id: &str,
        observation: &AutotuneObservation,
    ) -> anyhow::Result<RollbackToken> {
        if input.simulate_action_effects && matches!(candidate, CandidateAction::Fake { .. }) {
            return Ok(RollbackToken::NiceRestore {
                records: Vec::new(),
            });
        }

        if input.simulate_action_effects {
            return Ok(RollbackToken::CpuAffinityRestoreFile {
                path: PathBuf::from(format!(
                    "/tmp/stutter-simulated-rollback-{experiment_id}.json"
                )),
                affected_tasks: observation.active_target_count.max(1),
            });
        }

        let journal_path = controller_journal_path(input);

        write_controller_journal_applying_with_metadata(
            &journal_path,
            experiment_id,
            action_id,
            self.controller_journal_metadata_for_candidate(
                input,
                candidate,
                observation,
                None,
                "pending_apply",
            ),
        )?;

        let applied_rollback =
            executor.apply_candidate(input, candidate, experiment_id, observation)?;

        if input.mode == DaemonMode::ApplyLowRisk
            && let Some(registry) = input.exit_rollback_registry
        {
            crate::autotune::shutdown::register_cpu_affinity_rollback(
                registry,
                action_id.to_owned(),
                applied_rollback.clone(),
            );
        }

        write_controller_journal_applied_with_metadata(
            &journal_path,
            experiment_id,
            action_id,
            applied_rollback.clone(),
            self.controller_journal_metadata_for_candidate(
                input,
                candidate,
                observation,
                Some(applied_rollback.affected_tasks()),
                "applied_pending_verify",
            ),
        )?;

        Ok(applied_rollback)
    }

    pub fn controller_journal_record_for_live_experiment(
        &self,
        input: &LiveExperimentManagerInput<'_>,
        experiment: &LiveExperiment,
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
            input,
            &experiment.candidate,
            observation,
            Some(experiment.rollback.affected_tasks()),
            verify_result,
        ))
        .with_mode(experiment.mode)
        .with_safety_class(experiment.safety_class.clone())
    }

    fn write_controller_journal_phase_for_live_experiment(
        &self,
        input: &LiveExperimentManagerInput<'_>,
        experiment: &LiveExperiment,
        observation: &AutotuneObservation,
        state: ControllerJournalState,
        verify_result: &'static str,
    ) -> anyhow::Result<()> {
        if input.simulate_action_effects && input.controller_journal_path.is_none() {
            return Ok(());
        }

        let record = self.controller_journal_record_for_live_experiment(
            input,
            experiment,
            observation,
            state,
            verify_result,
        );
        write_controller_journal_record(&controller_journal_path(input), &record)
    }

    fn controller_journal_metadata_for_candidate(
        &self,
        input: &LiveExperimentManagerInput<'_>,
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
            .with_restore_command(input.manual_restore_command)
            .with_verify_result(verify_result)
            .with_mode(input.mode)
            .with_safety_class(candidate.safety_class())
    }

    fn keep_current_experiment_with_executor<E: LiveExperimentActionExecutor + ?Sized>(
        &mut self,
        input: &LiveExperimentManagerInput<'_>,
        controller_state: &mut ControllerRuntimeState,
        active_profile_state: &mut ActiveProfileState,
        observation: &AutotuneObservation,
        reason: &str,
        executor: &mut E,
    ) -> anyhow::Result<LiveExperimentOutcome> {
        let Some(experiment) = self.current.take() else {
            return Ok(LiveExperimentOutcome::noop());
        };

        let candidate_score = window_score_from_observation(observation);
        validate_window_score_for_apply("candidate", &candidate_score)?;
        let result = Self::compare_keep_result(&experiment, &candidate_score, observation);

        match result {
            ExperimentResult::Improved { .. } => {
                if let Err(err) = self.write_controller_journal_phase_for_live_experiment(
                    input,
                    &experiment,
                    observation,
                    ControllerJournalState::Keeping,
                    "kept_pending_manual_restore",
                ) {
                    self.current = Some(experiment);
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

                if let Err(err) = active_profile_state.record_kept_candidate(kept, result.clone()) {
                    self.current = Some(experiment);
                    return Err(err);
                }

                controller_state.record_candidate_result(
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
                            .saturating_add(input.controller_policy.cooldown_after_keep.as_nanos()),
                    ),
                );

                let cooldown_until_unix_nanos = observation
                    .now_unix_nanos
                    .saturating_add(input.controller_policy.cooldown_after_keep.as_nanos());

                let history_context = LiveExperimentHistoryContext {
                    experiment_id: experiment.experiment_id.as_str().to_owned(),
                    action_id: experiment.action_id(),
                    candidate_name: experiment.candidate.profile_name().to_owned(),
                    action_kind: experiment.candidate.action_kind().to_owned(),
                    mode: experiment.mode,
                    safety_class: experiment.safety_class.clone(),
                    score_before: Some(experiment.baseline_score.clone()),
                    score_after: Some(candidate_score),
                    rollback_performed: false,
                    rollback_policy: "rollback-on-restore".to_owned(),
                    cooldown_until_unix_nanos: Some(cooldown_until_unix_nanos),
                    manual_restore_command: Some(input.manual_restore_command.to_owned()),
                };

                controller_state.enter_cooldown_after_keep(
                    &input.controller_policy,
                    observation.now_unix_nanos,
                );
                controller_state.active_experiment = None;

                Ok(LiveExperimentOutcome::with_history(
                    LiveExperimentEvent::Kept,
                    history_context,
                ))
            }
            other => {
                self.current = Some(experiment);
                self.rollback_active_experiment_with_executor(
                    input,
                    controller_state,
                    observation,
                    observation.now_unix_nanos,
                    &format!("candidate was not improved at keep point: {other:?}"),
                    executor,
                )
            }
        }
    }

    fn rollback_active_experiment_with_executor<E: LiveExperimentActionExecutor + ?Sized>(
        &mut self,
        input: &LiveExperimentManagerInput<'_>,
        controller_state: &mut ControllerRuntimeState,
        observation: &AutotuneObservation,
        now_unix_nanos: u128,
        reason: &str,
        executor: &mut E,
    ) -> anyhow::Result<LiveExperimentOutcome> {
        let Some(experiment) = self.current.take() else {
            return Ok(LiveExperimentOutcome::noop());
        };

        if let Err(err) = self.write_controller_journal_phase_for_live_experiment(
            input,
            &experiment,
            observation,
            ControllerJournalState::Reverting,
            "rollback_in_progress",
        ) {
            self.current = Some(experiment);
            return Err(err);
        }

        if !input.simulate_action_effects
            && let Err(err) = executor.rollback_candidate(input, &experiment, observation)
        {
            self.current = Some(experiment);
            return Err(err);
        }

        self.write_controller_journal_phase_for_live_experiment(
            input,
            &experiment,
            observation,
            ControllerJournalState::Reverted,
            "rollback_verified",
        )?;

        controller_state.record_candidate_result(
            &experiment.candidate,
            observation,
            None,
            CandidateMemoryResult::Reverted,
            Some(experiment.baseline_score.score.total),
            Some(observation.score.total),
            Some(reason.to_owned()),
            Some(
                now_unix_nanos
                    .saturating_add(input.controller_policy.cooldown_after_revert.as_nanos()),
            ),
        );

        let cooldown_until_unix_nanos =
            now_unix_nanos.saturating_add(input.controller_policy.cooldown_after_revert.as_nanos());

        let history_context = LiveExperimentHistoryContext {
            experiment_id: experiment.experiment_id.as_str().to_owned(),
            action_id: experiment.action_id(),
            candidate_name: experiment.candidate.profile_name().to_owned(),
            action_kind: experiment.candidate.action_kind().to_owned(),
            mode: experiment.mode,
            safety_class: experiment.safety_class.clone(),
            score_before: Some(experiment.baseline_score.clone()),
            score_after: Some(window_score_from_observation(observation)),
            rollback_performed: true,
            rollback_policy: "rollback-performed".to_owned(),
            cooldown_until_unix_nanos: Some(cooldown_until_unix_nanos),
            manual_restore_command: Some(input.manual_restore_command.to_owned()),
        };

        controller_state.enter_cooldown_after_revert(&input.controller_policy, now_unix_nanos);

        Ok(LiveExperimentOutcome::with_history(
            LiveExperimentEvent::Reverted,
            history_context,
        ))
    }
}

fn controller_journal_path(input: &LiveExperimentManagerInput<'_>) -> PathBuf {
    input
        .controller_journal_path
        .clone()
        .unwrap_or_else(default_controller_journal_path)
}

fn policy_context_for_runtime_apply(observation: &AutotuneObservation) -> DaemonPolicyContext {
    DaemonPolicyContext {
        data_quality_ok: !observation.data_quality.blocks_action(),
        data_quality_reason_code: observation
            .data_quality
            .reason_code_strings()
            .first()
            .cloned(),
        system_health_ok: observation.system_health.ok_for_apply,
        system_health_reason_code: observation.system_health.reason_code.clone(),
        workload_stable: observation.workload_identity.is_some(),
        cooldown_active: false,
        rollback_pending: false,
        capabilities: Some(observation.capabilities.clone()),
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

fn experiment_data_quality(
    quality: &crate::autotune::quality::OnlineDataQuality,
) -> ExperimentDataQuality {
    match quality {
        crate::autotune::quality::OnlineDataQuality::High => ExperimentDataQuality::High,
        crate::autotune::quality::OnlineDataQuality::Medium { .. } => ExperimentDataQuality::Medium,
        crate::autotune::quality::OnlineDataQuality::Low { .. } => ExperimentDataQuality::Low,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::{
        actions::{ActionId, ActionState, RollbackToken, TaskIdentity},
        autotune::{
            candidate::{CandidateDryRunRecord, CandidateEvidence, NiceActionPlan},
            comparison::ExperimentResult,
            objective::ObjectiveKind,
            quality::OnlineDataQuality,
        },
        daemon_policy::ActionSource,
        scorer::StutterScore,
    };

    #[derive(Default)]
    struct FakeLiveExecutor {
        apply_calls: usize,
        rollback_calls: usize,
        fail_apply: bool,
        fail_rollback: bool,
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

            Ok(rollback())
        }

        fn rollback_candidate(
            &mut self,
            _input: &LiveExperimentManagerInput<'_>,
            _experiment: &LiveExperiment,
            _observation: &AutotuneObservation,
        ) -> anyhow::Result<()> {
            self.rollback_calls += 1;

            if self.fail_rollback {
                anyhow::bail!("intentional rollback failure");
            }

            Ok(())
        }
    }

    #[derive(Debug, Default)]
    struct FakePrivilegedService {
        apply_calls: Mutex<usize>,
        rollback_calls: Mutex<usize>,
    }

    impl FakePrivilegedService {
        fn apply_calls(&self) -> usize {
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

    fn low_risk_candidate() -> CandidateAction {
        CandidateAction::fake(
            ActionId("fake-low-risk".to_owned()),
            SafetyClass::ReversibleLowRisk,
        )
    }

    fn medium_risk_candidate() -> CandidateAction {
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

    fn rollback() -> RollbackToken {
        RollbackToken::CpuAffinityRestoreFile {
            path: PathBuf::from("/tmp/stutter-test-rollback.json"),
            affected_tasks: 1,
        }
    }

    fn score(total: u64) -> WindowScore {
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

    fn observation(total: u64, now_unix_nanos: u128) -> AutotuneObservation {
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

    fn live_experiment() -> LiveExperiment {
        LiveExperiment {
            experiment_id: ExperimentId::new("experiment-active"),
            candidate: low_risk_candidate(),
            safety_class: SafetyClass::ReversibleLowRisk,
            mode: DaemonMode::ApplyLowRisk,
            baseline_score: score(1_000),
            baseline_signals: ObjectiveSignals::from_window_score(&score(1_000)),
            applied_unix_nanos: 100,
            washout_until_unix_nanos: 200,
            measure_until_unix_nanos: 300,
            rollback: rollback(),
        }
    }

    fn temp_journal_path(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-live-experiment-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("controller_journal.json")
    }

    fn input(journal_path: PathBuf) -> LiveExperimentManagerInput<'static> {
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

    fn medium_input<'a>(
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
    fn runtime_executor_uses_injected_privileged_service_for_medium_risk_apply() {
        let journal_path = temp_journal_path("medium-injected");
        let service = FakePrivilegedService::default();
        let input = medium_input(journal_path, Some(&service));
        let observation = observation(1_000, 1_000_000_000);
        let mut executor = RuntimeLiveExperimentActionExecutor;

        let rollback = executor
            .apply_candidate(
                &input,
                &medium_risk_candidate(),
                "medium-experiment",
                &observation,
            )
            .unwrap();

        assert!(matches!(rollback, RollbackToken::NiceRestore { .. }));
        assert_eq!(service.apply_calls(), 1);
    }

    #[test]
    fn runtime_executor_requires_privileged_service_for_medium_risk_apply() {
        let journal_path = temp_journal_path("medium-missing-service");
        let input = medium_input(journal_path, None);
        let observation = observation(1_000, 1_000_000_000);
        let mut executor = RuntimeLiveExperimentActionExecutor;

        let err = executor
            .apply_candidate(
                &input,
                &medium_risk_candidate(),
                "medium-experiment",
                &observation,
            )
            .unwrap_err()
            .to_string();

        assert!(err.contains("privileged_worker_required"));
    }

    #[test]
    fn start_candidate_applies_action_writes_journal_registers_rollback_and_clears_window() {
        let journal_path = temp_journal_path("start");
        let registry = crate::autotune::shutdown::ActiveAutotuneActionRegistry::new();
        let daemon_policy = DaemonPolicy::apply_low_risk(ActionSource::Test);
        let input = LiveExperimentManagerInput {
            mode: DaemonMode::ApplyLowRisk,
            controller_policy: ControllerPolicy::from_daemon_policy(&daemon_policy),
            daemon_policy,
            simulate_action_effects: false,
            washout: WashoutWindowConfig::default(),
            candidate_window_seconds: 30,
            manual_restore_command: "stutter daemon emergency-restore",
            controller_journal_path: Some(journal_path),
            exit_rollback_registry: Some(&registry),
            privileged_action_service: None,
        };
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
                    candidate: low_risk_candidate(),
                    reason: "candidate passed gate".to_owned(),
                },
                "candidate passed gate",
                &mut executor,
            )
            .unwrap();

        assert_eq!(outcome.event, LiveExperimentEvent::Started);
        assert!(outcome.clear_measurement_window);
        assert!(manager.has_active_experiment());
        assert_eq!(executor.apply_calls, 1);
        assert_eq!(registry.len(), 1);
        assert_eq!(controller_state.phase, ControllerPhase::Measuring);
        assert!(controller_state.active_experiment.is_some());
        assert_eq!(
            outcome
                .history_context
                .as_ref()
                .map(|context| context.action_id.as_str()),
            Some("fake-low-risk")
        );
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

    #[test]
    fn active_window_decision_tracks_washout_and_measurement_windows() {
        let mut manager = LiveExperimentManager::new();
        manager.set_current_for_tests(live_experiment());

        let washout = manager
            .active_window_decision(&observation(1_000, 150))
            .unwrap();
        assert!(matches!(washout, AutotuneDecision::Noop { .. }));
        assert!(decision_reason(&washout).contains("washout window"));

        let measurement = manager
            .active_window_decision(&observation(1_000, 250))
            .unwrap();
        assert!(matches!(measurement, AutotuneDecision::Noop { .. }));
        assert!(decision_reason(&measurement).contains("measurement window"));

        assert!(
            manager
                .active_window_decision(&observation(1_000, 350))
                .is_none()
        );
    }

    #[test]
    fn keep_current_records_kept_state_and_clears_active_experiment() {
        let journal_path = temp_journal_path("keep");
        let mut manager = LiveExperimentManager::new();
        manager.set_current_for_tests(live_experiment());
        let mut controller_state = ControllerRuntimeState {
            active_experiment: Some(ControllerActiveExperiment {
                experiment_id: ExperimentId::new("experiment-active"),
                candidate: low_risk_candidate(),
                baseline_score_total: 1_000,
            }),
            ..ControllerRuntimeState::default()
        };
        let mut active_profile_state = ActiveProfileState::default();
        let mut executor = FakeLiveExecutor::default();
        let observation = observation(500, 400);

        let outcome = manager
            .apply_decision_side_effects_with_executor(
                input(journal_path),
                LiveExperimentRuntimeState {
                    controller_state: &mut controller_state,
                    active_profile_state: &mut active_profile_state,
                },
                &observation,
                &AutotuneDecision::KeepCurrent {
                    experiment_id: ExperimentId::new("experiment-active"),
                    reason: "candidate improved".to_owned(),
                },
                "candidate improved",
                &mut executor,
            )
            .unwrap();

        assert_eq!(outcome.event, LiveExperimentEvent::Kept);
        assert!(!manager.has_active_experiment());
        assert_eq!(controller_state.phase, ControllerPhase::Cooldown);
        assert!(controller_state.active_experiment.is_none());
        assert_eq!(active_profile_state.kept_action_count(), 1);
        assert_eq!(executor.rollback_calls, 0);
        assert_eq!(
            outcome
                .history_context
                .as_ref()
                .map(|context| context.rollback_performed),
            Some(false)
        );
    }

    #[test]
    fn revert_rolls_back_candidate_and_enters_cooldown() {
        let journal_path = temp_journal_path("revert");
        let mut manager = LiveExperimentManager::new();
        manager.set_current_for_tests(live_experiment());
        let mut controller_state = ControllerRuntimeState {
            active_experiment: Some(ControllerActiveExperiment {
                experiment_id: ExperimentId::new("experiment-active"),
                candidate: low_risk_candidate(),
                baseline_score_total: 1_000,
            }),
            ..ControllerRuntimeState::default()
        };
        let mut active_profile_state = ActiveProfileState::default();
        let mut executor = FakeLiveExecutor::default();
        let observation = observation(1_200, 400);

        let outcome = manager
            .apply_decision_side_effects_with_executor(
                input(journal_path),
                LiveExperimentRuntimeState {
                    controller_state: &mut controller_state,
                    active_profile_state: &mut active_profile_state,
                },
                &observation,
                &AutotuneDecision::Revert {
                    experiment_id: ExperimentId::new("experiment-active"),
                    reason: "candidate regressed".to_owned(),
                },
                "candidate regressed",
                &mut executor,
            )
            .unwrap();

        assert_eq!(outcome.event, LiveExperimentEvent::Reverted);
        assert!(!manager.has_active_experiment());
        assert_eq!(executor.rollback_calls, 1);
        assert_eq!(controller_state.phase, ControllerPhase::Cooldown);
        assert!(controller_state.active_experiment.is_none());
        assert_eq!(
            outcome
                .history_context
                .as_ref()
                .map(|context| context.rollback_performed),
            Some(true)
        );
    }

    #[test]
    fn rollback_failure_keeps_active_experiment_and_returns_error() {
        let journal_path = temp_journal_path("rollback-failure");
        let mut manager = LiveExperimentManager::new();
        manager.set_current_for_tests(live_experiment());
        let mut controller_state = ControllerRuntimeState::default();
        let mut active_profile_state = ActiveProfileState::default();
        let mut executor = FakeLiveExecutor {
            fail_rollback: true,
            ..FakeLiveExecutor::default()
        };
        let observation = observation(1_200, 400);

        let err = manager
            .apply_decision_side_effects_with_executor(
                input(journal_path),
                LiveExperimentRuntimeState {
                    controller_state: &mut controller_state,
                    active_profile_state: &mut active_profile_state,
                },
                &observation,
                &AutotuneDecision::Revert {
                    experiment_id: ExperimentId::new("experiment-active"),
                    reason: "candidate regressed".to_owned(),
                },
                "candidate regressed",
                &mut executor,
            )
            .unwrap_err();

        assert!(err.to_string().contains("intentional rollback failure"));
        assert!(manager.has_active_experiment());
        assert_eq!(executor.rollback_calls, 1);
    }

    #[test]
    fn compare_keep_result_rejects_io_candidate_when_live_io_signal_regresses() {
        let experiment = LiveExperiment {
            experiment_id: ExperimentId::new("io-test"),
            safety_class: SafetyClass::ReversibleMediumRisk,
            mode: DaemonMode::ApplyMediumRisk,
            candidate: CandidateAction::IoPrio {
                plan: crate::autotune::candidate::IoPrioActionPlan {
                    name: "fake-io".to_owned(),
                    action: crate::actions::ioprio::IoPrioAction {
                        targets: vec![crate::actions::TaskIdentity {
                            tid: 99999,
                            process_pid: Some(99999),
                            comm: Some("fake-io".to_owned()),
                            starttime_ticks: None,
                        }],
                        ioprio: crate::actions::ioprio::IoPrioValue::best_effort(0),
                        policy: crate::actions::ioprio::IoPrioPolicy {
                            allow_ioprio_changes: true,
                            strong_block_io_evidence: true,
                            ..Default::default()
                        },
                    },
                    target_root_pid: Some(99999),
                    evidence: Vec::new(),
                    objective: ObjectiveKind::IoLatency,
                },
            },
            baseline_score: score(1_000),
            baseline_signals: ObjectiveSignals {
                block_io_overlap_count: Some(1),
                block_io_worst_latency_ns: Some(2_000_000),
                ..ObjectiveSignals::from_window_score(&score(1_000))
            },
            applied_unix_nanos: 10,
            washout_until_unix_nanos: 20,
            measure_until_unix_nanos: 30,
            rollback: rollback(),
        };
        let candidate_score = score(800);
        let observation = AutotuneObservation {
            target_present: true,
            target_root_pid: Some(99999),
            data_quality: OnlineDataQuality::High,
            objective_signals: ObjectiveSignals {
                block_io_overlap_count: Some(2),
                block_io_worst_latency_ns: Some(3_000_000),
                ..ObjectiveSignals::from_window_score(&candidate_score)
            },
            ..AutotuneObservation::default()
        };

        let result =
            LiveExperimentManager::compare_keep_result(&experiment, &candidate_score, &observation);

        assert!(matches!(result, ExperimentResult::Regressed { .. }));
        assert_eq!(experiment.candidate.objective(), ObjectiveKind::IoLatency);
    }

    #[test]
    fn live_experiment_journal_record_carries_phase_metadata_and_rollback() {
        let manager = LiveExperimentManager::new();
        let journal_path = temp_journal_path("journal-record");
        let input = input(journal_path);
        let observation = observation(1_000, 1_000_000_000);
        let experiment = live_experiment();

        let record = manager.controller_journal_record_for_live_experiment(
            &input,
            &experiment,
            &observation,
            ControllerJournalState::Reverting,
            "rollback_in_progress",
        );

        assert_eq!(record.state(), ControllerJournalState::Reverting);
        assert_eq!(
            record.experiment_action(),
            Some(("experiment-active", "fake-low-risk"))
        );
        assert_eq!(record.candidate.as_deref(), Some("fake-profile"));
        assert_eq!(
            record.target_identity.as_deref(),
            Some("pid:99999:starttime:unknown:active_tasks:1")
        );
        assert_eq!(
            record.verify_result.as_deref(),
            Some("rollback_in_progress")
        );
        assert_eq!(record.mode, Some(DaemonMode::ApplyLowRisk));
        assert_eq!(record.safety_class, Some(SafetyClass::ReversibleLowRisk));
        assert!(record.rollback_token().is_some());
        assert!(record.may_have_mutated_system());
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
}
