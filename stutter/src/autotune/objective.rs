use serde::{Deserialize, Serialize};

use crate::autotune::{
    comparison::{
        ExperimentComparisonInput, ExperimentDataQuality, ExperimentResult, compare_experiment,
        regression_percent_f64,
    },
    experiment::WindowScore,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveKind {
    #[default]
    StutterScore,
    GameFramePacing,
    GameRunnableLatency,
    DesktopInteractivity,
    BrowserInteractivity,
    CompileThroughputWithForegroundProtection,
    IoLatency,
    IrqOverlapReduction,
    ThermalRecovery,
}

#[derive(Clone, Debug)]
pub struct ObjectiveComparisonInput<'a> {
    pub objective: ObjectiveKind,
    pub baseline: &'a WindowScore,
    pub candidate: &'a WindowScore,
    pub baseline_signals: &'a ObjectiveSignals,
    pub candidate_signals: &'a ObjectiveSignals,
    pub data_quality: ExperimentDataQuality,
    pub target_disappeared: bool,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveSignalQuality {
    Direct,
    Derived,
    Approximate,
    #[default]
    Missing,
}

impl ObjectiveSignalQuality {
    pub fn is_missing(self) -> bool {
        self == Self::Missing
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Derived => "derived",
            Self::Approximate => "approximate",
            Self::Missing => "missing",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObjectiveSignalQualitySnapshot {
    pub block_io_overlap: ObjectiveSignalQuality,
    pub irq_overlap: ObjectiveSignalQuality,
    pub thermal: ObjectiveSignalQuality,
    pub cpu_power: ObjectiveSignalQuality,
    pub gpu_power: ObjectiveSignalQuality,
    pub gpu_active_render_node: ObjectiveSignalQuality,
    pub memory_pressure: ObjectiveSignalQuality,
    pub swap_activity: ObjectiveSignalQuality,
    pub dirty_writeback: ObjectiveSignalQuality,
    pub frame_pacing: ObjectiveSignalQuality,
    pub foreground_latency: ObjectiveSignalQuality,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ObjectiveSignals {
    pub block_io_overlap_count: Option<u64>,
    pub block_io_worst_latency_ns: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_io_overlap_basis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_io_overlap_trust: Option<String>,
    pub irq_overlap_count: Option<u64>,
    pub irq_worst_overlap_ns: Option<u64>,
    pub irq_hot_irq: Option<u32>,
    pub irq_hot_cpu: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub irq_overlap_basis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub irq_overlap_trust: Option<String>,
    pub thermal_degraded: Option<bool>,
    pub thermal_throttle_count: Option<u64>,
    pub cpu_power_limited: Option<bool>,
    pub cpu_power_limited_cpu: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_power_limit_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_power_limited_policy: Option<String>,
    pub gpu_power_limited: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_power_limit_reason: Option<String>,
    pub gpu_busy_percent: Option<u32>,
    pub gpu_clock_mhz: Option<u32>,
    pub gpu_temp_millidegrees: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_drm_card: Option<String>,
    pub gpu_active_render_node: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_focus_confidence: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_focus_source: Option<String>,
    pub memory_pressure_some_avg10_percent: Option<f32>,
    pub swap_activity_events: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mem_stall_spike_count: Option<u64>,
    pub dirty_writeback_events: Option<u64>,
    pub frame_p99_ms: Option<f64>,
    pub foreground_over_5ms: Option<u64>,
    #[serde(default)]
    pub signal_quality: ObjectiveSignalQualitySnapshot,
}

impl ObjectiveSignals {
    pub fn has_block_io_signal(&self) -> bool {
        self.block_io_overlap_count.is_some() && self.block_io_worst_latency_ns.is_some()
    }

    pub fn has_irq_signal(&self) -> bool {
        self.irq_overlap_count.is_some() && self.irq_worst_overlap_ns.is_some()
    }

    pub fn has_thermal_signal(&self) -> bool {
        self.thermal_degraded.is_some() && self.thermal_throttle_count.is_some()
    }

    pub fn missing_required_signals_for(&self, objective: ObjectiveKind) -> Vec<&'static str> {
        let mut missing = Vec::new();

        match objective {
            ObjectiveKind::IoLatency => {
                if self.block_io_overlap_count.is_none() {
                    missing.push("block_io_overlap_count");
                }
                if self.block_io_worst_latency_ns.is_none() {
                    missing.push("block_io_worst_latency_ns");
                }
            }
            ObjectiveKind::IrqOverlapReduction => {
                if self.irq_overlap_count.is_none() {
                    missing.push("irq_overlap_count");
                }
                if self.irq_worst_overlap_ns.is_none() {
                    missing.push("irq_worst_overlap_ns");
                }
            }
            ObjectiveKind::ThermalRecovery => {
                if self.thermal_degraded.is_none() {
                    missing.push("thermal_degraded");
                }
                if self.thermal_throttle_count.is_none() {
                    missing.push("thermal_throttle_count");
                }
            }
            _ => {}
        }

        missing
    }

    pub fn from_window_score(score: &WindowScore) -> Self {
        Self {
            frame_p99_ms: Some(score.score.frame_p99_ms),
            foreground_over_5ms: Some(score.score.over_5ms),
            signal_quality: ObjectiveSignalQualitySnapshot {
                frame_pacing: ObjectiveSignalQuality::Derived,
                foreground_latency: ObjectiveSignalQuality::Derived,
                ..ObjectiveSignalQualitySnapshot::default()
            },
            ..Self::default()
        }
    }
}

pub fn compare_for_objective(input: ObjectiveComparisonInput<'_>) -> ExperimentResult {
    let generic_score_result = compare_experiment(ExperimentComparisonInput {
        baseline: input.baseline,
        candidate: input.candidate,
        data_quality: input.data_quality,
        target_disappeared: input.target_disappeared,
    });

    if let Some(result) = reject_invalid_or_low_quality(&input, &generic_score_result) {
        return result;
    }

    if let Some(result) = missing_objective_signals(&input) {
        return result;
    }

    if let Some(result) = reject_if_power_or_thermal_regressed(&input) {
        return result;
    }

    match input.objective {
        ObjectiveKind::StutterScore => compare_stutter_score_objective(generic_score_result),
        ObjectiveKind::GameFramePacing => {
            compare_game_frame_pacing_objective(input, generic_score_result)
        }
        ObjectiveKind::GameRunnableLatency => {
            compare_game_runnable_latency_objective(input, generic_score_result)
        }
        ObjectiveKind::DesktopInteractivity => {
            compare_desktop_interactivity_objective(input, generic_score_result)
        }
        ObjectiveKind::BrowserInteractivity => {
            compare_browser_interactivity_objective(input, generic_score_result)
        }
        ObjectiveKind::CompileThroughputWithForegroundProtection => {
            compare_compile_throughput_with_foreground_protection_objective(
                input,
                generic_score_result,
            )
        }
        ObjectiveKind::IoLatency => compare_io_latency_objective(input, generic_score_result),
        ObjectiveKind::IrqOverlapReduction => {
            compare_irq_overlap_reduction_objective(input, generic_score_result)
        }
        ObjectiveKind::ThermalRecovery => {
            compare_thermal_recovery_objective(input, generic_score_result)
        }
    }
}

fn reject_invalid_or_low_quality(
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

fn missing_objective_signals(input: &ObjectiveComparisonInput<'_>) -> Option<ExperimentResult> {
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

fn compare_stutter_score_objective(generic_score_result: ExperimentResult) -> ExperimentResult {
    generic_score_result
}

fn compare_game_frame_pacing_objective(
    input: ObjectiveComparisonInput<'_>,
    generic_score_result: ExperimentResult,
) -> ExperimentResult {
    if let Some(result) = reject_if_foreground_regressed(&input) {
        return result;
    }

    match frame_p99_comparison(&input, 1.0) {
        PrimaryMetricComparison::Improved => improved_with_generic_guard(generic_score_result),
        PrimaryMetricComparison::Regressed => objective_regressed(&input),
        PrimaryMetricComparison::Unchanged | PrimaryMetricComparison::Missing => {
            generic_score_result
        }
    }
}

fn compare_game_runnable_latency_objective(
    input: ObjectiveComparisonInput<'_>,
    generic_score_result: ExperimentResult,
) -> ExperimentResult {
    reject_if_frame_pacing_regressed(&input).unwrap_or(generic_score_result)
}

fn compare_desktop_interactivity_objective(
    input: ObjectiveComparisonInput<'_>,
    generic_score_result: ExperimentResult,
) -> ExperimentResult {
    compare_interactivity_objective(input, generic_score_result)
}

fn compare_browser_interactivity_objective(
    input: ObjectiveComparisonInput<'_>,
    generic_score_result: ExperimentResult,
) -> ExperimentResult {
    compare_interactivity_objective(input, generic_score_result)
}

fn compare_compile_throughput_with_foreground_protection_objective(
    input: ObjectiveComparisonInput<'_>,
    generic_score_result: ExperimentResult,
) -> ExperimentResult {
    // TODO(compile_progress_intervals): add a direct compile-throughput signal and make it primary.
    reject_if_foreground_regressed(&input).unwrap_or(generic_score_result)
}

fn compare_io_latency_objective(
    input: ObjectiveComparisonInput<'_>,
    generic_score_result: ExperimentResult,
) -> ExperimentResult {
    let baseline_count = input.baseline_signals.block_io_overlap_count.unwrap_or(0);
    let candidate_count = input.candidate_signals.block_io_overlap_count.unwrap_or(0);
    let baseline_worst = input
        .baseline_signals
        .block_io_worst_latency_ns
        .unwrap_or(0);
    let candidate_worst = input
        .candidate_signals
        .block_io_worst_latency_ns
        .unwrap_or(0);

    if candidate_count > baseline_count || candidate_worst > baseline_worst {
        return objective_regressed(&input);
    }

    if candidate_count < baseline_count
        || (candidate_count == baseline_count && candidate_worst < baseline_worst)
    {
        return improved_with_generic_guard(generic_score_result);
    }

    ExperimentResult::Inconclusive {
        reason: "block I/O objective primary signal did not improve".to_owned(),
    }
}

fn compare_irq_overlap_reduction_objective(
    input: ObjectiveComparisonInput<'_>,
    generic_score_result: ExperimentResult,
) -> ExperimentResult {
    let baseline_count = input.baseline_signals.irq_overlap_count.unwrap_or(0);
    let candidate_count = input.candidate_signals.irq_overlap_count.unwrap_or(0);
    let baseline_worst = input.baseline_signals.irq_worst_overlap_ns.unwrap_or(0);
    let candidate_worst = input.candidate_signals.irq_worst_overlap_ns.unwrap_or(0);

    if candidate_count > baseline_count || candidate_worst > baseline_worst {
        return objective_regressed(&input);
    }

    if candidate_count < baseline_count
        || (candidate_count == baseline_count && candidate_worst < baseline_worst)
    {
        return improved_with_generic_guard(generic_score_result);
    }

    ExperimentResult::Inconclusive {
        reason: "IRQ objective primary signal did not improve".to_owned(),
    }
}

fn compare_thermal_recovery_objective(
    input: ObjectiveComparisonInput<'_>,
    generic_score_result: ExperimentResult,
) -> ExperimentResult {
    let baseline_degraded = input.baseline_signals.thermal_degraded.unwrap_or(false);
    let candidate_degraded = input.candidate_signals.thermal_degraded.unwrap_or(false);
    let baseline_throttles = input.baseline_signals.thermal_throttle_count.unwrap_or(0);
    let candidate_throttles = input.candidate_signals.thermal_throttle_count.unwrap_or(0);

    if candidate_degraded && !baseline_degraded || candidate_throttles > baseline_throttles {
        return objective_regressed(&input);
    }

    if baseline_degraded && !candidate_degraded || candidate_throttles < baseline_throttles {
        return improved_with_generic_guard(generic_score_result);
    }

    ExperimentResult::Inconclusive {
        reason: "thermal objective primary signal did not improve".to_owned(),
    }
}

fn reject_if_power_or_thermal_regressed(
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
enum PrimaryMetricComparison {
    Improved,
    Regressed,
    Unchanged,
    Missing,
}

fn frame_p99_comparison(
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

fn compare_interactivity_objective(
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

fn foreground_over_5ms_comparison(input: &ObjectiveComparisonInput<'_>) -> PrimaryMetricComparison {
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

fn reject_if_frame_pacing_regressed(
    input: &ObjectiveComparisonInput<'_>,
) -> Option<ExperimentResult> {
    if frame_p99_comparison(input, 1.0) == PrimaryMetricComparison::Regressed {
        return Some(objective_regressed(input));
    }

    reject_if_foreground_regressed(input)
}

fn reject_if_foreground_regressed(
    input: &ObjectiveComparisonInput<'_>,
) -> Option<ExperimentResult> {
    if input.candidate.score.frame_p99_ms > input.baseline.score.frame_p99_ms + 2.0
        || foreground_over_5ms_comparison(input) == PrimaryMetricComparison::Regressed
    {
        return Some(objective_regressed(input));
    }

    None
}

fn improved_with_generic_guard(generic_score_result: ExperimentResult) -> ExperimentResult {
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

fn objective_regressed(input: &ObjectiveComparisonInput<'_>) -> ExperimentResult {
    ExperimentResult::Regressed {
        regression_percent: objective_regression_percent(input.baseline, input.candidate),
    }
}

fn objective_regression_percent(baseline: &WindowScore, candidate: &WindowScore) -> f64 {
    match (baseline.score_per_sample(), candidate.score_per_sample()) {
        (Some(baseline), Some(candidate)) => regression_percent_f64(baseline, candidate),
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scorer::StutterScore;

    fn score(total: u64, frame_p99_ms: f64, over_5ms: u64) -> WindowScore {
        WindowScore {
            started_unix_nanos: 1,
            finished_unix_nanos: 2,
            interval_count: 5,
            scored_samples: 100,
            scored_task_count: 1,
            score: StutterScore {
                total,
                frame_p99_ms,
                over_5ms,
                ..StutterScore::default()
            },
        }
    }

    fn compare(
        objective: ObjectiveKind,
        baseline: &WindowScore,
        candidate: &WindowScore,
        baseline_signals: &ObjectiveSignals,
        candidate_signals: &ObjectiveSignals,
    ) -> ExperimentResult {
        compare_for_objective(ObjectiveComparisonInput {
            objective,
            baseline,
            candidate,
            baseline_signals,
            candidate_signals,
            data_quality: ExperimentDataQuality::High,
            target_disappeared: false,
        })
    }

    fn io_signals(score: &WindowScore, count: u64, worst_latency_ns: u64) -> ObjectiveSignals {
        ObjectiveSignals {
            block_io_overlap_count: Some(count),
            block_io_worst_latency_ns: Some(worst_latency_ns),
            ..ObjectiveSignals::from_window_score(score)
        }
    }

    fn irq_signals(score: &WindowScore, count: u64, worst_overlap_ns: u64) -> ObjectiveSignals {
        ObjectiveSignals {
            irq_overlap_count: Some(count),
            irq_worst_overlap_ns: Some(worst_overlap_ns),
            ..ObjectiveSignals::from_window_score(score)
        }
    }

    fn thermal_signals(
        score: &WindowScore,
        degraded: bool,
        throttle_count: u64,
    ) -> ObjectiveSignals {
        ObjectiveSignals {
            thermal_degraded: Some(degraded),
            thermal_throttle_count: Some(throttle_count),
            ..ObjectiveSignals::from_window_score(score)
        }
    }

    #[test]
    fn game_objective_rejects_frame_p99_regression_even_when_total_improves() {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(800, 14.0, 1);

        let baseline_signals = ObjectiveSignals::from_window_score(&baseline);
        let candidate_signals = ObjectiveSignals::from_window_score(&candidate);
        let result = compare(
            ObjectiveKind::GameFramePacing,
            &baseline,
            &candidate,
            &baseline_signals,
            &candidate_signals,
        );

        assert!(matches!(result, ExperimentResult::Regressed { .. }));
    }

    #[test]
    fn desktop_objective_rejects_over_5ms_regression() {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(800, 12.0, 2);

        let baseline_signals = ObjectiveSignals::from_window_score(&baseline);
        let candidate_signals = ObjectiveSignals::from_window_score(&candidate);
        let result = compare(
            ObjectiveKind::DesktopInteractivity,
            &baseline,
            &candidate,
            &baseline_signals,
            &candidate_signals,
        );

        assert!(matches!(result, ExperimentResult::Regressed { .. }));
    }

    #[test]
    fn default_score_objective_keeps_existing_improvement_path() {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(800, 12.0, 1);

        let baseline_signals = ObjectiveSignals::from_window_score(&baseline);
        let candidate_signals = ObjectiveSignals::from_window_score(&candidate);
        let result = compare(
            ObjectiveKind::StutterScore,
            &baseline,
            &candidate,
            &baseline_signals,
            &candidate_signals,
        );

        assert!(matches!(result, ExperimentResult::Improved { .. }));
    }

    #[test]
    fn io_objective_is_inconclusive_when_io_signal_does_not_improve() {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(800, 12.0, 1);
        let baseline_signals = io_signals(&baseline, 1, 2_000_000);
        let candidate_signals = io_signals(&candidate, 1, 2_000_000);

        let result = compare(
            ObjectiveKind::IoLatency,
            &baseline,
            &candidate,
            &baseline_signals,
            &candidate_signals,
        );

        assert!(matches!(result, ExperimentResult::Inconclusive { .. }));
    }

    #[test]
    fn io_objective_improves_when_io_overlap_drops_even_if_generic_score_is_flat() {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(1_000, 12.0, 1);
        let baseline_signals = io_signals(&baseline, 3, 2_000_000);
        let candidate_signals = io_signals(&candidate, 1, 2_000_000);

        let result = compare(
            ObjectiveKind::IoLatency,
            &baseline,
            &candidate,
            &baseline_signals,
            &candidate_signals,
        );

        assert!(matches!(result, ExperimentResult::Improved { .. }));
    }

    #[test]
    fn io_objective_rejects_when_io_overlap_worsens_even_if_generic_score_improves() {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(800, 12.0, 1);
        let baseline_signals = io_signals(&baseline, 1, 2_000_000);
        let candidate_signals = io_signals(&candidate, 2, 3_000_000);

        let result = compare(
            ObjectiveKind::IoLatency,
            &baseline,
            &candidate,
            &baseline_signals,
            &candidate_signals,
        );

        assert!(matches!(result, ExperimentResult::Regressed { .. }));
    }

    #[test]
    fn io_objective_rejects_when_required_signals_missing() {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(800, 12.0, 1);
        let baseline_signals = ObjectiveSignals::from_window_score(&baseline);
        let candidate_signals = ObjectiveSignals::from_window_score(&candidate);

        let result = compare(
            ObjectiveKind::IoLatency,
            &baseline,
            &candidate,
            &baseline_signals,
            &candidate_signals,
        );

        assert!(matches!(result, ExperimentResult::Inconclusive { .. }));
    }

    #[test]
    fn irq_objective_improves_when_overlap_drops_even_if_generic_score_is_flat() {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(1_000, 12.0, 1);
        let baseline_signals = irq_signals(&baseline, 3, 5_000_000);
        let candidate_signals = irq_signals(&candidate, 1, 5_000_000);

        let result = compare(
            ObjectiveKind::IrqOverlapReduction,
            &baseline,
            &candidate,
            &baseline_signals,
            &candidate_signals,
        );

        assert!(matches!(result, ExperimentResult::Improved { .. }));
    }

    #[test]
    fn irq_objective_rejects_when_overlap_worsens_even_if_generic_score_improves() {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(800, 12.0, 1);
        let baseline_signals = irq_signals(&baseline, 1, 5_000_000);
        let candidate_signals = irq_signals(&candidate, 2, 6_000_000);

        let result = compare(
            ObjectiveKind::IrqOverlapReduction,
            &baseline,
            &candidate,
            &baseline_signals,
            &candidate_signals,
        );

        assert!(matches!(result, ExperimentResult::Regressed { .. }));
    }

    #[test]
    fn irq_objective_rejects_when_required_signals_missing() {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(800, 12.0, 1);
        let baseline_signals = ObjectiveSignals::from_window_score(&baseline);
        let candidate_signals = ObjectiveSignals::from_window_score(&candidate);

        let result = compare(
            ObjectiveKind::IrqOverlapReduction,
            &baseline,
            &candidate,
            &baseline_signals,
            &candidate_signals,
        );

        assert!(matches!(result, ExperimentResult::Inconclusive { .. }));
    }

    #[test]
    fn thermal_objective_improves_when_candidate_clears_degraded_state() {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(1_000, 12.0, 1);
        let baseline_signals = thermal_signals(&baseline, true, 1);
        let candidate_signals = thermal_signals(&candidate, false, 1);

        let result = compare(
            ObjectiveKind::ThermalRecovery,
            &baseline,
            &candidate,
            &baseline_signals,
            &candidate_signals,
        );

        assert!(matches!(result, ExperimentResult::Improved { .. }));
    }

    #[test]
    fn thermal_objective_improves_when_throttle_count_drops() {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(1_000, 12.0, 1);
        let baseline_signals = thermal_signals(&baseline, false, 3);
        let candidate_signals = thermal_signals(&candidate, false, 1);

        let result = compare(
            ObjectiveKind::ThermalRecovery,
            &baseline,
            &candidate,
            &baseline_signals,
            &candidate_signals,
        );

        assert!(matches!(result, ExperimentResult::Improved { .. }));
    }

    #[test]
    fn thermal_objective_rejects_new_thermal_degradation() {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(800, 12.0, 1);
        let baseline_signals = thermal_signals(&baseline, false, 0);
        let candidate_signals = thermal_signals(&candidate, true, 1);

        let result = compare(
            ObjectiveKind::ThermalRecovery,
            &baseline,
            &candidate,
            &baseline_signals,
            &candidate_signals,
        );

        assert!(matches!(result, ExperimentResult::Regressed { .. }));
    }

    #[test]
    fn thermal_objective_rejects_new_power_limit() {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(800, 12.0, 1);
        let baseline_signals = ObjectiveSignals {
            cpu_power_limited: Some(false),
            ..thermal_signals(&baseline, false, 0)
        };
        let candidate_signals = ObjectiveSignals {
            cpu_power_limited: Some(true),
            ..thermal_signals(&candidate, false, 0)
        };

        let result = compare(
            ObjectiveKind::ThermalRecovery,
            &baseline,
            &candidate,
            &baseline_signals,
            &candidate_signals,
        );

        assert!(matches!(result, ExperimentResult::Regressed { .. }));
    }

    #[test]
    fn power_or_thermal_regression_rejects_frame_pacing_improvement() {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(800, 11.0, 1);
        let baseline_signals = ObjectiveSignals {
            gpu_power_limited: Some(false),
            ..thermal_signals(&baseline, false, 0)
        };
        let candidate_signals = ObjectiveSignals {
            gpu_power_limited: Some(true),
            ..thermal_signals(&candidate, true, 1)
        };

        let result = compare(
            ObjectiveKind::GameFramePacing,
            &baseline,
            &candidate,
            &baseline_signals,
            &candidate_signals,
        );

        assert!(matches!(result, ExperimentResult::Regressed { .. }));
    }
}
