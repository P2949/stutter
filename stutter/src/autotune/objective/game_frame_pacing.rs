use super::{
    ObjectiveComparisonInput, ObjectiveSignalQuality,
    common::{improved_with_generic_guard, reject_if_foreground_regressed},
    relative_change::{RelativeChange, classify_relative_change},
};
use crate::autotune::comparison::ExperimentResult;

/// Provisional until the frame campaign (`xtask frame-campaign`) freezes it: the symmetric frame-p99
/// change, in percent, that counts as a real improvement or regression.
pub const MINIMUM_CHANGE_PERCENT: f64 = 20.0;

/// Provisional until the frame campaign freezes it: the fewest valid frames either window needs.
pub const MINIMUM_SAMPLES: u64 = 600;

/// A frame-pacing decision is direct-only: the primary metric must come from a trusted frame source
/// (`ObjectiveSignalQuality::Direct`) with a sufficient frame population in both windows, and the
/// change must clear the calibrated threshold. The normalized score is a veto guard only; it can
/// never turn missing, derived, sparse or unchanged frame evidence into an improvement.
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
        if signals.frame_samples.unwrap_or(0) < MINIMUM_SAMPLES {
            return ExperimentResult::Inconclusive {
                reason: format!(
                    "{side}: fewer than {MINIMUM_SAMPLES} valid frames for a frame-pacing decision"
                ),
            };
        }
    }
    let (Some(baseline), Some(candidate)) = (
        input.baseline_signals.frame_p99_ms,
        input.candidate_signals.frame_p99_ms,
    ) else {
        return ExperimentResult::Inconclusive {
            reason: "frame p99 missing; no direct frame evidence".to_owned(),
        };
    };
    if !(baseline.is_finite() && candidate.is_finite() && baseline > 0.0 && candidate > 0.0) {
        return ExperimentResult::Inconclusive {
            reason: "frame p99 is not a positive finite value".to_owned(),
        };
    }
    match classify_relative_change(baseline, candidate, MINIMUM_CHANGE_PERCENT) {
        RelativeChange::Regressed(regression_percent) => {
            ExperimentResult::Regressed { regression_percent }
        }
        RelativeChange::Improved(_) => improved_with_generic_guard(generic_score_result),
        RelativeChange::Inconclusive(change_percent) => ExperimentResult::Inconclusive {
            reason: format!(
                "frame p99 change {change_percent:.2}% did not meet the {MINIMUM_CHANGE_PERCENT}% threshold"
            ),
        },
    }
}
