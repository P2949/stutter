use super::{
    ObjectiveComparisonInput,
    common::{
        PrimaryMetricComparison, objective_regression_percent, reject_if_foreground_regressed,
    },
};
use crate::autotune::comparison::ExperimentResult;

pub(super) fn compare(
    input: ObjectiveComparisonInput<'_>,
    generic_score_result: ExperimentResult,
) -> ExperimentResult {
    if let Some(result) = reject_if_foreground_regressed(&input) {
        return result;
    }

    match compile_throughput_comparison(&input) {
        PrimaryMetricComparison::Improved => direct_compile_throughput_improved(&input),
        PrimaryMetricComparison::Regressed => direct_compile_throughput_regressed(&input),
        PrimaryMetricComparison::Unchanged => ExperimentResult::Inconclusive {
            reason: "compile throughput primary signal did not improve".to_owned(),
        },
        PrimaryMetricComparison::Missing => compile_throughput_fallback(generic_score_result),
    }
}

fn compile_throughput_comparison(input: &ObjectiveComparisonInput<'_>) -> PrimaryMetricComparison {
    match (
        input
            .baseline_signals
            .compile_throughput_signal()
            .direct_progress(),
        input
            .candidate_signals
            .compile_throughput_signal()
            .direct_progress(),
    ) {
        (
            Some((baseline_intervals, baseline_samples)),
            Some((candidate_intervals, candidate_samples)),
        ) if candidate_intervals > baseline_intervals
            || (candidate_intervals == baseline_intervals
                && candidate_samples > baseline_samples) =>
        {
            PrimaryMetricComparison::Improved
        }
        (
            Some((baseline_intervals, baseline_samples)),
            Some((candidate_intervals, candidate_samples)),
        ) if candidate_intervals < baseline_intervals
            || (candidate_intervals == baseline_intervals
                && candidate_samples < baseline_samples) =>
        {
            PrimaryMetricComparison::Regressed
        }
        (Some(_), Some(_)) => PrimaryMetricComparison::Unchanged,
        _ => PrimaryMetricComparison::Missing,
    }
}

fn compile_throughput_fallback(generic_score_result: ExperimentResult) -> ExperimentResult {
    match generic_score_result {
        ExperimentResult::Inconclusive { reason } => ExperimentResult::Inconclusive {
            reason: format!(
                "compile throughput direct signal missing; using lower-confidence stutter-score proxy: {reason}"
            ),
        },
        result => result,
    }
}

fn direct_compile_throughput_improved(input: &ObjectiveComparisonInput<'_>) -> ExperimentResult {
    let interval_improvement = match (
        input.baseline_signals.compile_progress_intervals,
        input.candidate_signals.compile_progress_intervals,
    ) {
        (Some(0), Some(candidate)) if candidate > 0 => 100.0,
        (Some(baseline), Some(candidate)) if candidate > baseline => {
            ((candidate - baseline) as f64 / baseline as f64) * 100.0
        }
        _ => 0.0,
    };
    let sample_improvement = match (
        input.baseline_signals.compile_progress_samples,
        input.candidate_signals.compile_progress_samples,
    ) {
        (Some(0), Some(candidate)) if candidate > 0 => 100.0,
        (Some(baseline), Some(candidate)) if candidate > baseline => {
            ((candidate - baseline) as f64 / baseline as f64) * 100.0
        }
        _ => 0.0,
    };

    ExperimentResult::Improved {
        improvement_percent: interval_improvement.max(sample_improvement),
    }
}

fn direct_compile_throughput_regressed(input: &ObjectiveComparisonInput<'_>) -> ExperimentResult {
    let interval_regression = match (
        input.baseline_signals.compile_progress_intervals,
        input.candidate_signals.compile_progress_intervals,
    ) {
        (Some(baseline), Some(0)) if baseline > 0 => 100.0,
        (Some(baseline), Some(candidate)) if baseline > candidate => {
            ((baseline - candidate) as f64 / baseline as f64) * 100.0
        }
        _ => 0.0,
    };
    let sample_regression = match (
        input.baseline_signals.compile_progress_samples,
        input.candidate_signals.compile_progress_samples,
    ) {
        (Some(baseline), Some(0)) if baseline > 0 => 100.0,
        (Some(baseline), Some(candidate)) if baseline > candidate => {
            ((baseline - candidate) as f64 / baseline as f64) * 100.0
        }
        _ => 0.0,
    };
    let regression_percent =
        interval_regression
            .max(sample_regression)
            .max(objective_regression_percent(
                input.baseline,
                input.candidate,
            ));

    ExperimentResult::Regressed { regression_percent }
}
