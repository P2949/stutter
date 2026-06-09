use super::{
    frame::{frame_count_low_reason, frame_count_medium_reason},
    policy::OnlineDataQualityPolicy,
    reason::OnlineDataQuality,
    score::scored_identity_overlap_ratio,
};
use crate::tune::comparability::ScoredIdentityCount;

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
