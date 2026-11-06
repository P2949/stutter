use super::{
    ObjectiveComparisonInput, ObjectiveSignalQuality, ObjectiveSignals,
    common::reject_if_foreground_regressed,
};
use crate::{
    autotune::comparison::ExperimentResult,
    workload_progress::{FixedWorkMeasurement, WorkloadKind},
};

// Provisional conservative effect thresholds, pending repeated real-workload
// calibration. These are not a statistical confidence interval.
pub const MINIMUM_CHANGE_PERCENT: f64 = 10.0;

// A calibrated value is only ever frozen from the pre-registered grid (7.5 to 20 percent).
const _: () = assert!(MINIMUM_CHANGE_PERCENT >= 7.5 && MINIMUM_CHANGE_PERCENT <= 20.0);

/// The elapsed-time verdict between two identical completed builds. The controller applies it with
/// [`MINIMUM_CHANGE_PERCENT`]; the calibration campaign applies it at each candidate threshold, so
/// both use exactly one rule. Lower elapsed time is better.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CompileChange {
    Improved(f64),
    Regressed(f64),
    Inconclusive(f64),
}

pub fn classify_change(
    baseline_elapsed_ns: u64,
    candidate_elapsed_ns: u64,
    threshold_percent: f64,
) -> CompileChange {
    let baseline = baseline_elapsed_ns as f64;
    let change_percent = (candidate_elapsed_ns as f64 - baseline) / baseline * 100.0;
    if change_percent >= threshold_percent {
        CompileChange::Regressed(change_percent)
    } else if -change_percent >= threshold_percent {
        CompileChange::Improved(-change_percent)
    } else {
        CompileChange::Inconclusive(change_percent)
    }
}

pub(super) fn measurement(signals: &ObjectiveSignals) -> Result<&FixedWorkMeasurement, String> {
    let work = signals.compile_fixed_work.as_ref().ok_or_else(|| {
        format!(
            "direct compile throughput unavailable; {}",
            signals
                .compile_fixed_work_unavailable_reason
                .as_deref()
                .unwrap_or("compile activity is not completed work")
        )
    })?;
    work.validate()?;
    if signals.signal_quality.compile_throughput != ObjectiveSignalQuality::Direct
        || work.start.identity.kind != WorkloadKind::Compile
    {
        return Err("direct compile throughput unavailable for completed compile work".to_owned());
    }
    Ok(work)
}

pub(super) fn compare(
    input: ObjectiveComparisonInput<'_>,
    _generic_score_result: ExperimentResult,
) -> ExperimentResult {
    if let Some(result) = reject_if_foreground_regressed(&input) {
        return result;
    }

    let (baseline, candidate) = match (
        measurement(input.baseline_signals),
        measurement(input.candidate_signals),
    ) {
        (Ok(baseline), Ok(candidate)) => (baseline, candidate),
        (Err(reason), _) => {
            return ExperimentResult::Inconclusive {
                reason: format!("baseline: {reason}"),
            };
        }
        (_, Err(reason)) => {
            return ExperimentResult::Inconclusive {
                reason: format!("candidate: {reason}"),
            };
        }
    };
    if baseline.start.identity != candidate.start.identity
        || baseline.start.total_units != candidate.start.total_units
        || baseline.start.source != candidate.start.source
        || baseline.start.target_root_pid != candidate.start.target_root_pid
        || baseline.start.target_starttime_ticks != candidate.start.target_starttime_ticks
    {
        return ExperimentResult::Inconclusive {
            reason:
                "fixed-work compile workload identities, target identities, units or sources differ"
                    .to_owned(),
        };
    }
    if baseline.start.execution_id == candidate.start.execution_id
        || candidate.start.monotonic_ns < baseline.complete.monotonic_ns
    {
        return ExperimentResult::Inconclusive { reason: "fixed-work compile comparisons require distinct, non-overlapping executions in baseline/candidate order".to_owned() };
    }
    if [input.baseline_signals, input.candidate_signals]
        .iter()
        .any(|signals| {
            signals.signal_quality.foreground_latency.is_missing()
                || signals.foreground_over_5ms.is_none()
        })
    {
        return ExperimentResult::Inconclusive {
            reason: "foreground protection signal unavailable for fixed-work compile comparison"
                .to_owned(),
        };
    }
    if [input.baseline_signals, input.candidate_signals]
        .iter()
        .any(|signals| {
            signals
                .memory_pressure_some_avg10_percent
                .is_some_and(|value| !value.is_finite() || !(0.0..=100.0).contains(&value))
        })
    {
        return ExperimentResult::Inconclusive {
            reason: "invalid memory pressure guardrail evidence".to_owned(),
        };
    }
    if memory_pressure_regressed(&input) {
        return ExperimentResult::Regressed {
            regression_percent: 0.0,
        };
    }
    match classify_change(
        baseline.complete.elapsed_ns,
        candidate.complete.elapsed_ns,
        MINIMUM_CHANGE_PERCENT,
    ) {
        CompileChange::Regressed(regression_percent) => {
            ExperimentResult::Regressed { regression_percent }
        }
        CompileChange::Improved(improvement_percent) => ExperimentResult::Improved {
            improvement_percent,
        },
        CompileChange::Inconclusive(change_percent) => ExperimentResult::Inconclusive {
            reason: format!(
                "fixed-work completion time change {change_percent:.2}% did not meet conservative {MINIMUM_CHANGE_PERCENT}% threshold"
            ),
        },
    }
}

pub(super) fn memory_pressure_regressed(input: &ObjectiveComparisonInput<'_>) -> bool {
    match (
        input.baseline_signals.memory_pressure_some_avg10_percent,
        input.candidate_signals.memory_pressure_some_avg10_percent,
    ) {
        (Some(baseline), Some(candidate)) => candidate > baseline,
        _ => false,
    }
}

#[cfg(test)]
mod rule_tests {
    use super::*;

    #[test]
    fn the_change_rule_is_symmetric_and_inclusive_at_the_threshold() {
        let second = 1_000_000_000_u64;
        // Exactly at the threshold counts, in both directions.
        assert_eq!(
            classify_change(100 * second, 90 * second, 10.0),
            CompileChange::Improved(10.0)
        );
        assert_eq!(
            classify_change(100 * second, 110 * second, 10.0),
            CompileChange::Regressed(10.0)
        );
        // Just inside it is inconclusive, never a keep.
        assert!(matches!(
            classify_change(100 * second, 91 * second, 10.0),
            CompileChange::Inconclusive(_)
        ));
        // A larger threshold turns the same gain into an inconclusive result.
        assert!(matches!(
            classify_change(100 * second, 85 * second, 20.0),
            CompileChange::Inconclusive(_)
        ));
    }
}
