use super::{
    ObjectiveComparisonInput, ObjectiveSignalQuality, ObjectiveSignals,
    common::reject_if_frame_pacing_regressed, runnable_signal::RUNNABLE_HISTOGRAM_SOURCE,
};
use crate::autotune::comparison::ExperimentResult;

/// Provisional conservative thresholds, pending real-system calibration. A p99
/// needs at least 1,000 events (roughly ten observations in its upper 1%).
pub const MINIMUM_RUNNABLE_SAMPLES: u64 = 1_000;
pub const DEFAULT_MIN_IMPROVEMENT_PERCENT: f64 = 12.5;
pub const DEFAULT_MAX_REGRESSION_PERCENT: f64 = 7.5;

/// Decision thresholds of the runnable-latency comparison. The defaults are the product
/// contract; a graduation campaign sweeps nearby values over recorded data before any change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RunnableThresholds {
    pub min_improvement_percent: f64,
    pub max_regression_percent: f64,
    pub min_samples: u64,
}

impl Default for RunnableThresholds {
    fn default() -> Self {
        Self {
            min_improvement_percent: DEFAULT_MIN_IMPROVEMENT_PERCENT,
            max_regression_percent: DEFAULT_MAX_REGRESSION_PERCENT,
            min_samples: MINIMUM_RUNNABLE_SAMPLES,
        }
    }
}

/// One window's whole-window runnable-latency population.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunnableSummary {
    pub p99_ns: u64,
    pub samples: u64,
    pub over_5ms: u64,
}

/// The keep/revert decision over two populations. Pure: target identity, data quality and frame
/// guardrails are checked by the caller.
pub fn decide_runnable(
    baseline: RunnableSummary,
    candidate: RunnableSummary,
    thresholds: RunnableThresholds,
) -> ExperimentResult {
    if baseline.samples < thresholds.min_samples || candidate.samples < thresholds.min_samples {
        return ExperimentResult::Inconclusive {
            reason: format!(
                "insufficient runnable-latency samples: baseline {} candidate {} < {}",
                baseline.samples, candidate.samples, thresholds.min_samples
            ),
        };
    }
    if u128::from(candidate.over_5ms) * u128::from(baseline.samples)
        > u128::from(baseline.over_5ms) * u128::from(candidate.samples)
    {
        return ExperimentResult::Regressed {
            regression_percent: tail_regression_percent(baseline, candidate),
        };
    }
    let change_percent = if baseline.p99_ns == 0 {
        if candidate.p99_ns > 0 { 100.0 } else { 0.0 }
    } else {
        (candidate.p99_ns as f64 - baseline.p99_ns as f64) / baseline.p99_ns as f64 * 100.0
    };
    if change_percent >= thresholds.max_regression_percent {
        ExperimentResult::Regressed {
            regression_percent: change_percent,
        }
    } else if -change_percent >= thresholds.min_improvement_percent {
        ExperimentResult::Improved {
            improvement_percent: -change_percent,
        }
    } else {
        ExperimentResult::Inconclusive {
            reason: format!(
                "runnable p99 change {change_percent:.2}% did not meet conservative thresholds (improvement {}%, regression {}%); no score fallback",
                thresholds.min_improvement_percent, thresholds.max_regression_percent
            ),
        }
    }
}

pub(super) fn compare(
    input: ObjectiveComparisonInput<'_>,
    _generic_score_result: ExperimentResult,
) -> ExperimentResult {
    let baseline_population = match population(input.baseline_signals) {
        Ok(population) => population,
        Err(reason) => {
            return ExperimentResult::Inconclusive {
                reason: format!("baseline: {reason}"),
            };
        }
    };
    let candidate_population = match population(input.candidate_signals) {
        Ok(population) => population,
        Err(reason) => {
            return ExperimentResult::Inconclusive {
                reason: format!("candidate: {reason}"),
            };
        }
    };
    if target_changed(&input) {
        return ExperimentResult::Inconclusive {
            reason: "runnable-latency target changed between windows".to_owned(),
        };
    }
    if let Some(result) = reject_if_frame_pacing_regressed(&input) {
        return result;
    }
    decide_runnable(
        baseline_population,
        candidate_population,
        RunnableThresholds::default(),
    )
}

pub(super) fn target_changed(input: &ObjectiveComparisonInput<'_>) -> bool {
    input.baseline_signals.runnable_target_root_pid
        != input.candidate_signals.runnable_target_root_pid
        || input.baseline_signals.runnable_target_starttime_ticks
            != input.candidate_signals.runnable_target_starttime_ticks
}

pub(super) fn unavailable_reason(signals: &ObjectiveSignals) -> Option<String> {
    population(signals).err()
}

type RunnablePopulation = RunnableSummary;

