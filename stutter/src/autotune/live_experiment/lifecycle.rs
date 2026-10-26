use std::path::PathBuf;

use super::{
    evidence::objective_evidence_for,
    executor::LiveExperimentActionExecutor,
    journal::{
        controller_journal_metadata_for_candidate, controller_journal_path,
        write_controller_journal_phase_for_live_experiment,
    },
    manager::{LiveExperimentManager, LiveExperimentRuntimeState},
    model::{
        LiveExperiment, LiveExperimentEvent, LiveExperimentHistoryContext,
        LiveExperimentManagerInput, LiveExperimentOutcome,
    },
    rollback::{log_rollback_verification, rollback_verification_for_experiment},
    scoring::{
        experiment_data_quality, validate_window_score_for_apply, window_score_from_observation,
    },
};
use crate::{
    actions::RollbackToken,
    autotune::{
        candidate_memory::CandidateMemoryResult,
        comparison::ExperimentResult,
        controller::{
            ActiveExperiment as ControllerActiveExperiment, ControllerCandidateResultInput,
            ControllerRuntimeState,
        },
        controller_journal::{
            ControllerJournalState, write_controller_journal_applied_with_metadata,
            write_controller_journal_applying_with_metadata,
        },
        decision::AutotuneDecision,
        experiment::ExperimentId,
        kept::{ActiveProfileState, KeptCandidateState},
        objective::{ObjectiveComparisonInput, ObjectiveEvidence, explain_for_objective},
        observation::AutotuneObservation,
        planning::candidate::CandidateAction,
        state::ControllerPhase,
    },
};
impl LiveExperimentManager {
    pub(crate) fn apply_decision_side_effects_with_executor<
        E: LiveExperimentActionExecutor + ?Sized,
    >(
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
                // A cooldown never clears a fault: only recovery or a restart does.
                let state = &mut *runtime_state.controller_state;
                if state.phase != ControllerPhase::Faulted {
                    state.phase = ControllerPhase::Cooldown;
                }
                let until = observation
                    .now_unix_nanos
                    .saturating_add(duration.as_nanos());
                state.cooldown_until_unix_nanos = Some(until);
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

    pub(crate) fn start_candidate_experiment_with_executor<
        E: LiveExperimentActionExecutor + ?Sized,
    >(
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
        Self::validate_autonomous_support(input, controller_state, observation, &candidate)?;

        let baseline_score = window_score_from_observation(observation);
        validate_window_score_for_apply("baseline", &baseline_score)?;

        let experiment_id = ExperimentId::new(format!(
            "live-{}:{}:{}",
            input.mode.as_str(),
            candidate.profile_name(),
            observation.now_unix_nanos
        ));
        let action_id = candidate.action_id();
        let action_kind = candidate.action_kind().to_owned();
        let safety_class = candidate.safety_class();

        self.failed_apply_context = None;
        let rollback = match self.apply_candidate_for_runtime_with_executor(
            input,
            executor,
            &candidate,
            experiment_id.as_str(),
            action_id.as_str(),
            observation,
        ) {
            Ok(token) => token,
            Err(error) => {
                let failure = error
                    .downcast_ref::<crate::autotune::action_transaction::JournaledActionFailure>();
                let rollback_completed = failure.is_some_and(|failure| failure.rollback_completed);
                let outstanding = failure
                    .filter(|failure| !failure.rollback_completed)
                    .and_then(|failure| failure.rollback.clone());
                let rollback_policy = if outstanding.is_some() {
                    "manual-restore-required"
                } else if rollback_completed {
                    "rollback-completed"
                } else {
                    "rollback-state-unknown"
                };
                self.failed_apply_context = Some(LiveExperimentHistoryContext {
                    experiment_id: experiment_id.clone(),
                    action_id: action_id.clone(),
                    candidate_name: candidate.profile_name().to_owned(),
                    action_kind: action_kind.clone(),
                    mode: input.mode,
                    safety_class: safety_class.clone(),
                    score_before: Some(baseline_score.clone()),
                    score_after: None,
                    rollback_performed: rollback_completed,
                    rollback_policy: rollback_policy.to_owned(),
                    cooldown_until_unix_nanos: None,
                    manual_restore_command: Some(input.manual_restore_command.to_owned()),
                    objective_evidence: None,
                });
                controller_state.phase = ControllerPhase::Faulted;
                if let Some(rollback) = outstanding {
                    controller_state.active_experiment = Some(ControllerActiveExperiment {
                        baseline_signals: observation.objective_signals.clone(),
                        experiment_id: experiment_id.clone(),
                        candidate: candidate.clone(),
                        baseline_score: baseline_score.clone(),
                    });
                    self.current = Some(LiveExperiment {
                        experiment_id,
                        candidate,
                        safety_class,
                        mode: input.mode,
                        baseline_score,
                        baseline_signals: observation.objective_signals.clone(),
                        baseline_active_config: observation.active_config_snapshot.clone(),
                        applied_unix_nanos: observation.now_unix_nanos,
                        washout_until_unix_nanos: observation.now_unix_nanos,
                        measure_until_unix_nanos: observation.now_unix_nanos,
                        rollback,
                    });
                }
                return Err(error);
            }
        };

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
            baseline_signals: observation.objective_signals.clone(),
            experiment_id: experiment_id.clone(),
            candidate: candidate.clone(),
            baseline_score: baseline_score.clone(),
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
            baseline_active_config: observation.active_config_snapshot.clone(),
            applied_unix_nanos: observation.now_unix_nanos,
            washout_until_unix_nanos,
            measure_until_unix_nanos,
            rollback,
        });

