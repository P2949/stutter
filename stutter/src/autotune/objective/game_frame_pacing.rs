use super::{
    ObjectiveComparisonInput, ObjectiveSignalQuality,
    common::{improved_with_generic_guard, reject_if_foreground_regressed},
    relative_change::{RelativeChange, classify_relative_change},
};
use crate::autotune::comparison::ExperimentResult;

// Frozen by the frame campaign (`xtask frame-campaign`, docs/validation/frame-pacing-calibration.json):
// selected on calibration pairs by a pre-registered rule before any holdout run existed, then evaluated
// once on an untouched holdout. It is a conservative effect size, not a statistical confidence interval,
// and it is calibrated only on large frame-p99 effects.
/// The symmetric frame-p99 change, in percent, that counts as a real improvement or regression.
pub const MINIMUM_CHANGE_PERCENT: f64 = 50.0;

/// The fewest valid frames either window needs.
pub const MINIMUM_SAMPLES: u64 = 2_400;

// Calibrated values are only ever frozen from the pre-registered grids.
const _: () = assert!(MINIMUM_CHANGE_PERCENT >= 25.0 && MINIMUM_CHANGE_PERCENT <= 75.0);
const _: () = assert!(MINIMUM_SAMPLES >= 300 && MINIMUM_SAMPLES <= 2_400);

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

#[cfg(test)]
mod calibration_tests {
    use super::*;

    /// The shipped threshold and population are the frozen ones: changing either without the other,
    /// or without a new passing holdout, fails here.
    #[test]
    fn the_shipped_rule_is_the_frozen_calibrated_one() {
        let record: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../docs/validation/frame-pacing-calibration.json"
        ))
        .unwrap();
        assert_eq!(
            record["threshold_percent"].as_f64(),
            Some(MINIMUM_CHANGE_PERCENT)
        );
        assert_eq!(
            record["minimum_frame_samples"].as_u64(),
            Some(MINIMUM_SAMPLES)
        );
        assert_eq!(record["holdout"]["passed"], true);
        assert_eq!(record["holdout"]["tally"]["false_keeps"], 0);
        assert_eq!(record["evidence"]["suite"], "live_frame_objective");
        let in_grid = |key: &str, value: f64| {
            record[key]
                .as_array()
                .is_some_and(|grid| grid.iter().any(|entry| entry.as_f64() == Some(value)))
        };
        assert!(in_grid("grid_percent", MINIMUM_CHANGE_PERCENT));
        assert!(in_grid("population_grid", MINIMUM_SAMPLES as f64));
    }
}
