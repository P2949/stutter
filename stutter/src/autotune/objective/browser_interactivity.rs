use super::{ObjectiveComparisonInput, common::compare_interactivity_objective};
use crate::autotune::comparison::ExperimentResult;

pub(super) fn compare(
    input: ObjectiveComparisonInput<'_>,
    generic_score_result: ExperimentResult,
) -> ExperimentResult {
    compare_interactivity_objective(input, generic_score_result)
}
