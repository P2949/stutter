use super::{
    ObjectiveComparisonInput,
    common::{improved_with_generic_guard, objective_regressed},
};
use crate::autotune::comparison::ExperimentResult;

pub(super) fn compare(
    input: ObjectiveComparisonInput<'_>,
    generic_score_result: ExperimentResult,
) -> ExperimentResult {
    let baseline_count = input.baseline_signals.block_io_overlap_count.unwrap_or(0);
    let candidate_count = input.candidate_signals.block_io_overlap_count.unwrap_or(0);
    let baseline_worst = input
        .baseline_signals
        .block_io_worst_latency_ns
        .unwrap_or(0);
    let candidate_worst = input
        .candidate_signals
        .block_io_worst_latency_ns
        .unwrap_or(0);

    if candidate_count > baseline_count || candidate_worst > baseline_worst {
        return objective_regressed(&input);
    }

    if candidate_count < baseline_count
        || (candidate_count == baseline_count && candidate_worst < baseline_worst)
    {
        return improved_with_generic_guard(generic_score_result);
    }

    ExperimentResult::Inconclusive {
        reason: "block I/O objective primary signal did not improve".to_owned(),
    }
}
