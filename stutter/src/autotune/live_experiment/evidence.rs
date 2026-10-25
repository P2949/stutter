//! The objective comparison recorded with every keep or revert.
use super::{
    model::LiveExperiment,
    scoring::{experiment_data_quality, window_score_from_observation},
};
use crate::autotune::{
    objective::{ObjectiveComparisonInput, ObjectiveEvidence, explain_for_objective},
    observation::AutotuneObservation,
};

/// The comparison between an experiment's baseline and the current observation.
pub(super) fn objective_evidence_for(
    experiment: &LiveExperiment,
    observation: &AutotuneObservation,
) -> ObjectiveEvidence {
    let candidate_score = window_score_from_observation(observation);
    ObjectiveEvidence {
        objective: experiment.candidate.objective(),
        baseline_signals: experiment.baseline_signals.clone(),
        candidate_signals: observation.objective_signals.clone(),
        decision: explain_for_objective(ObjectiveComparisonInput {
            objective: experiment.candidate.objective(),
            baseline: &experiment.baseline_score,
            candidate: &candidate_score,
            baseline_signals: &experiment.baseline_signals,
            candidate_signals: &observation.objective_signals,
            data_quality: experiment_data_quality(&observation.data_quality),
            target_disappeared: !observation.target_present,
        }),
    }
}
