use super::{ObjectiveComparisonInput, ObjectiveSignals};
use crate::autotune::{
    comparison::{ExperimentResult, regression_percent_f64},
    experiment::WindowScore,
};

pub(super) fn reject_invalid_or_low_quality(
    input: &ObjectiveComparisonInput<'_>,
    generic_score_result: &ExperimentResult,
) -> Option<ExperimentResult> {
    if matches!(generic_score_result, ExperimentResult::Invalid { .. })
        || input.target_disappeared
        || input.data_quality.is_low()
    {
        return Some(generic_score_result.clone());
    }

    None
}

pub(super) fn missing_objective_signals(
    input: &ObjectiveComparisonInput<'_>,
) -> Option<ExperimentResult> {
    let mut missing = input
        .baseline_signals
        .missing_required_signals_for(input.objective)
        .into_iter()
        .map(|signal| format!("baseline.{signal}"))
        .collect::<Vec<_>>();
    missing.extend(
        input
            .candidate_signals
            .missing_required_signals_for(input.objective)
            .into_iter()
            .map(|signal| format!("candidate.{signal}")),
    );

    (!missing.is_empty()).then(|| ExperimentResult::Inconclusive {
        reason: format!(
            "missing required objective signal(s) for {:?}: {}",
            input.objective,
            missing.join(", ")
        ),
    })
}

pub(super) fn reject_if_power_or_thermal_regressed(
    input: &ObjectiveComparisonInput<'_>,
) -> Option<ExperimentResult> {
    if let (Some(false), Some(true)) = (
        input.baseline_signals.thermal_degraded,
        input.candidate_signals.thermal_degraded,
    ) {
        return Some(objective_regressed(input));
    }

    if let (Some(baseline), Some(candidate)) = (
        input.baseline_signals.thermal_throttle_count,
        input.candidate_signals.thermal_throttle_count,
    ) && candidate > baseline
    {
        return Some(objective_regressed(input));
    }

    if let (Some(false), Some(true)) = (
        input.baseline_signals.cpu_power_limited,
        input.candidate_signals.cpu_power_limited,
    ) {
        return Some(objective_regressed(input));
    }

    if let (Some(false), Some(true)) = (
        input.baseline_signals.gpu_power_limited,
        input.candidate_signals.gpu_power_limited,
    ) {
        return Some(objective_regressed(input));
    }

    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PrimaryMetricComparison {
    Improved,
    Regressed,
    Unchanged,
    Missing,
}

pub(super) fn frame_p99_comparison(
    input: &ObjectiveComparisonInput<'_>,
    regression_slack_ms: f64,
) -> PrimaryMetricComparison {
    if input
        .baseline_signals
        .signal_quality
        .frame_pacing
        .is_missing()
        || input
            .candidate_signals
            .signal_quality
            .frame_pacing
            .is_missing()
    {
        return PrimaryMetricComparison::Missing;
    }

    match (
        input.baseline_signals.frame_p99_ms,
        input.candidate_signals.frame_p99_ms,
    ) {
        (Some(baseline), Some(candidate)) if candidate > baseline + regression_slack_ms => {
            PrimaryMetricComparison::Regressed
        }
        (Some(baseline), Some(candidate)) if candidate < baseline => {
            PrimaryMetricComparison::Improved
        }
        (Some(_), Some(_)) => PrimaryMetricComparison::Unchanged,
        _ => PrimaryMetricComparison::Missing,
    }
}

pub(super) fn compare_interactivity_objective(
    input: ObjectiveComparisonInput<'_>,
    generic_score_result: ExperimentResult,
) -> ExperimentResult {
    match foreground_over_5ms_comparison(&input) {
        PrimaryMetricComparison::Improved => improved_with_generic_guard(generic_score_result),
        PrimaryMetricComparison::Regressed => objective_regressed(&input),
        PrimaryMetricComparison::Unchanged | PrimaryMetricComparison::Missing => {
            generic_score_result
        }
    }
}

pub(super) fn foreground_over_5ms_comparison(
    input: &ObjectiveComparisonInput<'_>,
) -> PrimaryMetricComparison {
    match (
        foreground_over_5ms_per_sample(input.baseline_signals, input.baseline),
        foreground_over_5ms_per_sample(input.candidate_signals, input.candidate),
    ) {
        (Some(baseline), Some(candidate)) if candidate > baseline => {
            PrimaryMetricComparison::Regressed
        }
        (Some(baseline), Some(candidate)) if candidate < baseline => {
            PrimaryMetricComparison::Improved
        }
        (Some(_), Some(_)) => PrimaryMetricComparison::Unchanged,
        _ => PrimaryMetricComparison::Missing,
    }
}

fn foreground_over_5ms_per_sample(signals: &ObjectiveSignals, score: &WindowScore) -> Option<f64> {
    if signals.signal_quality.foreground_latency.is_missing() {
        return None;
    }

    let over_5ms = signals.foreground_over_5ms?;
    let scored_samples = score.scored_samples;
    (scored_samples > 0).then(|| over_5ms as f64 / scored_samples as f64)
}

pub(super) fn reject_if_frame_pacing_regressed(
    input: &ObjectiveComparisonInput<'_>,
) -> Option<ExperimentResult> {
    if frame_p99_comparison(input, 1.0) == PrimaryMetricComparison::Regressed {
        return Some(objective_regressed(input));
    }

    reject_if_foreground_regressed(input)
}

pub(super) fn reject_if_foreground_regressed(
    input: &ObjectiveComparisonInput<'_>,
) -> Option<ExperimentResult> {
    if input.candidate.score.frame_p99_ms > input.baseline.score.frame_p99_ms + 2.0
        || foreground_over_5ms_comparison(input) == PrimaryMetricComparison::Regressed
    {
        return Some(objective_regressed(input));
    }

    None
}

pub(super) fn improved_with_generic_guard(
    generic_score_result: ExperimentResult,
) -> ExperimentResult {
    match generic_score_result {
        ExperimentResult::Regressed { .. } | ExperimentResult::Invalid { .. } => {
            generic_score_result
        }
        ExperimentResult::Improved {
            improvement_percent,
        } => ExperimentResult::Improved {
            improvement_percent,
        },
        ExperimentResult::Inconclusive { .. } => ExperimentResult::Improved {
            improvement_percent: 0.0,
        },
    }
}

pub(super) fn objective_regressed(input: &ObjectiveComparisonInput<'_>) -> ExperimentResult {
    ExperimentResult::Regressed {
        regression_percent: objective_regression_percent(input.baseline, input.candidate),
    }
}

pub(super) fn objective_regression_percent(baseline: &WindowScore, candidate: &WindowScore) -> f64 {
    match (baseline.score_per_sample(), candidate.score_per_sample()) {
        (Some(baseline), Some(candidate)) => regression_percent_f64(baseline, candidate),
        _ => 0.0,
    }
}
