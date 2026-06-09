use super::*;
use crate::autotune::{
    active_config::{RollbackVerification, verify_rollback_restored_baseline},
    observation::{ActiveConfigSnapshot, AutotuneObservation},
};

pub(super) fn rollback_verification_for_experiment(
    experiment: &LiveExperiment,
    post_rollback_active_config: Option<&ActiveConfigSnapshot>,
    observation: &AutotuneObservation,
) -> Option<RollbackVerification> {
    let baseline = experiment.baseline_active_config.as_ref()?;
    Some(match post_rollback_active_config {
        Some(post_rollback) => verify_rollback_restored_baseline(
            &experiment.candidate,
            baseline,
            post_rollback,
            &observation.active_tasks,
        ),
        None => RollbackVerification {
            verified: false,
            expected: "post-rollback active config snapshot".to_owned(),
            actual: "missing".to_owned(),
            reason_code: "rollback_verification_unavailable".to_owned(),
        },
    })
}
pub(super) fn log_rollback_verification(
    experiment: &LiveExperiment,
    verification: &RollbackVerification,
) {
    if verification.verified {
        log::info!(
            "autotune_rollback_verification_passed experiment_id={} action_id={} expected={} actual={}",
            experiment.experiment_id.as_str(),
            experiment.action_id(),
            verification.expected,
            verification.actual
        );
    } else {
        log::error!(
            "autotune_rollback_verification_failed experiment_id={} action_id={} reason_code={} expected={} actual={}",
            experiment.experiment_id.as_str(),
            experiment.action_id(),
            verification.reason_code,
            verification.expected,
            verification.actual
        );
    }
}
