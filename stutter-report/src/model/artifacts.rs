use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpikeDensityBucket {
    pub start_ms: u64,
    pub end_ms: u64,
    pub count: u64,
    pub max_latency_ms: f64,
    pub p99_latency_ms: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SpikeClusterSource {
    SpikeEvents,
    TopSpikesFallback,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactsSummary {
    pub spike_count: u64,
    pub frame_count: u64,
    pub irq_event_count: u64,
    pub gpu_sample_count: u64,
    pub frame_event_count: u64,
    pub migration_event_count: u64,
    pub cpu_freq_sample_count: u64,
    pub block_io_event_count: u64,
    pub runtime_slice_count: u64,
    pub interval_record_count: u64,
    pub scx_event_count: u64,
    pub focus_event_count: u64,
    pub foreground_event_count: u64,
    pub kms_flip_event_count: u64,
    pub drm_fence_event_count: u64,
    pub wayland_presentation_event_count: u64,
    pub dmabuf_event_count: u64,
    pub gpu_engine_sample_count: u64,
}
