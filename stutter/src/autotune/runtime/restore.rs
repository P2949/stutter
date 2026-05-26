use super::{AutotuneRuntime, planning::plan_has_deny_reason};
use crate::autotune::{
    active_config::{ActiveConfigMatch, ActiveConfigMatchInput},
    decision::AutotuneDecision,
    experiment::ExperimentId,
    external_mutation::{
        ExternalMutationRecoveryDecision, recovery_decision_for_active_experiment,
        recovery_decision_for_kept_action,
    },
    observation::AutotuneObservation,
    planner::CandidateDenyReason,
    planning::candidate::CandidateAction,
    state::ControllerPhase,
};

impl AutotuneRuntime {
    pub(super) fn active_experiment_external_mutation_decision(
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

    pub(super) fn plan_external_mutation_recovery_decision(&mut self) -> Option<AutotuneDecision> {
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

    pub(super) fn rollback_live_experiment(
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
