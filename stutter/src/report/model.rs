use std::collections::BTreeMap;

use serde::Serialize;

use crate::{
    diagnosis::DiagnosisThresholdDoc,
    process_tree::TaskClass,
    recorder::{FrameEvent, GpuSample, SessionFile, SpikeEvent},
    session_io::{self},
};

#[derive(Debug, Clone, Serialize)]
pub struct SpikeClusterAnalysis {
    pub(crate) source: SpikeClusterSource,
    pub(crate) source_count: usize,
    pub(crate) clusters: Vec<crate::spike::SpikeCluster>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct RuntimeSliceAnalysisSummary {
    pub available: bool,
    pub sample_count: usize,
    pub missing_reason: Option<String>,
    pub data_quality_notes: Vec<String>,
    pub source_counts: BTreeMap<String, usize>,
    pub high_runtime_threads: Vec<RuntimeThreadSummary>,
    pub high_wait_threads: Vec<RuntimeThreadSummary>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeThreadSummary {
    pub task: u32,
    pub process_pid: Option<u32>,
    pub class: TaskClass,
    pub comm: String,
    pub process_comm: String,
    pub max_runtime_ratio: f64,
    pub max_wait_ratio: Option<f64>,
    pub max_runtime_delta_ns: u64,
    pub max_runqueue_wait_delta_ns: Option<u64>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportAnalysisJson {
    pub session: SessionFile,
    pub cluster_analysis: SpikeClusterAnalysis,
    pub frame_diagnoses: Vec<crate::diagnosis::FrameDiagnosis>,
    pub frame_pacing: FramePacingSummary,
    pub pressure_timeline: PressureTimelineSummary,
    pub runtime_slices: RuntimeSliceAnalysisSummary,
    pub diagnosis_thresholds: Vec<DiagnosisThresholdDoc>,
    pub artifacts_summary: ArtifactsSummary,
    pub data_quality: DataQualitySummary,
    pub focus_summary: FocusReportSummary,
    pub foreground_summary: ForegroundReportSummary,
    pub kms_timing: KmsTimingSummary,
    pub drm_fence_timing: DrmFenceTimingSummary,
    pub cross_gpu_fence: CrossGpuFenceSummary,
    pub wayland_presentation: WaylandPresentationSummary,
    pub direct_scanout: DirectScanoutSummary,
    pub dmabuf_path: DmaBufPathSummary,
    pub gpu_engine_activity: GpuEngineActivitySummary,
    pub display_path_diagnosis: DisplayPathDiagnosisSummary,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct FramePacingSummary {
    pub frame_count: usize,
    pub median_frametime_ms: Option<f64>,
    pub p95_frametime_ms: Option<f64>,
    pub p99_frametime_ms: Option<f64>,
    pub max_frametime_ms: Option<f64>,
    pub outlier_count: usize,
    pub outliers: Vec<FrameOutlierView>,
    pub compositor_cluster_count: usize,
    pub game_cluster_count: usize,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FrameOutlierView {
    pub elapsed_ms: u64,
    pub frametime_ms: f64,
    pub over_median_ratio: Option<f64>,
    pub nearest_cluster_delta_ms: Option<i64>,
    pub nearest_cluster_cause: Option<String>,
    pub nearest_cluster_anchor_class: Option<TaskClass>,
    pub nearest_cluster_anchor_comm: Option<String>,
    pub foreground_pid: Option<u32>,
    pub foreground_app_id: Option<String>,
    pub foreground_class: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskHtmlRow {
    pub task: u32,
    pub active: bool,
    pub class: TaskClass,
    pub process_pid: Option<u32>,
    pub process_comm: String,
    pub comm: String,
    pub samples: u64,
    pub spike_count: u64,
    pub max_latency_ms: f64,
    pub p99_latency_ms: f64,
    pub avg_latency_ms: f64,
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct HtmlChartArtifacts {
    pub gpu_samples: Vec<GpuSample>,
    pub frame_events: Vec<FrameEvent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HtmlReportModel {
    pub session: SessionFile,
    pub data_quality: DataQualitySummary,
    pub cluster_analysis: SpikeClusterAnalysis,
    pub frame_diagnoses: Vec<crate::diagnosis::FrameDiagnosis>,
    pub frame_pacing: FramePacingSummary,
    pub pressure_timeline: PressureTimelineSummary,
    pub runtime_slices: RuntimeSliceAnalysisSummary,
    pub artifacts_summary: ArtifactsSummary,
    pub focus_summary: FocusReportSummary,
    pub foreground_summary: ForegroundReportSummary,
    pub display_path_diagnosis: DisplayPathDiagnosisSummary,
    pub top_tasks_by_max: Vec<TaskHtmlRow>,
    pub top_tasks_by_p99: Vec<TaskHtmlRow>,
    pub spike_density: Vec<SpikeDensityBucket>,

    pub top_limit: usize,
    pub spike_events: Option<Vec<SpikeEvent>>,
    pub chart_artifacts: HtmlChartArtifacts,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub legacy_text_report: Option<String>,
}

// Old RunArtifacts removed in favor of session_io::RunArtifacts

pub(crate) struct ReportInputModel {
    artifacts: session_io::RunArtifacts,
}

impl ReportInputModel {
    pub(crate) fn from_artifacts(artifacts: session_io::RunArtifacts) -> Self {
        Self { artifacts }
    }

    pub(crate) fn into_artifacts(self) -> session_io::RunArtifacts {
        self.artifacts
    }
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct SpikeClusterCandidate {
    pub(crate) start_idx: usize,
    pub(crate) end_idx: usize,
    pub(crate) distinct_tasks: usize,
    pub(crate) min_switch_ns: u64,
    pub(crate) max_switch_ns: u64,
    pub(crate) max_latency_ns: u64,
}

impl Ord for SpikeClusterCandidate {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (
            self.distinct_tasks,
            self.max_latency_ns,
            self.end_idx.saturating_sub(self.start_idx),
            std::cmp::Reverse(self.min_switch_ns),
        )
            .cmp(&(
                other.distinct_tasks,
                other.max_latency_ns,
                other.end_idx.saturating_sub(other.start_idx),
                std::cmp::Reverse(other.min_switch_ns),
            ))
    }
}

impl PartialOrd for SpikeClusterCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

pub(crate) struct ReportBuildResult {
    pub(crate) analysis: ReportAnalysisJson,
    pub(crate) artifacts: session_io::RunArtifacts,
}

// Keep near-term report improvements view-only: derive new report fields from
// existing RunArtifacts/session data. Do not require new live probes here.
pub use stutter_report::model::*;
