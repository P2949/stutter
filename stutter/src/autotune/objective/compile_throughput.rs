use super::{ObjectiveComparisonInput, common::reject_if_foreground_regressed};
use crate::autotune::comparison::ExperimentResult;

pub(super) fn compare(
    input: ObjectiveComparisonInput<'_>,
    _generic_score_result: ExperimentResult,
) -> ExperimentResult {
    if let Some(result) = reject_if_foreground_regressed(&input) {
        return result;
    }

    // Active intervals and scheduler samples measure activity, not completed work.
    // Neither they nor the generic score can establish throughput improvement.
    ExperimentResult::Inconclusive {
        reason: "direct compile throughput unavailable; compile activity is not completed work"
            .to_owned(),
    }
}
