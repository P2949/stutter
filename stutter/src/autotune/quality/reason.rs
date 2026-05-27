use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::{input::OnlineDataQualityInput, policy::OnlineDataQualityPolicy};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum OnlineDataQuality {
    High,
    Medium { reasons: Vec<String> },
    Low { reasons: Vec<String> },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum OnlineDataQualityReasonCode {
    InsufficientSamples,
    TargetMissing,
    FocusLowConfidence,
    DropCountersHigh,
    WorkloadChanged,
    ThermalDegraded,
    FrameDataInvalid,
    FrameCountMismatch,
    MeasurementUncertain,
}

impl OnlineDataQualityReasonCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::InsufficientSamples => "insufficient_samples",
            Self::TargetMissing => "target_missing",
            Self::FocusLowConfidence => "focus_low_confidence",
            Self::DropCountersHigh => "drop_counters_high",
            Self::WorkloadChanged => "workload_changed",
            Self::ThermalDegraded => "thermal_degraded",
            Self::FrameDataInvalid => "frame_data_invalid",
            Self::FrameCountMismatch => "frame_count_mismatch",
            Self::MeasurementUncertain => "measurement_uncertain",
        }
    }
}

impl OnlineDataQuality {
    pub fn reasons(&self) -> &[String] {
        match self {
            Self::High => &[],
            Self::Medium { reasons } | Self::Low { reasons } => reasons,
        }
    }

    pub fn is_high(&self) -> bool {
        matches!(self, Self::High)
    }

    pub fn is_medium(&self) -> bool {
        matches!(self, Self::Medium { .. })
    }

    pub fn is_low(&self) -> bool {
        matches!(self, Self::Low { .. })
    }

    pub fn blocks_action(&self) -> bool {
        self.is_low()
    }

    pub fn reason_codes(&self) -> Vec<OnlineDataQualityReasonCode> {
        self.reasons()
            .iter()
            .map(|reason| data_quality_reason_code_for_reason(reason))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    pub fn reason_code_strings(&self) -> Vec<String> {
        self.reason_codes()
            .into_iter()
            .map(|code| code.as_str().to_owned())
            .collect()
    }

    pub fn primary_reason_code(&self) -> Option<OnlineDataQualityReasonCode> {
        self.reasons()
            .first()
            .map(|reason| data_quality_reason_code_for_reason(reason))
    }

    pub fn evaluate(input: OnlineDataQualityInput<'_>) -> Self {
        input.evaluate_with_policy(&OnlineDataQualityPolicy::default())
    }
}

impl Default for OnlineDataQuality {
    fn default() -> Self {
        Self::Low {
            reasons: vec!["no online data has been evaluated".to_owned()],
        }
    }
}

fn data_quality_reason_code_for_reason(reason: &str) -> OnlineDataQualityReasonCode {
    let lower = reason.to_ascii_lowercase();

    if lower.contains("fewer than min_scored_intervals")
        || lower.contains("fewer than min_scored_samples")
        || lower.contains("zero scored tasks")
        || lower.contains("insufficient")
    {
        OnlineDataQualityReasonCode::InsufficientSamples
    } else if lower.contains("target disappeared") || lower.contains("target missing") {
        OnlineDataQualityReasonCode::TargetMissing
    } else if lower.contains("focus") && lower.contains("confidence") {
        OnlineDataQualityReasonCode::FocusLowConfidence
    } else if lower.contains("drop counters") || lower.contains("dropped events") {
        OnlineDataQualityReasonCode::DropCountersHigh
    } else if lower.contains("target identity shifted")
        || lower.contains("workload changed")
        || lower.contains("scored identity overlap")
    {
        OnlineDataQualityReasonCode::WorkloadChanged
    } else if lower.contains("thermal") || lower.contains("overheat") {
        OnlineDataQualityReasonCode::ThermalDegraded
    } else if lower.contains("no frame data")
        || lower.contains("one window has frames and the other has none")
        || lower.contains("frame data mismatch")
    {
        OnlineDataQualityReasonCode::FrameDataInvalid
    } else if lower.contains("frame count differs") {
        OnlineDataQualityReasonCode::FrameCountMismatch
    } else {
        OnlineDataQualityReasonCode::MeasurementUncertain
    }
}
