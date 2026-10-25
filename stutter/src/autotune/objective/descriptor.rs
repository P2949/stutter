//! Authoritative objective contract metadata, shared by runtime and documentation.
use serde::{Deserialize, Serialize};

use super::{ObjectiveKind, ObjectiveSignalQuality};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveFallbackPolicy {
    None,
    NormalizedScoreWhenMissingOrUnchanged,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveMaturity {
    Experimental,
    Validating,
    Supported,
}

/// Declared contract, not a claim that the current observation has this quality.
/// Owned fields allow the exact source contract to survive artifact round trips.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObjectiveDescriptor {
    pub kind: ObjectiveKind,
    pub primary_metric: String,
    pub primary_signal_quality: ObjectiveSignalQuality,
    pub required_signals: Vec<String>,
    pub fallback_policy: ObjectiveFallbackPolicy,
    pub guardrails: Vec<String>,
    /// Eligible under separately configured experimental action/safety policy.
    /// This flag neither grants privileges nor enables autonomous mode.
    pub autonomous_apply_allowed: bool,
    pub maturity: ObjectiveMaturity,
    pub limitations: String,
}

impl ObjectiveDescriptor {
    pub fn summary(&self) -> String {
        format!(
            "primary_metric={} quality={} fallback={:?} maturity={:?}",
            self.primary_metric,
            self.primary_signal_quality.as_str(),
            self.fallback_policy,
            self.maturity
        )
    }
}

impl ObjectiveKind {
    pub const ALL: [Self; 9] = [
        Self::StutterScore,
        Self::GameFramePacing,
        Self::GameRunnableLatency,
        Self::DesktopInteractivity,
        Self::BrowserInteractivity,
        Self::CompileThroughputWithForegroundProtection,
        Self::IoLatency,
        Self::IrqOverlapReduction,
        Self::ThermalRecovery,
    ];

    pub fn descriptor(self) -> ObjectiveDescriptor {
        use ObjectiveFallbackPolicy::{
            None, NormalizedScoreWhenMissingOrUnchanged as ScoreFallback,
        };
        use ObjectiveMaturity::{Experimental, Validating};
        use ObjectiveSignalQuality::{Approximate, Derived, Direct};
        // Exhaustive: new objectives cannot compile without a contract decision.
        let (metric, quality, required, fallback, guards, maturity, limitations): (
            &str,
            _,
            &[&str],
            _,
            &[&str],
            _,
            &str,
        ) = match self {
            Self::StutterScore => (
                "normalized_score_per_sample",
                Derived,
                &["scored_samples", "score_total"],
                None,
                &["frame_p99", "over_5ms_rate"],
                Experimental,
                "Ranking proxy within comparable experiments; weights are not empirically calibrated.",
            ),
            Self::GameFramePacing => (
                "frame_p99_ms",
                Derived,
                &["frame_p99_ms_or_explicit_score_fallback"],
                ScoreFallback,
                &[
                    "foreground_over_5ms_rate",
                    "foreground_frame_pacing",
                    "generic_score",
                ],
                Experimental,
                "Frame source quality varies; unchanged or missing primary uses explicit score fallback.",
            ),
            Self::GameRunnableLatency => (
                "runnable_p99_ns",
                Direct,
                &[
                    "complete_target_scheduler_histogram",
                    "stable_target_process_identity",
                    "at_least_1000_runnable_samples",
                ],
                None,
                &[
                    "runnable_over_5ms_rate",
                    "frame_p99",
                    "foreground_over_5ms_rate",
                    "foreground_frame_pacing",
                ],
                Validating,
                "Merged latency-sensitive target-game histograms; finite buckets and provisional effect thresholds; live observation, affinity and game-runnable acceptance exist, and the remaining limitation is decision-threshold calibration.",
            ),
            Self::DesktopInteractivity => (
                "foreground_over_5ms_per_sample",
                Derived,
                &["foreground_latency_or_explicit_score_fallback"],
                ScoreFallback,
                &["generic_score"],
                Experimental,
                "Existing derived foreground signal; unchanged or missing primary uses score fallback.",
            ),
            Self::BrowserInteractivity => (
                "foreground_over_5ms_per_sample",
                Approximate,
                &["foreground_latency_or_explicit_score_fallback"],
                ScoreFallback,
                &["generic_score"],
                Experimental,
                "Uses the existing foreground approximation, without a browser-specific direct primary measurement.",
            ),
            Self::CompileThroughputWithForegroundProtection => (
                "compile_fixed_work_elapsed_ns",
                Direct,
                &[
                    "successful_identical_fixed_work",
                    "stable_target_process_identity",
                    "ordered_distinct_executions",
                    "foreground_latency",
                ],
                None,
                &[
                    "foreground_over_5ms_rate",
                    "foreground_frame_pacing",
                    "observed_memory_pressure",
                ],
                Validating,
                "Cache/toolchain/environment policy is harness-declared; foreground protection is derived; provisional 10 percent threshold and live acceptance pending.",
            ),
            Self::IoLatency => (
                "block_io_overlap_count_then_worst_latency_ns",
                Direct,
                &["block_io_overlap_count", "block_io_worst_latency_ns"],
                None,
                &["generic_score"],
                Experimental,
                "Overlap evidence is correlation, not proof of causal storage delay; source trust varies.",
            ),
            Self::IrqOverlapReduction => (
                "irq_overlap_count_then_worst_overlap_ns",
                Direct,
                &["irq_overlap_count", "irq_worst_overlap_ns"],
                None,
                &["generic_score"],
                Experimental,
                "Overlap evidence is correlation, not proof of causal interrupt delay.",
            ),
            Self::ThermalRecovery => (
                "thermal_degraded_then_throttle_count",
                Derived,
                &["thermal_degraded", "thermal_throttle_count"],
                None,
                &["generic_score"],
                Experimental,
                "Thermal evidence depends on available platform telemetry.",
            ),
        };
        let mut guardrails = vec![
            "target_presence",
            "experiment_data_quality",
            "valid_nonzero_scored_samples",
            "thermal_degradation",
            "thermal_throttle_count",
            "cpu_power_limit",
            "gpu_power_limit",
        ];
        guardrails.extend_from_slice(guards);
        ObjectiveDescriptor {
            kind: self,
            primary_metric: metric.to_owned(),
            primary_signal_quality: quality,
            required_signals: required.iter().map(|value| (*value).to_owned()).collect(),
            fallback_policy: fallback,
            guardrails: guardrails.into_iter().map(str::to_owned).collect(),
            autonomous_apply_allowed: true,
            maturity,
            limitations: limitations.to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descriptors_round_trip_and_keep_direct_objectives_without_score_fallback() {
        for kind in ObjectiveKind::ALL {
            let descriptor = kind.descriptor();
            assert_eq!(descriptor.kind, kind);
            assert!(!descriptor.primary_metric.is_empty());
            assert!(!descriptor.required_signals.is_empty());
            assert!(!descriptor.limitations.is_empty());
            assert_ne!(descriptor.maturity, ObjectiveMaturity::Supported);
            let json = serde_json::to_string(&descriptor).unwrap();
            assert_eq!(
                serde_json::from_str::<ObjectiveDescriptor>(&json).unwrap(),
                descriptor
            );
        }
        for kind in [
            ObjectiveKind::GameRunnableLatency,
            ObjectiveKind::CompileThroughputWithForegroundProtection,
        ] {
            assert_eq!(
                kind.descriptor().fallback_policy,
                ObjectiveFallbackPolicy::None
            );
            assert_eq!(
                kind.descriptor().primary_signal_quality,
                ObjectiveSignalQuality::Direct
            );
        }
    }
}
