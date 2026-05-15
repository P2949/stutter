use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::tune::comparability::{
    ScoredIdentityCount, scored_identity_counts_to_map, scored_identity_overlap,
};

pub const DEFAULT_MIN_SCORED_INTERVALS: usize = 5;
pub const DEFAULT_MIN_SCORED_SAMPLES: u64 = 100;
pub const DEFAULT_MAX_DROP_COUNTER_TOTAL: u64 = 0;
pub(crate) const DEFAULT_FRAME_DATA_POLICY: FrameDataPolicy = FrameDataPolicy::Advisory;
pub const LOW_IDENTITY_OVERLAP_RATIO: f64 = 0.75;
pub const MEDIUM_IDENTITY_OVERLAP_RATIO: f64 = 0.90;
pub const LOW_FRAME_COUNT_RATIO: f64 = 1.50;
pub const MEDIUM_FRAME_COUNT_RATIO: f64 = 1.20;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FrameDataPolicy {
    Ignore,
    #[default]
    Advisory,
    Required,
}

impl FrameDataPolicy {
    pub fn requires_frames(self) -> bool {
        matches!(self, Self::Required)
    }
    pub fn checks_frame_count_mismatch(self) -> bool {
        !matches!(self, Self::Ignore)
    }
}

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

#[derive(Clone, Debug)]
pub struct OnlineDataQualityPolicy {
    pub min_scored_intervals: usize,
    pub min_scored_samples: u64,
    pub max_drop_counter_total: u64,
    pub frame_data_policy: FrameDataPolicy,
    pub low_identity_overlap_ratio: f64,
    pub medium_identity_overlap_ratio: f64,
    pub low_frame_count_ratio: f64,
    pub medium_frame_count_ratio: f64,
}

impl Default for OnlineDataQualityPolicy {
    fn default() -> Self {
        Self {
            min_scored_intervals: DEFAULT_MIN_SCORED_INTERVALS,
            min_scored_samples: DEFAULT_MIN_SCORED_SAMPLES,
            max_drop_counter_total: DEFAULT_MAX_DROP_COUNTER_TOTAL,
            frame_data_policy: DEFAULT_FRAME_DATA_POLICY,
            low_identity_overlap_ratio: LOW_IDENTITY_OVERLAP_RATIO,
            medium_identity_overlap_ratio: MEDIUM_IDENTITY_OVERLAP_RATIO,
            low_frame_count_ratio: LOW_FRAME_COUNT_RATIO,
            medium_frame_count_ratio: MEDIUM_FRAME_COUNT_RATIO,
        }
    }
}

#[derive(Clone, Debug)]
pub struct OnlineDataQualityInput<'a> {
    pub scored_intervals: usize,
    pub scored_samples: u64,
    pub scored_task_count: usize,
    pub drop_counter_total: u64,
    pub target_identity_shifted: bool,
    pub target_present: bool,
    pub frame_data_required: bool,
    pub frame_count: usize,
    pub baseline_frame_count: Option<usize>,
    pub candidate_frame_count: Option<usize>,
    pub baseline_scored_identity_counts: &'a [ScoredIdentityCount],
    pub candidate_scored_identity_counts: &'a [ScoredIdentityCount],
}

