use super::{ObjectiveComparisonInput, ObjectiveSignalQuality, ObjectiveSignals, common::reject_if_foreground_regressed};
use crate::autotune::comparison::ExperimentResult;
use crate::workload_progress::{FixedWorkMeasurement, WorkloadKind};

// Provisional conservative effect thresholds, pending repeated real-workload
// calibration. These are not a statistical confidence interval.
const MINIMUM_CHANGE_PERCENT: f64 = 10.0;

pub(super) fn measurement(signals: &ObjectiveSignals) -> Result<&FixedWorkMeasurement, String> {
    let work = signals.compile_fixed_work.as_ref().ok_or_else(|| format!("direct compile throughput unavailable; {}", signals.compile_fixed_work_unavailable_reason.as_deref().unwrap_or("compile activity is not completed work")))?;
    work.validate()?;
    if signals.signal_quality.compile_throughput != ObjectiveSignalQuality::Direct || work.start.identity.kind != WorkloadKind::Compile {
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

    let (baseline, candidate) = match (measurement(input.baseline_signals), measurement(input.candidate_signals)) {
        (Ok(baseline), Ok(candidate)) => (baseline, candidate),
        (Err(reason), _) => return ExperimentResult::Inconclusive { reason: format!("baseline: {reason}") },
        (_, Err(reason)) => return ExperimentResult::Inconclusive { reason: format!("candidate: {reason}") },
    };
    if baseline.start.identity != candidate.start.identity || baseline.start.total_units != candidate.start.total_units
        || baseline.start.source != candidate.start.source {
        return ExperimentResult::Inconclusive { reason: "fixed-work compile workload identities, units or sources differ".to_owned() };
    }
    if baseline.start.execution_id == candidate.start.execution_id || candidate.start.monotonic_ns < baseline.complete.monotonic_ns {
        return ExperimentResult::Inconclusive { reason: "fixed-work compile comparisons require distinct, non-overlapping executions in baseline/candidate order".to_owned() };
    }
    if [input.baseline_signals, input.candidate_signals].iter().any(|signals| signals.signal_quality.foreground_latency.is_missing() || signals.foreground_over_5ms.is_none()) {
        return ExperimentResult::Inconclusive { reason: "foreground protection signal unavailable for fixed-work compile comparison".to_owned() };
    }
    if memory_pressure_regressed(&input) {
        return ExperimentResult::Regressed { regression_percent: 0.0 };
    }
    let baseline_ns = baseline.complete.elapsed_ns as f64;
    let change_percent = (candidate.complete.elapsed_ns as f64 - baseline_ns) / baseline_ns * 100.0;
    if change_percent >= MINIMUM_CHANGE_PERCENT {
        ExperimentResult::Regressed { regression_percent: change_percent }
    } else if -change_percent >= MINIMUM_CHANGE_PERCENT {
        ExperimentResult::Improved { improvement_percent: -change_percent }
    } else {
        ExperimentResult::Inconclusive { reason: format!("fixed-work completion time change {change_percent:.2}% did not meet conservative {MINIMUM_CHANGE_PERCENT}% threshold") }
    }
}

pub(super) fn memory_pressure_regressed(input: &ObjectiveComparisonInput<'_>) -> bool {
    match (input.baseline_signals.memory_pressure_some_avg10_percent, input.candidate_signals.memory_pressure_some_avg10_percent) {
        (Some(baseline), Some(candidate)) => candidate > baseline,
        _ => false,
    }
}
