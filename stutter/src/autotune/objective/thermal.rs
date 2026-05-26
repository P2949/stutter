use super::{
    ObjectiveComparisonInput,
    common::{improved_with_generic_guard, objective_regressed},
};
use crate::autotune::comparison::ExperimentResult;

pub(super) fn compare(
    input: ObjectiveComparisonInput<'_>,
    generic_score_result: ExperimentResult,
) -> ExperimentResult {
    let baseline_degraded = input.baseline_signals.thermal_degraded.unwrap_or(false);
    let candidate_degraded = input.candidate_signals.thermal_degraded.unwrap_or(false);
    let baseline_throttles = input.baseline_signals.thermal_throttle_count.unwrap_or(0);
    let candidate_throttles = input.candidate_signals.thermal_throttle_count.unwrap_or(0);

    if candidate_degraded && !baseline_degraded || candidate_throttles > baseline_throttles {
        return objective_regressed(&input);
    }

    if baseline_degraded && !candidate_degraded || candidate_throttles < baseline_throttles {
        return improved_with_generic_guard(generic_score_result);
    }

    ExperimentResult::Inconclusive {
        reason: "thermal objective primary signal did not improve".to_owned(),
    }
}
