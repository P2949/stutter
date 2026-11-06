use super::{
    ObjectiveComparisonInput, ObjectiveSignalQuality,
    common::{
        PrimaryMetricComparison, frame_p99_comparison, improved_with_generic_guard,
        reject_if_foreground_regressed,
    },
};
use crate::autotune::comparison::ExperimentResult;

/// A frame-pacing decision is direct-only: the primary metric must come from a trusted frame source
/// (`ObjectiveSignalQuality::Direct`) in both windows. The normalized score is diagnostic and can never
/// turn missing, derived or unchanged frame evidence into an improvement.
pub(super) fn compare(
    input: ObjectiveComparisonInput<'_>,
    generic_score_result: ExperimentResult,
) -> ExperimentResult {
    if let Some(result) = reject_if_foreground_regressed(&input) {
        return result;
    }
    for (side, signals) in [
        ("baseline", input.baseline_signals),
        ("candidate", input.candidate_signals),
    ] {
        if signals.signal_quality.frame_pacing != ObjectiveSignalQuality::Direct {
            return ExperimentResult::Inconclusive {
                reason: format!(
                    "{side}: no trusted direct frame source; frame pacing is direct-only and does not fall back to the normalized score"
                ),
            };
        }
    }

    match frame_p99_comparison(&input, 1.0) {
        PrimaryMetricComparison::Improved => improved_with_generic_guard(generic_score_result),
        PrimaryMetricComparison::Regressed => super::common::objective_regressed(&input),
        PrimaryMetricComparison::Unchanged | PrimaryMetricComparison::Missing => {
            ExperimentResult::Inconclusive {
                reason: "frame p99 unchanged or missing; no direct frame improvement".to_owned(),
            }
        }
    }
}
