mod browser_interactivity;
mod common;
mod compile_throughput;
pub(crate) mod decision;
mod desktop_interactivity;
mod game_frame_pacing;
mod game_runnable_latency;
mod io_latency;
mod irq_overlap;
mod stutter_score;
mod thermal;

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
    #[serde(default)]
    pub compile_throughput: ObjectiveSignalQuality,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compile_progress_intervals: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compile_progress_samples: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compile_progress_source: Option<String>,
    #[serde(default)]
    pub signal_quality: ObjectiveSignalQualitySnapshot,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompileThroughputSignal {
    pub progress_intervals: Option<u64>,
    pub progress_samples: Option<u64>,
    pub quality: ObjectiveSignalQuality,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded_reason: Option<String>,
}

impl CompileThroughputSignal {
    pub fn direct_progress(&self) -> Option<(u64, u64)> {
        if self.quality != ObjectiveSignalQuality::Direct {
            return None;
        }

        Some((self.progress_intervals?, self.progress_samples.unwrap_or(0)))
    }
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

    pub fn compile_throughput_signal(&self) -> CompileThroughputSignal {
        if self.signal_quality.compile_throughput == ObjectiveSignalQuality::Direct
            && self.compile_progress_intervals.is_some()
        {
            return CompileThroughputSignal {
                progress_intervals: self.compile_progress_intervals,
                progress_samples: self.compile_progress_samples,
                quality: ObjectiveSignalQuality::Direct,
                degraded_reason: None,
            };
        }

        CompileThroughputSignal {
            progress_intervals: None,
            progress_samples: None,
            quality: ObjectiveSignalQuality::Derived,
            degraded_reason: Some(
                "direct compile progress intervals missing; using lower-confidence stutter-score proxy"
                    .to_owned(),
            ),
        }
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
    explain_for_objective(input).experiment_result()
}

pub fn explain_for_objective(input: ObjectiveComparisonInput<'_>) -> decision::ObjectiveDecision {
    let generic_score_result = compare_experiment(ExperimentComparisonInput {
        baseline: input.baseline,
        candidate: input.candidate,
        data_quality: input.data_quality,
        target_disappeared: input.target_disappeared,
    });

    let result = compare_result_for_objective(input.clone(), generic_score_result.clone());
    decision::ObjectiveDecision::from_comparison(&input, &generic_score_result, result)
}

fn compare_result_for_objective(
    input: ObjectiveComparisonInput<'_>,
    generic_score_result: ExperimentResult,
) -> ExperimentResult {
    if let Some(result) = common::reject_invalid_or_low_quality(&input, &generic_score_result) {
        return result;
    }

    if let Some(result) = common::missing_objective_signals(&input) {
        return result;
    }

    if let Some(result) = common::reject_if_power_or_thermal_regressed(&input) {
        return result;
    }

    match input.objective {
        ObjectiveKind::StutterScore => stutter_score::compare(generic_score_result),
        ObjectiveKind::GameFramePacing => game_frame_pacing::compare(input, generic_score_result),
        ObjectiveKind::GameRunnableLatency => {
            game_runnable_latency::compare(input, generic_score_result)
        }
        ObjectiveKind::DesktopInteractivity => {
            desktop_interactivity::compare(input, generic_score_result)
        }
        ObjectiveKind::BrowserInteractivity => {
            browser_interactivity::compare(input, generic_score_result)
        }
        ObjectiveKind::CompileThroughputWithForegroundProtection => {
            compile_throughput::compare(input, generic_score_result)
        }
        ObjectiveKind::IoLatency => io_latency::compare(input, generic_score_result),
        ObjectiveKind::IrqOverlapReduction => irq_overlap::compare(input, generic_score_result),
        ObjectiveKind::ThermalRecovery => thermal::compare(input, generic_score_result),
    }
}

#[cfg(test)]
mod tests;
