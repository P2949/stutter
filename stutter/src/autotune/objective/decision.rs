use serde::{Deserialize, Serialize};

use super::{
    ObjectiveComparisonInput, ObjectiveKind, ObjectiveSignalQuality, common,
    common::PrimaryMetricComparison,
};
use crate::{
    autotune::comparison::{ExperimentDataQuality, ExperimentResult},
    diagnosis::Confidence,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ObjectiveDecision {
    pub outcome: ObjectiveOutcome,
    pub primary_signal: ObjectiveSignal,
    pub guard_failures: Vec<ObjectiveGuardFailure>,
    pub confidence: Confidence,
}

impl ObjectiveDecision {
    pub(super) fn from_comparison(
        input: &ObjectiveComparisonInput<'_>,
        generic_score_result: &ExperimentResult,
        result: ExperimentResult,
    ) -> Self {
        let outcome = ObjectiveOutcome::from_experiment_result(result);
        let primary_signal = primary_signal_for(input);
        let guard_failures = guard_failures_for(input, generic_score_result);
        let confidence = confidence_for(&outcome, &primary_signal, &guard_failures);

        Self {
            outcome,
            primary_signal,
            guard_failures,
            confidence,
        }
    }

    pub fn experiment_result(&self) -> ExperimentResult {
        self.outcome.experiment_result()
    }

    pub fn summary(&self) -> String {
        let mut parts = vec![
            format!("outcome={}", self.outcome.as_str()),
            format!("primary_signal={}", self.primary_signal.name),
            format!("confidence={:?}", self.confidence),
            self.primary_signal.summary.clone(),
        ];

        if let Some(reason) = self.primary_signal.degraded_reason.as_ref() {
            parts.push(format!("degraded_reason={reason}"));
        }

        if !self.guard_failures.is_empty() {
            parts.push(format!(
                "guard_failures={}",
                self.guard_failures
                    .iter()
                    .map(|failure| format!("{}: {}", failure.guard, failure.reason))
                    .collect::<Vec<_>>()
                    .join("; ")
            ));
        }

        parts.join(" ")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveOutcome {
    Improved { improvement_percent: f64 },
    Regressed { regression_percent: f64 },
    Inconclusive { reason: String },
    Invalid { reason: String },
}

impl ObjectiveOutcome {
    fn from_experiment_result(result: ExperimentResult) -> Self {
        match result {
            ExperimentResult::Improved {
                improvement_percent,
            } => Self::Improved {
                improvement_percent,
            },
            ExperimentResult::Regressed { regression_percent } => {
                Self::Regressed { regression_percent }
            }
            ExperimentResult::Inconclusive { reason } => Self::Inconclusive { reason },
            ExperimentResult::Invalid { reason } => Self::Invalid { reason },
        }
    }

    fn experiment_result(&self) -> ExperimentResult {
        match self {
            Self::Improved {
                improvement_percent,
            } => ExperimentResult::Improved {
                improvement_percent: *improvement_percent,
            },
            Self::Regressed { regression_percent } => ExperimentResult::Regressed {
                regression_percent: *regression_percent,
            },
            Self::Inconclusive { reason } => ExperimentResult::Inconclusive {
                reason: reason.clone(),
            },
            Self::Invalid { reason } => ExperimentResult::Invalid {
                reason: reason.clone(),
            },
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            Self::Improved { .. } => "improved",
            Self::Regressed { .. } => "regressed",
            Self::Inconclusive { .. } => "inconclusive",
            Self::Invalid { .. } => "invalid",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObjectiveSignal {
    pub name: String,
    pub quality: ObjectiveSignalQuality,
    pub baseline: Option<String>,
    pub candidate: Option<String>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObjectiveGuardFailure {
    pub guard: String,
    pub reason: String,
}

fn primary_signal_for(input: &ObjectiveComparisonInput<'_>) -> ObjectiveSignal {
    match input.objective {
        ObjectiveKind::StutterScore => ObjectiveSignal {
            name: "stutter_score".to_owned(),
            quality: ObjectiveSignalQuality::Derived,
            baseline: input.baseline.score_per_sample().map(format_f64),
            candidate: input.candidate.score_per_sample().map(format_f64),
            summary: format!(
                "baseline_score_per_sample={} candidate_score_per_sample={}",
                format_optional_f64(input.baseline.score_per_sample()),
                format_optional_f64(input.candidate.score_per_sample())
            ),
            degraded_reason: None,
        },
        ObjectiveKind::GameFramePacing => ObjectiveSignal {
            name: "frame_p99_ms".to_owned(),
            quality: pair_quality(
                input.baseline_signals.signal_quality.frame_pacing,
                input.candidate_signals.signal_quality.frame_pacing,
            ),
            baseline: input.baseline_signals.frame_p99_ms.map(format_f64),
            candidate: input.candidate_signals.frame_p99_ms.map(format_f64),
            summary: format!(
                "baseline_frame_p99_ms={} candidate_frame_p99_ms={}",
                format_optional_f64(input.baseline_signals.frame_p99_ms),
                format_optional_f64(input.candidate_signals.frame_p99_ms)
            ),
            degraded_reason: None,
        },
        ObjectiveKind::GameRunnableLatency => ObjectiveSignal {
            name: "runnable_latency_with_frame_guard".to_owned(),
            quality: pair_quality(
                input.baseline_signals.signal_quality.foreground_latency,
                input.candidate_signals.signal_quality.foreground_latency,
            ),
            baseline: input.baseline_signals.foreground_over_5ms.map(|value| {
                format!(
                    "{} over_5ms events / {} samples",
                    value, input.baseline.scored_samples
                )
            }),
            candidate: input.candidate_signals.foreground_over_5ms.map(|value| {
                format!(
                    "{} over_5ms events / {} samples",
                    value, input.candidate.scored_samples
                )
            }),
            summary: "foreground runnable latency is primary, frame pacing is guarded".to_owned(),
            degraded_reason: None,
        },
        ObjectiveKind::DesktopInteractivity => interactivity_signal(input, "desktop_interactivity"),
        ObjectiveKind::BrowserInteractivity => interactivity_signal(input, "browser_interactivity"),
        ObjectiveKind::CompileThroughputWithForegroundProtection => compile_signal(input),
        ObjectiveKind::IoLatency => ObjectiveSignal {
            name: "block_io_overlap".to_owned(),
            quality: pair_quality(
                input.baseline_signals.signal_quality.block_io_overlap,
                input.candidate_signals.signal_quality.block_io_overlap,
            ),
            baseline: Some(format!(
                "count={} worst_latency_ns={}",
                format_optional_u64(input.baseline_signals.block_io_overlap_count),
                format_optional_u64(input.baseline_signals.block_io_worst_latency_ns)
            )),
            candidate: Some(format!(
                "count={} worst_latency_ns={}",
                format_optional_u64(input.candidate_signals.block_io_overlap_count),
                format_optional_u64(input.candidate_signals.block_io_worst_latency_ns)
            )),
            summary: "block I/O overlap count and worst latency".to_owned(),
            degraded_reason: degraded_overlap_reason(
                input.baseline_signals.block_io_overlap_trust.as_deref(),
                input.candidate_signals.block_io_overlap_trust.as_deref(),
            ),
        },
        ObjectiveKind::IrqOverlapReduction => ObjectiveSignal {
            name: "irq_overlap".to_owned(),
            quality: pair_quality(
                input.baseline_signals.signal_quality.irq_overlap,
                input.candidate_signals.signal_quality.irq_overlap,
            ),
            baseline: Some(format!(
                "count={} worst_overlap_ns={}",
                format_optional_u64(input.baseline_signals.irq_overlap_count),
                format_optional_u64(input.baseline_signals.irq_worst_overlap_ns)
            )),
            candidate: Some(format!(
                "count={} worst_overlap_ns={}",
                format_optional_u64(input.candidate_signals.irq_overlap_count),
                format_optional_u64(input.candidate_signals.irq_worst_overlap_ns)
            )),
            summary: "IRQ overlap count and worst overlap duration".to_owned(),
            degraded_reason: None,
        },
        ObjectiveKind::ThermalRecovery => ObjectiveSignal {
            name: "thermal_recovery".to_owned(),
            quality: pair_quality(
                input.baseline_signals.signal_quality.thermal,
                input.candidate_signals.signal_quality.thermal,
            ),
            baseline: Some(format!(
                "degraded={} throttle_count={}",
                format_optional_bool(input.baseline_signals.thermal_degraded),
                format_optional_u64(input.baseline_signals.thermal_throttle_count)
            )),
            candidate: Some(format!(
                "degraded={} throttle_count={}",
                format_optional_bool(input.candidate_signals.thermal_degraded),
                format_optional_u64(input.candidate_signals.thermal_throttle_count)
            )),
            summary: "thermal degraded state and throttle count".to_owned(),
            degraded_reason: None,
        },
    }
}

fn interactivity_signal(input: &ObjectiveComparisonInput<'_>, name: &str) -> ObjectiveSignal {
    ObjectiveSignal {
        name: name.to_owned(),
        quality: pair_quality(
            input.baseline_signals.signal_quality.foreground_latency,
            input.candidate_signals.signal_quality.foreground_latency,
        ),
        baseline: input.baseline_signals.foreground_over_5ms.map(|value| {
            format!(
                "{} over_5ms events / {} samples",
                value, input.baseline.scored_samples
            )
        }),
        candidate: input.candidate_signals.foreground_over_5ms.map(|value| {
            format!(
                "{} over_5ms events / {} samples",
                value, input.candidate.scored_samples
            )
        }),
        summary: "foreground over-5ms events per scored sample".to_owned(),
        degraded_reason: None,
    }
}

fn compile_signal(input: &ObjectiveComparisonInput<'_>) -> ObjectiveSignal {
    let baseline = input.baseline_signals.compile_throughput_signal();
    let candidate = input.candidate_signals.compile_throughput_signal();
    let quality = pair_quality(baseline.quality, candidate.quality);
    let degraded_reason = baseline
        .degraded_reason
        .clone()
        .or_else(|| candidate.degraded_reason.clone());

    ObjectiveSignal {
        name: "compile_throughput".to_owned(),
        quality,
        baseline: Some(format!(
            "progress_intervals={} progress_samples={}",
            format_optional_u64(baseline.progress_intervals),
            format_optional_u64(baseline.progress_samples)
        )),
        candidate: Some(format!(
            "progress_intervals={} progress_samples={}",
            format_optional_u64(candidate.progress_intervals),
            format_optional_u64(candidate.progress_samples)
        )),
        summary: "compile progress intervals and samples".to_owned(),
        degraded_reason,
    }
}

fn guard_failures_for(
    input: &ObjectiveComparisonInput<'_>,
    generic_score_result: &ExperimentResult,
) -> Vec<ObjectiveGuardFailure> {
    let mut failures = Vec::new();

    match generic_score_result {
        ExperimentResult::Invalid { reason } => failures.push(guard_failure(
            "score_validation",
            format!("score comparison invalid: {reason}"),
        )),
        ExperimentResult::Regressed { .. } if input.target_disappeared => failures.push(
            guard_failure("target_presence", "target disappeared during comparison"),
        ),
        _ => {}
    }

    if input.data_quality == ExperimentDataQuality::Low {
        failures.push(guard_failure(
            "data_quality",
            "comparison data quality was low",
        ));
    }

    let missing = missing_required_signal_names(input);
    if !missing.is_empty() {
        failures.push(guard_failure(
            "required_signal",
            format!(
                "missing required objective signal(s): {}",
                missing.join(", ")
            ),
        ));
    }

    failures.extend(power_or_thermal_guard_failures(input));
    failures.extend(objective_specific_guard_failures(input));

    failures
}

fn missing_required_signal_names(input: &ObjectiveComparisonInput<'_>) -> Vec<String> {
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
    missing
}

fn power_or_thermal_guard_failures(
    input: &ObjectiveComparisonInput<'_>,
) -> Vec<ObjectiveGuardFailure> {
    let mut failures = Vec::new();

    if let (Some(false), Some(true)) = (
        input.baseline_signals.thermal_degraded,
        input.candidate_signals.thermal_degraded,
    ) {
        failures.push(guard_failure(
            "thermal",
            "candidate introduced thermal degradation",
        ));
    }

    if let (Some(baseline), Some(candidate)) = (
        input.baseline_signals.thermal_throttle_count,
        input.candidate_signals.thermal_throttle_count,
    ) && candidate > baseline
    {
        failures.push(guard_failure(
            "thermal",
            format!("candidate throttle count {candidate} exceeded baseline {baseline}"),
        ));
    }

    if let (Some(false), Some(true)) = (
        input.baseline_signals.cpu_power_limited,
        input.candidate_signals.cpu_power_limited,
    ) {
        failures.push(guard_failure(
            "cpu_power",
            "candidate introduced CPU power limiting",
        ));
    }

    if let (Some(false), Some(true)) = (
        input.baseline_signals.gpu_power_limited,
        input.candidate_signals.gpu_power_limited,
    ) {
        failures.push(guard_failure(
            "gpu_power",
            "candidate introduced GPU power limiting",
        ));
    }

    failures
}

fn objective_specific_guard_failures(
    input: &ObjectiveComparisonInput<'_>,
) -> Vec<ObjectiveGuardFailure> {
    let mut failures = Vec::new();

    match input.objective {
        ObjectiveKind::GameFramePacing
        | ObjectiveKind::CompileThroughputWithForegroundProtection => {
            failures.extend(foreground_guard_failures(input, 2.0));
        }
        ObjectiveKind::GameRunnableLatency => {
            if common::frame_p99_comparison(input, 1.0) == PrimaryMetricComparison::Regressed {
                failures.push(guard_failure(
                    "frame_pacing",
                    "candidate regressed frame p99 beyond runnable-latency guard",
                ));
            }
            failures.extend(foreground_guard_failures(input, 2.0));
        }
        _ => {}
    }

    failures
}

fn foreground_guard_failures(
    input: &ObjectiveComparisonInput<'_>,
    frame_p99_slack_ms: f64,
) -> Vec<ObjectiveGuardFailure> {
    let mut failures = Vec::new();

    if input.candidate.score.frame_p99_ms > input.baseline.score.frame_p99_ms + frame_p99_slack_ms {
        failures.push(guard_failure(
            "foreground_frame_pacing",
            format!(
                "candidate frame_p99_ms {:.3} exceeded baseline {:.3} + slack {:.3}",
                input.candidate.score.frame_p99_ms,
                input.baseline.score.frame_p99_ms,
                frame_p99_slack_ms
            ),
        ));
    }

    if common::foreground_over_5ms_comparison(input) == PrimaryMetricComparison::Regressed {
        failures.push(guard_failure(
            "foreground_latency",
            "candidate foreground over-5ms rate regressed",
        ));
    }

    failures
}

fn confidence_for(
    outcome: &ObjectiveOutcome,
    primary_signal: &ObjectiveSignal,
    guard_failures: &[ObjectiveGuardFailure],
) -> Confidence {
    if matches!(outcome, ObjectiveOutcome::Invalid { .. }) {
        return Confidence::Low;
    }

    if !guard_failures.is_empty() {
        return match primary_signal.quality {
            ObjectiveSignalQuality::Direct => Confidence::Medium,
            _ => Confidence::Low,
        };
    }

    match primary_signal.quality {
        ObjectiveSignalQuality::Direct => Confidence::High,
        ObjectiveSignalQuality::Derived | ObjectiveSignalQuality::Approximate => Confidence::Medium,
        ObjectiveSignalQuality::Missing => Confidence::Low,
    }
}

fn pair_quality(
    baseline: ObjectiveSignalQuality,
    candidate: ObjectiveSignalQuality,
) -> ObjectiveSignalQuality {
    baseline.max(candidate)
}

fn degraded_overlap_reason(baseline: Option<&str>, candidate: Option<&str>) -> Option<String> {
    [baseline, candidate]
        .into_iter()
        .flatten()
        .find(|quality| *quality != "direct")
        .map(|quality| format!("block I/O overlap evidence quality is {quality}"))
}

fn guard_failure(guard: impl Into<String>, reason: impl Into<String>) -> ObjectiveGuardFailure {
    ObjectiveGuardFailure {
        guard: guard.into(),
        reason: reason.into(),
    }
}

fn format_optional_u64(value: Option<u64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "missing".to_owned())
}

fn format_optional_bool(value: Option<bool>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "missing".to_owned())
}

fn format_optional_f64(value: Option<f64>) -> String {
    value
        .map(format_f64)
        .unwrap_or_else(|| "missing".to_owned())
}

fn format_f64(value: f64) -> String {
    format!("{value:.6}")
}
