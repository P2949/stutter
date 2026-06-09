pub const DEFAULT_MIN_SCORED_INTERVALS: usize = 5;
pub const DEFAULT_MIN_SCORED_SAMPLES: u64 = 100;
pub const DEFAULT_MAX_DROP_COUNTER_TOTAL: u64 = 0;
pub const DEFAULT_FRAME_DATA_POLICY: FrameDataPolicy = FrameDataPolicy::Advisory;
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
