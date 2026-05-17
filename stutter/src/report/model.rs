use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    diagnosis::{DiagnosisThresholdDoc, FrameDiagnosis},
    process_tree::TaskClass,
    recorder::{FrameEvent, GpuSample, SessionFile, SpikeEvent},
    session_io::{self},
    spike::SpikeCluster,
};

#[derive(Debug, Clone, Serialize)]
pub struct SpikeDensityBucket {
    pub start_ms: u64,
    pub end_ms: u64,
    pub count: u64,
    pub max_latency_ms: f64,
    pub p99_latency_ms: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum SpikeClusterSource {
    SpikeEvents,
    TopSpikesFallback,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpikeClusterAnalysis {
    pub(crate) source: SpikeClusterSource,
    pub(crate) source_count: usize,
    pub(crate) clusters: Vec<SpikeCluster>,
}

#[derive(Clone, Debug, Serialize)]
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

#[derive(Clone, Debug, Serialize)]
pub struct DataQualitySummary {
    pub level: DataQualityLevel,
    pub reasons: Vec<String>,
    pub missing_optional_files: Vec<String>,
    pub validation_errors: Vec<String>,
    pub validation_warnings: Vec<String>,
    pub schema_version: u32,
    pub expected_schema_version: u32,
    pub event_stream_write_errors: u64,
    pub spike_events_truncated: bool,
    pub spike_events_retained_count: u64,
    pub spike_events_dropped_count: u64,
    pub interval_record_count: u64,
    pub active_target_pids_count: u64,
    pub drop_counters_nonzero: bool,
    pub percentile_scope_counts: BTreeMap<String, u64>,
    pub block_io_correlation_basis: String,
    pub block_io_correlation_confidence: String,
    pub block_io_correlation_warning: Option<String>,
    pub frame_timestamp_alignment: String,
    pub cpu_perf_requested: bool,
    pub cpu_perf_open_errors: u64,
    pub cpu_perf_read_errors: u64,
    pub cpu_perf_skipped_tasks: u64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DataQualityLevel {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct ForegroundReportSummary {
    pub enabled: bool,
    pub source: Option<String>,
    pub final_pid: Option<u32>,
    pub final_app_id: Option<String>,
    pub final_class: Option<String>,
    pub final_title: Option<String>,
    pub final_workspace: Option<String>,
    pub event_count: u64,
    pub confidence: Option<f32>,
    pub provider_status: Option<String>,
    pub reasons: Vec<String>,
}

impl ForegroundReportSummary {
    pub fn is_visible(&self) -> bool {
        self.enabled
            || self.source.is_some()
            || self.final_pid.is_some()
            || self.final_app_id.is_some()
            || self.final_class.is_some()
            || self.final_title.is_some()
            || self.final_workspace.is_some()
            || self.event_count > 0
            || self.confidence.is_some()
            || self.provider_status.is_some()
            || !self.reasons.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct FocusReportSummary {
    pub mode: Option<String>,
    pub final_focus: Option<String>,
    pub display_name: Option<String>,
    pub situation: Option<String>,
    pub confidence: Option<f32>,
    pub score: Option<f32>,
    pub roots: Vec<u32>,
    pub member_pids: Vec<u32>,
    pub focus_switches: u64,
    pub reasons: Vec<String>,
}

impl FocusReportSummary {
    pub fn is_visible(&self) -> bool {
        self.mode.is_some()
            || self.final_focus.is_some()
            || self.situation.is_some()
            || self.confidence.is_some()
            || !self.roots.is_empty()
            || self.focus_switches > 0
            || !self.reasons.is_empty()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportAnalysisJson {
    pub session: SessionFile,
    pub cluster_analysis: SpikeClusterAnalysis,
    pub frame_diagnoses: Vec<FrameDiagnosis>,
    pub frame_pacing: FramePacingSummary,
    pub pressure_timeline: PressureTimelineSummary,
    pub runtime_slices: RuntimeSliceAnalysisSummary,
    pub diagnosis_thresholds: Vec<DiagnosisThresholdDoc>,
    pub artifacts_summary: ArtifactsSummary,
    pub data_quality: DataQualitySummary,
    pub focus_summary: FocusReportSummary,
    pub foreground_summary: ForegroundReportSummary,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct PressureTimelineSummary {
    pub sample_count: usize,
    pub max_cpu_some: f64,
    pub max_mem_some: Option<f64>,
    pub max_mem_full: Option<f64>,
    pub max_io_some: Option<f64>,
    pub max_io_full: Option<f64>,
    pub windows: Vec<PressureWindow>,
    pub peak_windows: Vec<PressurePeakWindow>,
    pub pressure_notes: Vec<String>,
    pub coverage: PressureTimelineCoverage,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct PressureTimelineCoverage {
    pub interval_records_loaded: usize,
    pub has_cpu_psi: bool,
    pub has_mem_psi: bool,
    pub has_io_psi: bool,
    pub has_near_spike_windows: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PressurePeakWindow {
    pub elapsed_ms: u64,
    pub pressure_kind: PressureKind,
    pub value: f64,
    pub near_spike: bool,
}

#[derive(Debug, Clone, Serialize)]
pub enum PressureKind {
    CpuSome,
    MemSome,
    MemFull,
    IoSome,
    IoFull,
}

#[derive(Debug, Clone, Serialize)]
pub struct PressureWindow {
    pub elapsed_ms: u64,
    pub cpu_some: f64,
    pub mem_some: Option<f64>,
    pub mem_full: Option<f64>,
    pub io_some: Option<f64>,
    pub io_full: Option<f64>,
    pub near_spike: bool,
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
    pub frame_diagnoses: Vec<FrameDiagnosis>,
    pub frame_pacing: FramePacingSummary,
    pub pressure_timeline: PressureTimelineSummary,
    pub runtime_slices: RuntimeSliceAnalysisSummary,
    pub artifacts_summary: ArtifactsSummary,
    pub focus_summary: FocusReportSummary,
    pub foreground_summary: ForegroundReportSummary,
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
#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegressionMetric {
    P99,
    Max,
}

pub(crate) struct CorrelationCtx<'a> {
    pub(crate) output: &'a mut String,
    pub(crate) clusters: &'a [SpikeCluster],
    pub(crate) top: usize,
}
