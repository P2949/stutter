use crate::autotune::quality::OnlineDataQuality;

#[derive(Debug, Clone)]
pub struct RollingWindowScore {
    pub duration_ms: u64,
    pub interval_count: usize,
    pub scored_task_count: usize,
    pub scored_samples: u64,
    pub diagnostic_score_total: u64,
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
    pub max_latency_ns: u64,
    pub frame_count: usize,
    pub frame_p99_ms: f64,
    pub frame_max_ms: f64,
    pub dropped_invalid_frames: u64,
    pub data_quality: OnlineDataQuality,
}
