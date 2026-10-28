use std::time::Duration;

use anyhow::Context;

use super::{
    journal::collect_post_rollback_active_config, scoring::policy_context_for_runtime_apply, *,
};
use crate::{
    actions::{RollbackToken, SafetyClass, TuningAction},
    autotune::{
        apply_low_risk::action_from_candidate,
        observation::{ActiveConfigSnapshot, AutotuneObservation},
        planning::candidate::CandidateAction,
    },
    daemon::{
        policy::DaemonMode,
        privilege::{CandidateApplyRequest, CandidatePlanRequest, RollbackRequest},
    },
};

pub(crate) trait LiveExperimentActionExecutor {
    /// Low-risk implementations must persist Applying and Applied(token) at the
    /// action boundary, before verification. Returning a token alone is insufficient.
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
    ) -> anyhow::Result<Option<ActiveConfigSnapshot>>;
}
pub(crate) struct RuntimeLiveExperimentActionExecutor;

impl LiveExperimentActionExecutor for RuntimeLiveExperimentActionExecutor {
    fn apply_candidate(
        &mut self,
        input: &LiveExperimentManagerInput<'_>,
        candidate: &CandidateAction,
        experiment_id: &str,
        observation: &AutotuneObservation,
    ) -> anyhow::Result<RollbackToken> {
        if input.mode == DaemonMode::ApplyMediumRisk
            && candidate.safety_class() > SafetyClass::ReversibleLowRisk
        {
            let service = input
                .privileged_action_service
                .ok_or_else(|| anyhow::anyhow!("privileged_worker_required: apply-medium-risk requires a privileged action service"))?;
            let request = CandidateApplyRequest {
                plan: CandidatePlanRequest::from_candidate(
                    candidate.clone(),
                    observation.now_unix_nanos,
                ),
                policy: input.daemon_policy.clone(),
                context: policy_context_for_runtime_apply(observation),
                max_plan_age_nanos: Duration::from_secs(30).as_nanos(),
            };
            let result = if candidate.operation_kind().prepared_transaction_implemented() {
                // Prepare -> journal Applying(token) durably -> commit. The worker performs no write
                // to the target until this journal write has succeeded.
                let journal_path = super::journal::controller_journal_path(input);
                let journaled = std::cell::RefCell::new(None);
                let applied = service.apply_candidate_prepared(request, &mut |token| {
                    crate::autotune::controller_journal::write_controller_journal_phase_with_metadata(
                        &journal_path,
                        crate::autotune::controller_journal::ControllerJournalState::Applying,
                        crate::autotune::experiment::ExperimentId::try_new(experiment_id)?,
                        candidate.action_id(),
                        Some(token.clone()),
                        super::journal::controller_journal_metadata_for_candidate(
                            input,
                            candidate,
                            observation,
                            Some(token.affected_tasks()),
                            "rollback_prepared_before_mutation",
                        ),
                    )
                    .map(|_| ())?;
                    *journaled.borrow_mut() = Some(token.clone());
                    Ok(())
                });
                match applied {
                    Ok(applied) => Ok(applied),
                    Err(error) => Err(settle_failed_prepared_apply(
                        service,
                        input,
                        candidate,
                        observation,
                        &journal_path,
                        journaled.into_inner(),
                        error,
                    )),
                }
            } else {
                service.apply_candidate(request)
            }
            .with_context(|| {
                    format!(
                        "privileged apply failed for autotune candidate '{}'",
                        candidate.candidate_name()
                    )
                })?;
            Ok(result.rollback)
        } else {
            let (name, mut action) = action_from_candidate(candidate.clone())?;
            let journal_path = super::journal::controller_journal_path(input);
            action.restore_path = Some(super::journal::reserve_experiment_restore_path(
                &journal_path,
            )?);
            let audit_path = input
                .audit_path
                .clone()
                .unwrap_or_else(crate::audit::default_audit_log_path);
            let outcome = crate::autotune::action_transaction::apply_journaled_action(
                &action,
                crate::actions::runner::ActionRunPolicy::apply_low_risk(
                    crate::daemon_policy::ActionSource::AutotuneRuntime,
                    false,
                ),
                crate::autotune::action_transaction::JournaledActionInput {
                    journal_path: &journal_path,
                    audit_path: &audit_path,
                    experiment_id,
                    metadata: super::journal::controller_journal_metadata_for_candidate(
                        input,
                        candidate,
                        observation,
                        None,
                        "pending_apply",
                    ),
                },
            )?;
            log::info!(
                "autotune_candidate_applied candidate={} action_kind={} affected_tasks={} safety_class={:?} state={:?}",
                name,
                "cpu_affinity_profile",
                outcome.state.affected_tasks,
                action.safety_class(),
                outcome.state
            );
            outcome
                .rollback
                .context("journaled apply returned without rollback token")
        }
    }

    fn rollback_candidate(
        &mut self,
        input: &LiveExperimentManagerInput<'_>,
        experiment: &LiveExperiment,
        observation: &AutotuneObservation,
    ) -> anyhow::Result<Option<ActiveConfigSnapshot>> {
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
            let executor = crate::autotune::apply::executor_for_candidate_preview(
                experiment.candidate.clone(),
            )?;
            executor.rollback(&experiment.rollback)?;
        }

        Ok(collect_post_rollback_active_config(observation))
    }
}

/// A prepared apply that fails must not leave the controller journal at `applying`: that state
/// means a token is outstanding. Without a journalled token the worker never mutated anything
/// (it writes only after the commit that follows the journal), so the stale pending entry is
/// cleared. With one, the commit failed or its answer was lost: the token is rolled back through
/// the worker and only then is the journal cleared; if that rollback fails the token stays
/// journalled for startup recovery, which is the safe direction.
fn settle_failed_prepared_apply(
    service: &dyn crate::daemon::privilege::PrivilegedActionService,
    input: &LiveExperimentManagerInput<'_>,
    candidate: &CandidateAction,
    observation: &AutotuneObservation,
    journal_path: &std::path::Path,
    token: Option<RollbackToken>,
    error: anyhow::Error,
) -> anyhow::Error {
    let clear = |what: &str| -> anyhow::Result<()> {
        crate::autotune::controller_journal::write_controller_journal_clean(journal_path)
            .map(|_| ())
            .with_context(|| format!("failed to clear the controller journal {what}"))
    };
    let Some(token) = token else {
        return match clear("after a prepare that never reached the commit") {
            Ok(()) => error,
            Err(clear_error) => error.context(format!("{clear_error:#}")),
        };
    };
    let rollback = service.rollback(RollbackRequest {
        candidate: candidate.clone(),
        token,
        policy: input.daemon_policy.clone(),
        context: policy_context_for_runtime_apply(observation),
    });
    match rollback {
        Ok(_) => match clear("after rolling back a failed apply") {
            Ok(()) => error.context("the failed apply was rolled back and the journal cleared"),
            Err(clear_error) => error.context(format!("{clear_error:#}")),
        },
        Err(rollback_error) => error.context(format!(
            "rolling back the failed apply also failed ({rollback_error:#}); the journal keeps the token for startup recovery"
        )),
    }
}