impl<'a> OnlineDataQualityInput<'a> {
    pub fn evaluate_with_policy(&self, policy: &OnlineDataQualityPolicy) -> OnlineDataQuality {
        let mut low_reasons = Vec::new();
        let mut medium_reasons = Vec::new();

        if self.scored_intervals < policy.min_scored_intervals {
            low_reasons.push(format!(
                "fewer than min_scored_intervals: scored_intervals={} min_scored_intervals={}",
                self.scored_intervals, policy.min_scored_intervals
            ));
        }

        if self.scored_samples < policy.min_scored_samples {
            low_reasons.push(format!(
                "fewer than min_scored_samples: scored_samples={} min_scored_samples={}",
                self.scored_samples, policy.min_scored_samples
            ));
        }

        if self.scored_task_count == 0 {
            low_reasons.push("zero scored tasks".to_owned());
        }

        if self.drop_counter_total > policy.max_drop_counter_total {
            low_reasons.push(format!(
                "drop counters above policy max: drop_counter_total={} max_drop_counter_total={}",
                self.drop_counter_total, policy.max_drop_counter_total
            ));
        } else if self.drop_counter_total > 0 {
            medium_reasons.push(format!(
                "drop counters are nonzero but within policy max: drop_counter_total={} max_drop_counter_total={}",
                self.drop_counter_total, policy.max_drop_counter_total
            ));
        }

        if self.target_identity_shifted {
            low_reasons.push("target identity shifted during candidate measurement".to_owned());
        }

        if !self.target_present {
            low_reasons.push("target disappeared".to_owned());
        }

        let frame_data_required =
            policy.frame_data_policy.requires_frames() || self.frame_data_required;
        if frame_data_required && self.frame_count == 0 {
            low_reasons.push(
                "frame data mismatch when frame-based scoring is required: no frame data"
                    .to_owned(),
            );
        }

        if policy.frame_data_policy.checks_frame_count_mismatch() {
            if let Some(reason) = frame_count_low_reason(
                self.baseline_frame_count,
                self.candidate_frame_count,
                policy.low_frame_count_ratio,
            ) {
                low_reasons.push(reason);
            } else if let Some(reason) = frame_count_medium_reason(
                self.baseline_frame_count,
                self.candidate_frame_count,
                policy.medium_frame_count_ratio,
            ) {
                medium_reasons.push(reason);
            }
        }

        if let Some(overlap_ratio) = scored_identity_overlap_ratio(
            self.baseline_scored_identity_counts,
            self.candidate_scored_identity_counts,
        ) {
            if overlap_ratio < policy.low_identity_overlap_ratio {
                low_reasons.push(format!(
                    "target identity shifted during candidate measurement: scored identity overlap {:.1}% is below {:.1}%",
                    overlap_ratio * 100.0,
                    policy.low_identity_overlap_ratio * 100.0
                ));
            } else if overlap_ratio < policy.medium_identity_overlap_ratio {
                medium_reasons.push(format!(
                    "scored identity overlap {:.1}% is below {:.1}%",
                    overlap_ratio * 100.0,
                    policy.medium_identity_overlap_ratio * 100.0
                ));
            }
        }

        if !low_reasons.is_empty() {
            OnlineDataQuality::Low {
                reasons: low_reasons,
            }
        } else if !medium_reasons.is_empty() {
            OnlineDataQuality::Medium {
                reasons: medium_reasons,
            }
        } else {
            OnlineDataQuality::High
        }
    }
}

fn scored_identity_overlap_ratio(
    baseline: &[ScoredIdentityCount],
    candidate: &[ScoredIdentityCount],
) -> Option<f64> {
    if baseline.is_empty() && candidate.is_empty() {
        return None;
    }

    let baseline_map = scored_identity_counts_to_map(baseline);
    let candidate_map = scored_identity_counts_to_map(candidate);
    let common = scored_identity_overlap(&baseline_map, &candidate_map, usize::min);
    let total = scored_identity_overlap(&baseline_map, &candidate_map, usize::max);

    if total == 0 {
        None
    } else {
        Some(common as f64 / total as f64)
    }
}

fn frame_count_low_reason(
    baseline_frame_count: Option<usize>,
    candidate_frame_count: Option<usize>,
    low_ratio: f64,
) -> Option<String> {
    let (baseline, candidate) = frame_count_pair(baseline_frame_count, candidate_frame_count)?;
    let min = baseline.min(candidate);
    let max = baseline.max(candidate);

    if min == 0 && max > 0 {
        return Some(format!(
            "frame data mismatch when frame-based scoring is required: one window has frames and the other has none (baseline_frame_count={} candidate_frame_count={})",
            baseline, candidate
        ));
    }

    if min > 0 {
        let ratio = max as f64 / min as f64;
        if ratio > low_ratio {
            return Some(format!(
                "frame data mismatch when frame-based scoring is required: frame count ratio {:.2} exceeds {:.2} (baseline_frame_count={} candidate_frame_count={})",
                ratio, low_ratio, baseline, candidate
            ));
        }
    }

    None
}

fn frame_count_medium_reason(
    baseline_frame_count: Option<usize>,
    candidate_frame_count: Option<usize>,
    medium_ratio: f64,
) -> Option<String> {
    let (baseline, candidate) = frame_count_pair(baseline_frame_count, candidate_frame_count)?;
    let min = baseline.min(candidate);
    let max = baseline.max(candidate);

    if min > 0 {
        let ratio = max as f64 / min as f64;
        if ratio > medium_ratio {
            return Some(format!(
                "frame count differs by more than {:.0}% but remains below low-quality threshold (baseline_frame_count={} candidate_frame_count={} ratio={:.2})",
                (medium_ratio - 1.0) * 100.0,
                baseline,
                candidate,
                ratio
            ));
        }
    }

    None
}