fn population(signals: &ObjectiveSignals) -> Result<RunnablePopulation, String> {
    if signals.signal_quality.runnable_latency != ObjectiveSignalQuality::Direct
        || signals.runnable_latency_source.as_deref() != Some(RUNNABLE_HISTOGRAM_SOURCE)
        || signals.runnable_target_root_pid.is_none_or(|pid| pid == 0)
        || signals
            .runnable_target_starttime_ticks
            .is_none_or(|ticks| ticks == 0)
    {
        return Err(signals
            .runnable_latency_unavailable_reason
            .clone()
            .unwrap_or_else(|| {
                "direct runnable-latency signal unavailable for identified target".to_owned()
            }));
    }
    let (Some(p99_ns), Some(samples), Some(over_5ms), Some(max_ns)) = (
        signals.runnable_latency_p99_ns,
        signals.runnable_samples,
        signals.runnable_over_5ms,
        signals.runnable_latency_max_ns,
    ) else {
        return Err("direct runnable-latency population incomplete".to_owned());
    };
    if samples < MINIMUM_RUNNABLE_SAMPLES {
        return Err(format!(
            "insufficient runnable-latency samples: {samples} < {MINIMUM_RUNNABLE_SAMPLES}"
        ));
    }
    if over_5ms > samples || max_ns < p99_ns {
        return Err("inconsistent runnable-latency population or percentile bounds".to_owned());
    }
    Ok(RunnablePopulation {
        p99_ns,
        samples,
        over_5ms,
    })
}

pub(super) fn tail_regressed(input: &ObjectiveComparisonInput<'_>) -> bool {
    match (
        input.baseline_signals.runnable_over_5ms,
        input.baseline_signals.runnable_samples,
        input.candidate_signals.runnable_over_5ms,
        input.candidate_signals.runnable_samples,
    ) {
        (Some(base), Some(base_samples), Some(candidate), Some(candidate_samples))
            if base_samples > 0 && candidate_samples > 0 =>
        {
            u128::from(candidate) * u128::from(base_samples)
                > u128::from(base) * u128::from(candidate_samples)
        }
        _ => false,
    }
}

fn tail_regression_percent(baseline: RunnablePopulation, candidate: RunnablePopulation) -> f64 {
    let base = baseline.over_5ms as f64 / baseline.samples as f64;
    let candidate = candidate.over_5ms as f64 / candidate.samples as f64;
    if base == 0.0 {
        100.0
    } else {
        (candidate - base) / base * 100.0
    }
}

#[cfg(test)]
mod decision_tests {
    use super::*;

    fn summary(p99_ns: u64, over_5ms: u64) -> RunnableSummary {
        RunnableSummary {
            p99_ns,
            samples: 2_000,
            over_5ms,
        }
    }

    #[test]
    fn default_thresholds_are_the_product_contract() {
        let thresholds = RunnableThresholds::default();
        assert_eq!(thresholds.min_improvement_percent, 12.5);
        assert_eq!(thresholds.max_regression_percent, 7.5);
        assert_eq!(thresholds.min_samples, 1_000);
    }

    #[test]
    fn improvement_regression_and_neutral_are_decided_by_p99_change() {
        let thresholds = RunnableThresholds::default();
        assert!(matches!(
            decide_runnable(summary(10_000_000, 20), summary(8_000_000, 10), thresholds),
            ExperimentResult::Improved { .. }
        ));
        assert!(matches!(
            decide_runnable(summary(10_000_000, 20), summary(10_800_000, 20), thresholds),
            ExperimentResult::Regressed { .. }
        ));
        assert!(matches!(
            decide_runnable(summary(10_000_000, 20), summary(9_500_000, 20), thresholds),
            ExperimentResult::Inconclusive { .. }
        ));
    }

    #[test]
    fn a_worse_over_5ms_tail_regresses_even_when_p99_improved() {
        assert!(matches!(
            decide_runnable(
                summary(10_000_000, 20),
                summary(5_000_000, 40),
                RunnableThresholds::default()
            ),
            ExperimentResult::Regressed { .. }
        ));
    }

    #[test]
    fn too_few_samples_are_inconclusive() {
        let mut candidate = summary(1_000_000, 0);
        candidate.samples = 999;
        assert!(matches!(
            decide_runnable(
                summary(10_000_000, 20),
                candidate,
                RunnableThresholds::default()
            ),
            ExperimentResult::Inconclusive { .. }
        ));
    }

    #[test]
    fn a_stricter_threshold_set_keeps_fewer_changes() {
        let strict = RunnableThresholds {
            min_improvement_percent: 25.0,
            ..RunnableThresholds::default()
        };
        let baseline = summary(10_000_000, 20);
        let modest = summary(8_000_000, 10);
        assert!(matches!(
            decide_runnable(baseline, modest, RunnableThresholds::default()),
            ExperimentResult::Improved { .. }
        ));
        assert!(matches!(
            decide_runnable(baseline, modest, strict),
            ExperimentResult::Inconclusive { .. }
        ));
    }
}
