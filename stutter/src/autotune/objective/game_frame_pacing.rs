use super::{
    ObjectiveComparisonInput,
    common::{
        PrimaryMetricComparison, frame_p99_comparison, improved_with_generic_guard,
        objective_regressed, reject_if_foreground_regressed,
    },
};
use crate::autotune::comparison::ExperimentResult;

pub(super) fn compare(
    input: ObjectiveComparisonInput<'_>,
    generic_score_result: ExperimentResult,
) -> ExperimentResult {
    if let Some(result) = reject_if_foreground_regressed(&input) {
        return result;
    }

    match frame_p99_comparison(&input, 1.0) {
        PrimaryMetricComparison::Improved => improved_with_generic_guard(generic_score_result),
        PrimaryMetricComparison::Regressed => objective_regressed(&input),
        PrimaryMetricComparison::Unchanged | PrimaryMetricComparison::Missing => {
            generic_score_result
        }
    }
}
