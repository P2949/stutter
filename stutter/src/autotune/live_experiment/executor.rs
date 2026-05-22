use std::time::Duration;

use anyhow::Context;

use super::{
    journal::collect_post_rollback_active_config, scoring::policy_context_for_runtime_apply, *,
};
use crate::{
    actions::{RollbackToken, SafetyClass},
    autotune::{
        apply_low_risk::apply_candidate_with_audit,
        candidate::CandidateAction,
        observation::{ActiveConfigSnapshot, AutotuneObservation},
    },
    daemon::{
        policy::DaemonMode,
        privilege::{
            CandidateApplyRequest, CandidatePlanRequest, PrivilegedActionService, RollbackRequest,
        },
    },
};

pub(super) trait LiveExperimentActionExecutor {
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
pub(super) struct RuntimeLiveExperimentActionExecutor;

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
            let outcome = apply_candidate_with_audit(candidate.clone())?;
            log::info!(
                "autotune_candidate_applied candidate={} action_kind={} affected_tasks={} safety_class={:?} state={:?}",
                &outcome.candidate_name,
                &outcome.action_kind,
                outcome.affected_tasks,
                &outcome.safety_class,
                &outcome.state
            );
            Ok(outcome.rollback)
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
