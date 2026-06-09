use super::{ObjectiveComparisonInput, common::reject_if_frame_pacing_regressed};
use crate::autotune::comparison::ExperimentResult;

pub(super) fn compare(
    input: ObjectiveComparisonInput<'_>,
    generic_score_result: ExperimentResult,
) -> ExperimentResult {
    reject_if_frame_pacing_regressed(&input).unwrap_or(generic_score_result)
}