fn frame_count_pair(
    baseline_frame_count: Option<usize>,
    candidate_frame_count: Option<usize>,
) -> Option<(usize, usize)> {
    match (baseline_frame_count, candidate_frame_count) {
        (Some(baseline), Some(candidate)) => Some((baseline, candidate)),
        _ => None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        process_tree::TaskClass,
        tune::comparability::{ScoredIdentityCount, TaskIdentity},
    };

    fn high_input() -> OnlineDataQualityInput<'static> {
        OnlineDataQualityInput {
            scored_intervals: DEFAULT_MIN_SCORED_INTERVALS,
            scored_samples: DEFAULT_MIN_SCORED_SAMPLES,
            scored_task_count: 1,
            drop_counter_total: 0,
            target_identity_shifted: false,
            target_present: true,
            frame_data_required: false,
            frame_count: 0,
            baseline_frame_count: None,
            candidate_frame_count: None,
            baseline_scored_identity_counts: &[],
            candidate_scored_identity_counts: &[],
        }
    }

    fn identity(comm: &str) -> ScoredIdentityCount {
        ScoredIdentityCount {
            identity: TaskIdentity {
                class: TaskClass::Game,
                process_comm: "game".to_owned(),
                comm: comm.to_owned(),
                process_starttime_ticks: Some(100),
                task_starttime_ticks: Some(200),
                exe_dev: Some(1),
                exe_ino: Some(2),
            },
            count: 100,
        }
    }

    #[test]
    fn high_when_all_required_online_gates_pass() {
        let quality = OnlineDataQuality::evaluate(high_input());

        assert_eq!(quality, OnlineDataQuality::High);
        assert!(!quality.blocks_action());
    }

    #[test]
    fn default_policy_does_not_require_frame_data() {
        let policy = OnlineDataQualityPolicy::default();
        let input = high_input();

        assert_eq!(policy.frame_data_policy, DEFAULT_FRAME_DATA_POLICY);
        assert_eq!(policy.frame_data_policy, FrameDataPolicy::Advisory);
        assert_eq!(input.evaluate_with_policy(&policy), OnlineDataQuality::High);
    }

    #[test]
    fn policy_required_frame_data_without_frames_is_low_quality() {
        let policy = OnlineDataQualityPolicy {
            frame_data_policy: FrameDataPolicy::Required,
            ..OnlineDataQualityPolicy::default()
        };
        let input = high_input();

        let quality = input.evaluate_with_policy(&policy);

        assert!(quality.is_low());
        assert!(
            quality
                .reasons()
                .iter()
                .any(|reason| reason.contains("no frame data"))
        );
    }

    #[test]
    fn ignore_frame_data_policy_skips_frame_count_mismatch() {
        let mut input = high_input();
        input.frame_count = 100;
        input.baseline_frame_count = Some(100);
        input.candidate_frame_count = Some(0);
        let policy = OnlineDataQualityPolicy {
            frame_data_policy: FrameDataPolicy::Ignore,
            ..OnlineDataQualityPolicy::default()
        };
        assert_eq!(input.evaluate_with_policy(&policy), OnlineDataQuality::High);
    }
    #[test]
    fn low_when_scored_intervals_are_below_policy_minimum() {
        let mut input = high_input();
        input.scored_intervals = DEFAULT_MIN_SCORED_INTERVALS - 1;

        let quality = OnlineDataQuality::evaluate(input);

        assert!(quality.is_low());
        assert!(
            quality
                .reasons()
                .iter()
                .any(|reason| reason.contains("fewer than min_scored_intervals"))
        );
    }

    #[test]
    fn low_when_scored_samples_are_below_policy_minimum() {
        let mut input = high_input();
        input.scored_samples = DEFAULT_MIN_SCORED_SAMPLES - 1;

        let quality = OnlineDataQuality::evaluate(input);

        assert!(quality.is_low());
        assert!(
            quality
                .reasons()
                .iter()
                .any(|reason| reason.contains("fewer than min_scored_samples"))
        );
    }

    #[test]
    fn low_when_scored_task_count_is_zero() {
        let mut input = high_input();
        input.scored_task_count = 0;

        let quality = OnlineDataQuality::evaluate(input);

        assert!(quality.is_low());
        assert!(
            quality
                .reasons()
                .iter()
                .any(|reason| reason == "zero scored tasks")
        );
    }

    #[test]
    fn low_when_drop_counters_exceed_policy_max() {
        let mut input = high_input();
        input.drop_counter_total = 1;

        let quality = OnlineDataQuality::evaluate(input);

        assert!(quality.is_low());
        assert!(
            quality
                .reasons()
                .iter()
                .any(|reason| reason.contains("drop counters above policy max"))
        );
    }

    #[test]
    fn low_when_target_identity_shifted() {
        let mut input = high_input();
        input.target_identity_shifted = true;

        let quality = OnlineDataQuality::evaluate(input);

        assert!(quality.is_low());
        assert!(
            quality
                .reasons()
                .iter()
                .any(|reason| reason
                    .contains("target identity shifted during candidate measurement"))
        );
    }

    #[test]
    fn low_when_target_disappeared() {
        let mut input = high_input();
        input.target_present = false;

        let quality = OnlineDataQuality::evaluate(input);

        assert!(quality.is_low());
        assert!(
            quality
                .reasons()
                .iter()
                .any(|reason| reason == "target disappeared")
        );
    }

    #[test]
    fn low_when_frame_data_required_but_missing() {
        let mut input = high_input();
        input.frame_data_required = true;
        input.frame_count = 0;

        let quality = OnlineDataQuality::evaluate(input);

        assert!(quality.is_low());
        assert!(
            quality
                .reasons()
                .iter()
                .any(|reason| reason.contains("no frame data"))
        );
    }

    #[test]
    fn low_when_frame_count_has_zero_nonzero_mismatch() {
        let mut input = high_input();
        input.frame_data_required = true;
        input.frame_count = 100;
        input.baseline_frame_count = Some(100);
        input.candidate_frame_count = Some(0);

        let quality = OnlineDataQuality::evaluate(input);

        assert!(quality.is_low());
        assert!(
            quality
                .reasons()
                .iter()
                .any(|reason| reason.contains("one window has frames and the other has none"))
        );
    }

    #[test]
    fn low_when_scored_identity_overlap_is_below_tune_threshold() {
        let baseline = vec![identity("render")];
        let candidate = vec![identity("worker")];

        let input = OnlineDataQualityInput {
            baseline_scored_identity_counts: &baseline,
            candidate_scored_identity_counts: &candidate,
            ..high_input()
        };

        let quality = OnlineDataQuality::evaluate(input);

        assert!(quality.is_low());
        assert!(
            quality
                .reasons()
                .iter()
                .any(|reason| reason.contains("scored identity overlap"))
        );
    }

    #[test]
    fn medium_when_frame_count_differs_but_not_enough_for_low() {
        let mut input = high_input();
        input.baseline_frame_count = Some(100);
        input.candidate_frame_count = Some(130);

        let quality = OnlineDataQuality::evaluate(input);

        assert!(quality.is_medium());
        assert!(
            quality
                .reasons()
                .iter()
                .any(|reason| reason.contains("frame count differs"))
        );
    }

    #[test]
    fn reason_codes_are_stable_and_deduplicated_for_low_quality() {
        let mut input = high_input();
        input.scored_intervals = 0;
        input.scored_samples = 0;
        input.drop_counter_total = 7;
        input.target_present = false;

        let quality = OnlineDataQuality::evaluate(input);

        assert_eq!(
            quality.reason_code_strings(),
            vec![
                "insufficient_samples".to_owned(),
                "target_missing".to_owned(),
                "drop_counters_high".to_owned()
            ]
        );
        assert_eq!(
            quality.primary_reason_code(),
            Some(OnlineDataQualityReasonCode::InsufficientSamples)
        );
    }

    #[test]
    fn reason_codes_cover_workload_frame_focus_and_thermal_reasons() {
        let quality = OnlineDataQuality::Low {
            reasons: vec![
                "target identity shifted during candidate measurement".to_owned(),
                "frame data mismatch when frame-based scoring is required: no frame data"
                    .to_owned(),
                "focus confidence below policy threshold".to_owned(),
                "thermal degraded".to_owned(),
            ],
        };

        assert_eq!(
            quality.reason_code_strings(),
            vec![
                "focus_low_confidence".to_owned(),
                "workload_changed".to_owned(),
                "thermal_degraded".to_owned(),
                "frame_data_invalid".to_owned()
            ]
        );
    }

    #[test]
    fn medium_quality_reason_codes_include_frame_count_mismatch() {
        let mut input = high_input();
        input.baseline_frame_count = Some(100);
        input.candidate_frame_count = Some(130);

        let quality = OnlineDataQuality::evaluate(input);

        assert_eq!(
            quality.reason_code_strings(),
            vec!["frame_count_mismatch".to_owned()]
        );
    }
}
