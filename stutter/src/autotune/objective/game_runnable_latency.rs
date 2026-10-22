use super::{
    ObjectiveComparisonInput, ObjectiveSignalQuality, ObjectiveSignals,
    common::reject_if_frame_pacing_regressed, runnable_signal::RUNNABLE_HISTOGRAM_SOURCE,
};
use crate::autotune::comparison::ExperimentResult;

/// Provisional conservative thresholds, pending real-system calibration. A p99
/// needs at least 1,000 events (roughly ten observations in its upper 1%).
pub const MINIMUM_RUNNABLE_SAMPLES: u64 = 1_000;
const MIN_IMPROVEMENT_PERCENT: f64 = 12.5;
const MAX_REGRESSION_PERCENT: f64 = 7.5;

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
    let baseline = baseline_population.p99_ns;
    let candidate = candidate_population.p99_ns;
    if tail_regressed(&input) {
        return ExperimentResult::Regressed {
            regression_percent: tail_regression_percent(baseline_population, candidate_population),
        };
    }
    let change_percent = if baseline == 0 {
        if candidate > 0 { 100.0 } else { 0.0 }
    } else {
        (candidate as f64 - baseline as f64) / baseline as f64 * 100.0
    };
    if change_percent >= MAX_REGRESSION_PERCENT {
        ExperimentResult::Regressed {
            regression_percent: change_percent,
        }
    } else if -change_percent >= MIN_IMPROVEMENT_PERCENT {
        ExperimentResult::Improved {
            improvement_percent: -change_percent,
        }
    } else {
        ExperimentResult::Inconclusive {
            reason: format!(
                "runnable p99 change {change_percent:.2}% did not meet conservative thresholds (improvement {MIN_IMPROVEMENT_PERCENT}%, regression {MAX_REGRESSION_PERCENT}%); no score fallback"
            ),
        }
    }
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

#[derive(Clone, Copy)]
struct RunnablePopulation {
    p99_ns: u64,
    samples: u64,
    over_5ms: u64,
}

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
