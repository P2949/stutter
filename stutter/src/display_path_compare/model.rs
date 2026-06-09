use std::path::PathBuf;

use serde::Serialize;

#[derive(Debug, Clone)]
pub struct DisplayPathCompareInput {
    pub baseline: PathBuf,
    pub test: PathBuf,
    pub json: bool,
    pub strict: bool,
    pub expect: Option<DisplayPathExpectation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[value(rename_all = "kebab-case")]
pub enum DisplayPathExpectation {
    DirectToOffload,
    OffloadToDirect,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayPathVerdictReason {
    SameScanoutGpu,
    CrossGpuFenceDetected,
    IgpuEngineActive,
    TopologyMismatch,
    MissingEvidence,
}

impl DisplayPathVerdictReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SameScanoutGpu => "same_scanout_gpu",
            Self::CrossGpuFenceDetected => "cross_gpu_fence_detected",
            Self::IgpuEngineActive => "igpu_engine_active",
            Self::TopologyMismatch => "topology_mismatch",
            Self::MissingEvidence => "missing_evidence",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DisplayPathCompareOutput {
    pub display_path_cost: DisplayPathCostSummary,
}

#[derive(Debug, Clone, Serialize)]
pub struct DisplayPathCostSummary {
    pub verdict: String,
    pub verdict_reason: DisplayPathVerdictReason,
    pub confidence_score: f64,
    pub evidence: Vec<String>,
    pub missing_evidence: Vec<String>,
    pub baseline_label: Option<String>,
    pub test_label: Option<String>,
    pub comparison_quality: String,
    pub comparison_warnings: Vec<String>,
    pub baseline_fps: Option<f64>,
    pub test_fps: Option<f64>,
    pub avg_fps_delta: Option<f64>,
    pub avg_fps_delta_percent: Option<f64>,
    pub median_frame_delta_ms: Option<f64>,
    pub p95_frame_delta_ms: Option<f64>,
    pub p99_frame_delta_ms: Option<f64>,
    pub max_frame_delta_ms: Option<f64>,
    pub kms_median_delta_ms: Option<f64>,
    pub kms_p95_delta_ms: Option<f64>,
    pub kms_p99_delta_ms: Option<f64>,
    pub kms_long_flip_delta_count: Option<i64>,
    pub display_side_fence_wait_p99_delta_ms: Option<f64>,
    pub render_side_fence_wait_p99_delta_ms: Option<f64>,
    pub cross_gpu_candidate_count_delta: i64,
    pub commit_to_present_p99_delta_ms: Option<f64>,
    pub discarded_frame_delta: i64,
    pub zero_copy_ratio_delta: Option<f64>,
    pub direct_scanout_status_delta: Option<String>,
    pub igpu_engine_activity_delta: Option<f64>,
    pub dmabuf_copy_required_delta: Option<i64>,
    pub fence_component_delta_ms: Option<f64>,
    pub kms_component_delta_ms: Option<f64>,
    pub wayland_component_delta_ms: Option<f64>,
    pub compositor_component_delta_ms: Option<f64>,
    pub game_cluster_count_delta: i64,
    pub compositor_cluster_count_delta: i64,
    pub scheduler_p99_delta_ms: Option<f64>,
    pub likely_causes: Vec<String>,
    pub notes: Vec<String>,
}
