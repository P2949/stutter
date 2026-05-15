use serde::{Deserialize, Serialize};

use crate::autotune::{
    comparison::{
        ExperimentComparisonInput, ExperimentDataQuality, ExperimentResult, compare_experiment,
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

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct ObjectiveSignals {
    pub block_io_overlap_count: Option<u64>,
    pub block_io_worst_latency_ns: Option<u64>,
    pub irq_overlap_count: Option<u64>,
    pub irq_worst_overlap_ns: Option<u64>,
    pub thermal_degraded: Option<bool>,
    pub thermal_throttle_count: Option<u64>,
    pub cpu_power_limited: Option<bool>,
    pub gpu_power_limited: Option<bool>,
    pub frame_p99_ms: Option<f64>,
    pub foreground_over_5ms: Option<u64>,
}

impl ObjectiveSignals {
    pub fn from_window_score(score: &WindowScore) -> Self {
        Self {
            frame_p99_ms: Some(score.score.frame_p99_ms),
            foreground_over_5ms: Some(score.score.over_5ms),
            ..Self::default()
        }
    }
}

pub fn compare_for_objective(input: ObjectiveComparisonInput<'_>) -> ExperimentResult {
    let base = compare_experiment(ExperimentComparisonInput {
        baseline: input.baseline,
        candidate: input.candidate,
        data_quality: input.data_quality,
        target_disappeared: input.target_disappeared,
    });

    if !matches!(base, ExperimentResult::Improved { .. }) {
        return base;
    }

    if missing_objective_signals(input.clone()) {
        return ExperimentResult::Regressed {
            regression_percent: 0.0,
        };
    }

    match input.objective {
        ObjectiveKind::GameFramePacing | ObjectiveKind::GameRunnableLatency => {
            reject_if_frame_pacing_regressed(input.baseline, input.candidate).unwrap_or(base)
        }
        ObjectiveKind::DesktopInteractivity | ObjectiveKind::BrowserInteractivity => {
            reject_if_interactivity_regressed(input.baseline, input.candidate).unwrap_or(base)
        }
        ObjectiveKind::CompileThroughputWithForegroundProtection => {
            reject_if_foreground_regressed(input.baseline, input.candidate).unwrap_or(base)
        }
        ObjectiveKind::IoLatency => compare_io_latency(input).unwrap_or(base),
        ObjectiveKind::IrqOverlapReduction => compare_irq_overlap_reduction(input).unwrap_or(base),
        ObjectiveKind::ThermalRecovery => compare_thermal_recovery(input).unwrap_or(base),
        ObjectiveKind::StutterScore => base,
    }
}

fn compare_io_latency(input: ObjectiveComparisonInput<'_>) -> Option<ExperimentResult> {
    let baseline_count = input.baseline_signals.block_io_overlap_count?;
    let candidate_count = input.candidate_signals.block_io_overlap_count?;
    let baseline_worst = input.baseline_signals.block_io_worst_latency_ns?;
    let candidate_worst = input.candidate_signals.block_io_worst_latency_ns?;

    if candidate_count > baseline_count || candidate_worst > baseline_worst {
        Some(ExperimentResult::Regressed {
            regression_percent: 0.0,
        })
    } else {
        None
    }
}

fn compare_irq_overlap_reduction(input: ObjectiveComparisonInput<'_>) -> Option<ExperimentResult> {
    let baseline_count = input.baseline_signals.irq_overlap_count?;
    let candidate_count = input.candidate_signals.irq_overlap_count?;
    let baseline_worst = input.baseline_signals.irq_worst_overlap_ns?;
    let candidate_worst = input.candidate_signals.irq_worst_overlap_ns?;

    if candidate_count >= baseline_count || candidate_worst > baseline_worst {
        Some(ExperimentResult::Regressed {
            regression_percent: 0.0,
        })
    } else {
        None
    }
}

fn compare_thermal_recovery(input: ObjectiveComparisonInput<'_>) -> Option<ExperimentResult> {
    let baseline_degraded = input.baseline_signals.thermal_degraded?;
    let candidate_degraded = input.candidate_signals.thermal_degraded?;
    let baseline_throttles = input.baseline_signals.thermal_throttle_count?;
    let candidate_throttles = input.candidate_signals.thermal_throttle_count?;

    if candidate_degraded && !baseline_degraded || candidate_throttles > baseline_throttles {
        return Some(ExperimentResult::Regressed {
            regression_percent: 0.0,
        });
    }

    if input.candidate_signals.cpu_power_limited.unwrap_or(false)
        && !input.baseline_signals.cpu_power_limited.unwrap_or(false)
        || input.candidate_signals.gpu_power_limited.unwrap_or(false)
            && !input.baseline_signals.gpu_power_limited.unwrap_or(false)
    {
        return Some(ExperimentResult::Regressed {
            regression_percent: 0.0,
        });
    }

    None
}

fn missing_objective_signals(input: ObjectiveComparisonInput<'_>) -> bool {
    match input.objective {
        ObjectiveKind::IoLatency => {
            input.baseline_signals.block_io_overlap_count.is_none()
                || input.candidate_signals.block_io_overlap_count.is_none()
                || input.baseline_signals.block_io_worst_latency_ns.is_none()
                || input.candidate_signals.block_io_worst_latency_ns.is_none()
        }
        ObjectiveKind::IrqOverlapReduction => {
            input.baseline_signals.irq_overlap_count.is_none()
                || input.candidate_signals.irq_overlap_count.is_none()
                || input.baseline_signals.irq_worst_overlap_ns.is_none()
                || input.candidate_signals.irq_worst_overlap_ns.is_none()
        }
        ObjectiveKind::ThermalRecovery => {
            input.baseline_signals.thermal_degraded.is_none()
                || input.candidate_signals.thermal_degraded.is_none()
                || input.baseline_signals.thermal_throttle_count.is_none()
                || input.candidate_signals.thermal_throttle_count.is_none()
        }
        _ => false,
    }
}

fn reject_if_frame_pacing_regressed(
    baseline: &WindowScore,
    candidate: &WindowScore,
) -> Option<ExperimentResult> {
    if candidate.score.frame_p99_ms > baseline.score.frame_p99_ms + 1.0 {
        return Some(ExperimentResult::Regressed {
            regression_percent: 0.0,
        });
    }
    if candidate.score.over_5ms > baseline.score.over_5ms {
        return Some(ExperimentResult::Regressed {
            regression_percent: 0.0,
        });
    }
    None
}

fn reject_if_interactivity_regressed(
    baseline: &WindowScore,
    candidate: &WindowScore,
) -> Option<ExperimentResult> {
    if candidate.score.over_5ms > baseline.score.over_5ms {
        return Some(ExperimentResult::Regressed {
            regression_percent: 0.0,
        });
    }
    None
}

fn reject_if_foreground_regressed(
    baseline: &WindowScore,
    candidate: &WindowScore,
) -> Option<ExperimentResult> {
    if candidate.score.frame_p99_ms > baseline.score.frame_p99_ms + 2.0
        || candidate.score.over_5ms > baseline.score.over_5ms
    {
        return Some(ExperimentResult::Regressed {
            regression_percent: 0.0,
        });
    }
    None
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
    fn io_objective_rejects_generic_improvement_when_io_signal_regresses() {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(800, 12.0, 1);
        let baseline_signals = ObjectiveSignals {
            block_io_overlap_count: Some(1),
            block_io_worst_latency_ns: Some(2_000_000),
            ..ObjectiveSignals::from_window_score(&baseline)
        };
        let candidate_signals = ObjectiveSignals {
            block_io_overlap_count: Some(2),
            block_io_worst_latency_ns: Some(3_000_000),
            ..ObjectiveSignals::from_window_score(&candidate)
        };

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
    fn irq_objective_rejects_generic_improvement_when_irq_signal_missing() {
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

        assert!(matches!(result, ExperimentResult::Regressed { .. }));
    }

    #[test]
    fn thermal_objective_rejects_new_power_limit() {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(800, 12.0, 1);
        let baseline_signals = ObjectiveSignals {
            thermal_degraded: Some(false),
            thermal_throttle_count: Some(0),
            cpu_power_limited: Some(false),
            ..ObjectiveSignals::from_window_score(&baseline)
        };
        let candidate_signals = ObjectiveSignals {
            thermal_degraded: Some(false),
            thermal_throttle_count: Some(0),
            cpu_power_limited: Some(true),
            ..ObjectiveSignals::from_window_score(&candidate)
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
}
