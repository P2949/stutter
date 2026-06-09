use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceQuality {
    Direct,
    Derived,
    Approximate { reason: String },
    Missing { reason: String },
}

impl Default for EvidenceQuality {
    fn default() -> Self {
        Self::Missing {
            reason: "not evaluated".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KmsTimingSummary {
    #[serde(default)]
    pub evidence_quality: EvidenceQuality,
    pub event_count: usize,
    pub duration_count: usize,
    pub median_flip_ms: Option<f64>,
    pub p95_flip_ms: Option<f64>,
    pub p99_flip_ms: Option<f64>,
    pub max_flip_ms: Option<f64>,
    pub scanout_window_estimate: ScanoutWindowEstimate,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScanoutWindowEstimate {
    #[serde(default)]
    pub evidence_quality: EvidenceQuality,
    pub estimate_count: usize,
    pub refresh_period_ns: Option<u64>,
    pub refresh_period_ms: Option<f64>,
    pub first_estimated_top_of_screen_visible_ns: Option<u64>,
    pub first_estimated_bottom_of_screen_visible_ns: Option<u64>,
    pub last_estimated_top_of_screen_visible_ns: Option<u64>,
    pub last_estimated_bottom_of_screen_visible_ns: Option<u64>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DrmFenceTimingSummary {
    #[serde(default)]
    pub evidence_quality: EvidenceQuality,
    pub event_count: usize,
    pub wait_interval_count: usize,
    pub median_wait_ms: Option<f64>,
    pub p95_wait_ms: Option<f64>,
    pub p99_wait_ms: Option<f64>,
    pub max_wait_ms: Option<f64>,
    pub render_gpu_wait_count: usize,
    pub display_gpu_wait_count: usize,
    pub cross_gpu_candidate_count: usize,
    pub waits_near_frame_outliers: usize,
    pub waits_near_kms_delays: usize,
    pub top_waits: Vec<DrmFenceWaitSummary>,
    pub notes: Vec<String>,
    pub confidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DrmFenceWaitSummary {
    pub elapsed_ms: u64,
    pub duration_ms: Option<f64>,
    pub source: String,
    pub gpu_role: Option<String>,
    pub context: Option<u64>,
    pub seqno: Option<u64>,
    pub correlation_basis: String,
    pub confidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CrossGpuFenceSummary {
    #[serde(default)]
    pub evidence_quality: EvidenceQuality,
    pub candidate_count: usize,
    pub high_confidence_count: usize,
    pub display_side_wait_count: usize,
    pub render_side_wait_count: usize,
    pub waits_near_frame_outliers: usize,
    pub waits_near_kms_delays: usize,
    pub p95_display_wait_ms: Option<f64>,
    pub p99_display_wait_ms: Option<f64>,
    pub top_candidates: Vec<CrossGpuFenceCandidate>,
    pub confidence: String,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CrossGpuFenceCandidate {
    pub elapsed_ms: u64,
    pub duration_ms: Option<f64>,
    pub wait_start_ns: Option<u64>,
    pub wait_done_ns: Option<u64>,
    pub signal_ns: Option<u64>,
    pub importer_driver: Option<String>,
    pub exporter_driver: Option<String>,
    pub context: Option<u64>,
    pub seqno: Option<u64>,
    pub timeline_hash: Option<u64>,
    pub near_frame_outlier: bool,
    pub near_kms_delay: bool,
    pub confidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WaylandPresentationSummary {
    #[serde(default)]
    pub evidence_quality: EvidenceQuality,
    pub event_count: usize,
    pub presented_count: usize,
    pub discarded_count: usize,
    pub zero_copy_count: usize,
    pub zero_copy_ratio: Option<f64>,
    pub source_counts: BTreeMap<String, usize>,
    pub surface_role_counts: BTreeMap<String, usize>,
    pub median_commit_to_present_ms: Option<f64>,
    pub p95_commit_to_present_ms: Option<f64>,
    pub p99_commit_to_present_ms: Option<f64>,
    pub max_commit_to_present_ms: Option<f64>,
    pub delays_near_frame_outliers: usize,
    pub delays_near_kms_delays: usize,
    pub compositor_queue_candidate_count: usize,
    pub outputs_seen: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DirectScanoutSummary {
    #[serde(default)]
    pub evidence_quality: EvidenceQuality,
    pub status: String,
    pub confidence: String,
    pub zero_copy_ratio: Option<f64>,
    pub direct_scanout_event_count: usize,
    pub composited_event_count: usize,
    pub blocking_reasons: Vec<String>,
    pub evidence: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DmaBufPathSummary {
    #[serde(default)]
    pub evidence_quality: EvidenceQuality,
    pub event_count: usize,
    pub linear_count: usize,
    pub scanout_capable_count: usize,
    pub copy_required_count: usize,
    pub modifier_mismatch_count: usize,
    pub cross_gpu_import_count: usize,
    pub top_reasons: BTreeMap<String, usize>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GpuEngineActivitySummary {
    #[serde(default)]
    pub evidence_quality: EvidenceQuality,
    pub sample_count: usize,
    pub active_sample_count: usize,
    pub engine_counts: BTreeMap<String, usize>,
    pub driver_counts: BTreeMap<String, usize>,
    pub igpu_render_activity_near_outliers: usize,
    pub igpu_blitter_activity_near_outliers: usize,
    pub amdgpu_gfx_activity_near_outliers: usize,
    pub max_igpu_render_busy_percent: Option<f64>,
    pub max_igpu_blitter_busy_percent: Option<f64>,
    pub max_amdgpu_gfx_busy_percent: Option<f64>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DisplayPathDiagnosisSummary {
    pub verdict: String,
    pub suspicion_score: f64,
    pub confidence: String,

    pub render_gpu: Option<String>,
    pub scanout_gpu: Option<String>,
    pub connector: Option<String>,
    pub is_cross_gpu: Option<bool>,

    pub direct_scanout: DirectScanoutSummary,
    pub cross_gpu_fence: CrossGpuFenceSummary,
    pub dmabuf_path: Option<DmaBufPathSummary>,
    pub gpu_engine_activity: Option<GpuEngineActivitySummary>,

    pub render_component: DisplayPathComponent,
    pub fence_component: DisplayPathComponent,
    pub kms_component: DisplayPathComponent,
    pub wayland_component: DisplayPathComponent,
    pub compositor_component: DisplayPathComponent,

    pub evidence: Vec<String>,
    pub missing_evidence: Vec<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DisplayPathComponent {
    pub status: String,
    pub score: f64,
    pub evidence: Vec<String>,
}
