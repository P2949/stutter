use crate::{
    autotune::{
        comparison::ExperimentDataQuality, experiment::WindowScore,
        observation::AutotuneObservation,
    },
    daemon::policy::DaemonPolicyContext,
};

pub(super) fn policy_context_for_runtime_apply(
    observation: &AutotuneObservation,
) -> DaemonPolicyContext {
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
pub(super) fn window_score_from_observation(observation: &AutotuneObservation) -> WindowScore {
    WindowScore {
        started_unix_nanos: observation.now_unix_nanos,
        finished_unix_nanos: observation.now_unix_nanos,
        interval_count: observation.interval_count,
        scored_samples: observation.scored_samples,
        scored_task_count: observation.scored_task_count,
        score: observation.score.clone(),
    }
}
pub(super) fn validate_window_score_for_apply(
    label: &str,
    score: &WindowScore,
) -> anyhow::Result<()> {
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
pub(super) fn experiment_data_quality(
    quality: &crate::autotune::quality::OnlineDataQuality,
) -> ExperimentDataQuality {
    match quality {
        crate::autotune::quality::OnlineDataQuality::High => ExperimentDataQuality::High,
        crate::autotune::quality::OnlineDataQuality::Medium { .. } => ExperimentDataQuality::Medium,
        crate::autotune::quality::OnlineDataQuality::Low { .. } => ExperimentDataQuality::Low,
    }
}