        if let Some(experiment) = self.current.as_ref() {
            write_controller_journal_phase_for_live_experiment(
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
                .map(|experiment| experiment.experiment_id.clone())
                .unwrap_or_else(|| ExperimentId::new("unknown-experiment")),
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
            objective_evidence: None,
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

    pub(crate) fn apply_candidate_for_runtime_with_executor<
        E: LiveExperimentActionExecutor + ?Sized,
    >(
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
        // The privileged service still uses its historical protocol. Low-risk
        // actions own these transitions inside the audited action boundary.
        let deferred_privileged_journal = input.mode
            == crate::daemon_policy::DaemonMode::ApplyMediumRisk
            && candidate.safety_class() > crate::actions::SafetyClass::ReversibleLowRisk;
        if deferred_privileged_journal {
            write_controller_journal_applying_with_metadata(
                &journal_path,
                ExperimentId::try_new(experiment_id)?,
                crate::actions::ActionId::try_new(action_id)?,
                controller_journal_metadata_for_candidate(
                    input,
                    candidate,
                    observation,
                    None,
                    "pending_apply",
                ),
            )?;
        }

        let applied_rollback =
            executor.apply_candidate(input, candidate, experiment_id, observation)?;

        #[cfg(test)]
        if input.mode == crate::daemon_policy::DaemonMode::ApplyLowRisk
            && let Some(registry) = input.exit_rollback_registry
        {
            crate::autotune::shutdown::register_cpu_affinity_rollback(
                registry,
                action_id.to_owned(),
                applied_rollback.clone(),
            );
        }

        if deferred_privileged_journal {
            write_controller_journal_applied_with_metadata(
                &journal_path,
                ExperimentId::try_new(experiment_id)?,
                crate::actions::ActionId::try_new(action_id)?,
                applied_rollback.clone(),
                controller_journal_metadata_for_candidate(
                    input,
                    candidate,
                    observation,
                    Some(applied_rollback.affected_tasks()),
                    "applied_pending_verify",
                ),
            )?;
        } else {
            let record =
                crate::autotune::controller_journal::read_controller_journal(&journal_path)?;
            anyhow::ensure!(
                record.state() == ControllerJournalState::Applied
                    && record.rollback_token() == Some(&applied_rollback)
                    && record.experiment_id.as_ref().map(|id| id.as_str()) == Some(experiment_id)
                    && record.action_id.as_ref().map(|id| id.as_str()) == Some(action_id),
                "live executor returned without its durable Applied rollback token"
            );
        }

        Ok(applied_rollback)
    }

    pub(crate) fn keep_current_experiment_with_executor<
        E: LiveExperimentActionExecutor + ?Sized,
    >(
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
        let objective_decision = explain_for_objective(ObjectiveComparisonInput {
            objective: experiment.candidate.objective(),
            baseline: &experiment.baseline_score,
            candidate: &candidate_score,
            baseline_signals: &experiment.baseline_signals,
            candidate_signals: &observation.objective_signals,
            data_quality: experiment_data_quality(&observation.data_quality),
            target_disappeared: !observation.target_present,
        });
        debug_assert_eq!(result, objective_decision.experiment_result());
        let evidence = ObjectiveEvidence {
            objective: experiment.candidate.objective(),
            baseline_signals: experiment.baseline_signals.clone(),
            candidate_signals: observation.objective_signals.clone(),
            decision: objective_decision.clone(),
        };

        match result {
            ExperimentResult::Improved { .. } => {
                if let Err(err) = Self::commit_keep_durably(input, &experiment, observation) {
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
                    Self::retire_kept_durably(input, &experiment.experiment_id);
                    self.current = Some(experiment);
                    return Err(err);
                }

                controller_state.record_candidate_result(ControllerCandidateResultInput {
                    candidate: &experiment.candidate,
                    observation,
                    cpu_topology_signature: None,
                    result: CandidateMemoryResult::Kept,
                    diagnostic_baseline_raw_score_total: Some(
                        experiment.baseline_score.score.total,
                    ),
                    diagnostic_current_raw_score_total: Some(candidate_score.score.total),
                    rollback_reason: None,
                    cooldown_expires_unix_nanos: Some(
                        observation
                            .now_unix_nanos
                            .saturating_add(input.controller_policy.cooldown_after_keep.as_nanos()),
                    ),
                });

                let cooldown_until_unix_nanos = observation
                    .now_unix_nanos
                    .saturating_add(input.controller_policy.cooldown_after_keep.as_nanos());

                let history_context = LiveExperimentHistoryContext {
                    experiment_id: experiment.experiment_id.clone(),
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
                    objective_evidence: Some(evidence.clone()),
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
                self.pending_objective_evidence = Some(evidence);
                self.rollback_active_experiment_with_executor(
                    input,
                    controller_state,
                    observation,
                    observation.now_unix_nanos,
                    &format!(
                        "candidate was not improved at keep point: {other:?}; {}",
                        objective_decision.summary()
                    ),
                    executor,
                )
            }
        }
    }

    pub(crate) fn rollback_active_experiment_with_executor<
        E: LiveExperimentActionExecutor + ?Sized,
    >(
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

        if let Err(err) = write_controller_journal_phase_for_live_experiment(
            input,
            &experiment,
            observation,
            ControllerJournalState::Reverting,
            "rollback_in_progress",
        ) {
            self.current = Some(experiment);
            return Err(err);
        }

        let post_rollback_active_config = if input.simulate_action_effects {
            None
        } else {
            match executor.rollback_candidate(input, &experiment, observation) {
                Ok(snapshot) => snapshot,
                Err(err) => {
                    self.current = Some(experiment);
                    return Err(err);
                }
            }
        };

        if !input.simulate_action_effects
            && let Some(verification) = rollback_verification_for_experiment(
                &experiment,
                post_rollback_active_config.as_ref(),
                observation,
            )
        {
            log_rollback_verification(&experiment, &verification);
            if !verification.verified {
                let verify_result = format!(
                    "{} expected={} actual={}",
                    verification.reason_code, verification.expected, verification.actual
                );
                write_controller_journal_phase_for_live_experiment(
                    input,
                    &experiment,
                    observation,
                    ControllerJournalState::Faulted,
                    &verify_result,
                )?;

                controller_state.record_candidate_result(ControllerCandidateResultInput {
                    candidate: &experiment.candidate,
                    observation,
                    cpu_topology_signature: None,
                    result: CandidateMemoryResult::Faulted,
                    diagnostic_baseline_raw_score_total: Some(
                        experiment.baseline_score.score.total,
                    ),
                    diagnostic_current_raw_score_total: Some(observation.score.total),
                    rollback_reason: Some(format!(
                        "rollback verification failed: {} expected={} actual={}",
                        verification.reason_code, verification.expected, verification.actual
                    )),
                    cooldown_expires_unix_nanos: Some(
                        now_unix_nanos.saturating_add(
                            input.controller_policy.cooldown_after_fault.as_nanos(),
                        ),
                    ),
                });

                let cooldown_until_unix_nanos = now_unix_nanos
                    .saturating_add(input.controller_policy.cooldown_after_fault.as_nanos());
                let history_context = LiveExperimentHistoryContext {
                    experiment_id: experiment.experiment_id.clone(),
                    action_id: experiment.action_id(),
                    candidate_name: experiment.candidate.profile_name().to_owned(),
                    action_kind: experiment.candidate.action_kind().to_owned(),
                    mode: experiment.mode,
                    safety_class: experiment.safety_class.clone(),
                    score_before: Some(experiment.baseline_score.clone()),
                    score_after: Some(window_score_from_observation(observation)),
                    rollback_performed: true,
                    rollback_policy: format!(
                        "rollback-verification-failed:{}",
                        verification.reason_code
                    ),
                    cooldown_until_unix_nanos: Some(cooldown_until_unix_nanos),
                    manual_restore_command: Some(input.manual_restore_command.to_owned()),
                    objective_evidence: self
                        .pending_objective_evidence
                        .take()
                        .or_else(|| Some(objective_evidence_for(&experiment, observation))),
                };

                controller_state
                    .enter_cooldown_after_fault(&input.controller_policy, now_unix_nanos);
                self.current = Some(experiment);

                return Ok(LiveExperimentOutcome::with_history(
                    LiveExperimentEvent::Faulted,
                    history_context,
                ));
            }
        }

        write_controller_journal_phase_for_live_experiment(
            input,
            &experiment,
            observation,
            ControllerJournalState::Reverted,
            if input.simulate_action_effects {
                "rollback_simulated"
            } else {
                "rollback_verified"
            },
        )?;

        controller_state.record_candidate_result(ControllerCandidateResultInput {
            candidate: &experiment.candidate,
            observation,
            cpu_topology_signature: None,
            result: CandidateMemoryResult::Reverted,
            diagnostic_baseline_raw_score_total: Some(experiment.baseline_score.score.total),
            diagnostic_current_raw_score_total: Some(observation.score.total),
            rollback_reason: Some(reason.to_owned()),
            cooldown_expires_unix_nanos: Some(
                now_unix_nanos
                    .saturating_add(input.controller_policy.cooldown_after_revert.as_nanos()),
            ),
        });

        let cooldown_until_unix_nanos =
            now_unix_nanos.saturating_add(input.controller_policy.cooldown_after_revert.as_nanos());

        let history_context = LiveExperimentHistoryContext {
            experiment_id: experiment.experiment_id.clone(),
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
            objective_evidence: self
                .pending_objective_evidence
                .take()
                .or_else(|| Some(objective_evidence_for(&experiment, observation))),
        };

        controller_state.enter_cooldown_after_revert(&input.controller_policy, now_unix_nanos);

        Ok(LiveExperimentOutcome::with_history(
            LiveExperimentEvent::Reverted,
            history_context,
        ))
    }
}
