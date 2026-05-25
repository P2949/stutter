use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};

use stutter_core::{ids::RunId, paths::LogicalPath, units::UnixNanoseconds};

/// Minimal report domain model placeholder for future report migration.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReportModel {
    pub run_id: Option<RunId>,
    pub source_path: Option<LogicalPath>,
    pub generated_at_unix_nanos: Option<UnixNanoseconds>,
    pub header: Option<ReportHeaderSummary>,
    pub data_quality: Option<DataQualitySummary>,
    #[serde(default)]
    pub clusters: Vec<SpikeCluster>,
    #[serde(default)]
    pub frames: Vec<FrameDiagnosis>,
    pub correlations: Option<TextReportCorrelationSections>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReportHeaderSummary {
    pub file_path: String,
    pub schema_version: u32,
    pub expected_schema_version: u32,
    pub run_name: String,
    pub duration_ms: u64,
    pub stop_reason: String,
    pub manual_pids: Vec<u32>,
    pub tree_roots: Vec<u32>,
    pub include_comm: Vec<String>,
    pub exclude_comm: Vec<String>,
    pub event_stream_warning: Option<String>,
    pub watch_process: String,
    pub persistent: bool,
    pub csv_stream: String,
    pub active_target_pids_count: u64,
}

impl ReportModel {
    pub fn new() -> Self {
        Self {
            run_id: None,
            source_path: None,
            generated_at_unix_nanos: None,
            header: None,
            data_quality: None,
            clusters: Vec::new(),
            frames: Vec::new(),
            correlations: None,
        }
    }

    pub fn with_run_id(mut self, run_id: RunId) -> Self {
        self.run_id = Some(run_id);
        self
    }

    pub fn with_source_path(mut self, source_path: LogicalPath) -> Self {
        self.source_path = Some(source_path);
        self
    }

    pub fn with_generated_at_unix_nanos(
        mut self,
        generated_at_unix_nanos: UnixNanoseconds,
    ) -> Self {
        self.generated_at_unix_nanos = Some(generated_at_unix_nanos);
        self
    }

    pub fn run_id(&self) -> Option<&RunId> {
        self.run_id.as_ref()
    }

    pub fn source_path(&self) -> Option<&LogicalPath> {
        self.source_path.as_ref()
    }

    pub fn generated_at_unix_nanos(&self) -> Option<UnixNanoseconds> {
        self.generated_at_unix_nanos
    }
}


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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct KmsTimingSummary {
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DataQualitySummary {
    pub level: DataQualityLevel,
    pub reasons: Vec<String>,
    pub missing_optional_files: Vec<String>,
    pub validation_errors: Vec<String>,
    pub validation_warnings: Vec<String>,
    pub probe_activation_warnings: Vec<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ForegroundReportSummary {
    pub enabled: bool,
    pub source: Option<String>,
    pub final_pid: Option<u32>,
    pub final_app_id: Option<String>,
    pub final_class: Option<String>,
    pub final_title: Option<String>,
    pub final_window_id: Option<String>,
    pub final_workspace: Option<String>,
    pub event_count: u64,
    pub confidence: Option<f32>,
    pub provider_status: Option<String>,
    pub stale_ms: Option<u64>,
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
            || self.final_window_id.is_some()
            || self.final_workspace.is_some()
            || self.event_count > 0
            || self.confidence.is_some()
            || self.provider_status.is_some()
            || self.stale_ms.is_some()
            || !self.reasons.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PressureTimelineCoverage {
    pub interval_records_loaded: usize,
    pub has_cpu_psi: bool,
    pub has_mem_psi: bool,
    pub has_io_psi: bool,
    pub has_near_spike_windows: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PressurePeakWindow {
    pub elapsed_ms: u64,
    pub pressure_kind: PressureKind,
    pub value: f64,
    pub near_spike: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PressureKind {
    CpuSome,
    MemSome,
    MemFull,
    IoSome,
    IoFull,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PressureWindow {
    pub elapsed_ms: u64,
    pub cpu_some: f64,
    pub mem_some: Option<f64>,
    pub mem_full: Option<f64>,
    pub io_some: Option<f64>,
    pub io_full: Option<f64>,
    pub near_spike: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TextReportCorrelationSections {
    pub sections: Vec<TextReportCorrelationSection>,
}

impl TextReportCorrelationSections {
    pub fn new() -> Self {
        Self {
            sections: Vec::new(),
        }
    }

    pub fn push_section(&mut self, section: TextReportCorrelationSection) {
        self.sections.push(section);
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextReportCorrelationSection {
    pub title: String,
    pub lines: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnosis {
    pub primary: Option<DiagnosisPrimary>,
    pub candidates: Vec<DiagnosisCandidate>,
    pub missing_evidence: Vec<String>,
    pub candidate_rejections: Vec<DiagnosisRejection>,
    pub secondary_causes: Vec<String>,
    pub report_summary: String,
}

impl Diagnosis {
    pub fn report_summary(&self) -> &str {
        &self.report_summary
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiagnosisPrimary {
    pub cause: String,
    pub confidence: String,
    pub score: f32,
    pub evidence: Vec<DiagnosisEvidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiagnosisCandidate {
    pub cause: String,
    pub confidence: String,
    pub score: f32,
    pub evidence: Vec<DiagnosisEvidence>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiagnosisRejection {
    pub cause: String,
    pub score: f32,
    pub confidence: String,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiagnosisEvidence {
    pub kind: String,
    pub strength: f32,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpikeClusterAnalysis {
    pub source: SpikeClusterSource,
    pub source_count: usize,
    pub clusters: Vec<SpikeCluster>,
}

pub const MIN_CLUSTER_TASKS: usize = 3;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpikeCluster {
    pub points: Vec<SpikePoint>,
    pub distinct_tasks: usize,
    pub min_switch_ns: u64,
    pub max_switch_ns: u64,
    pub max_latency_ns: u64,
    pub diagnosis: Option<Diagnosis>,
    pub wake_graph: Vec<WakeGraphEdge>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WakeGraphEdge {
    pub waker_tid: u32,
    pub waker_comm: String,
    pub wakee_tid: u32,
    pub wakee_comm: String,
    pub count: u64,
    pub max_latency_ns: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpikePoint {
    pub task: u32,
    pub class: String,
    pub process_pid: Option<u32>,
    pub comm: String,
    pub cpu: u32,
    pub wakeup_target_cpu: u32,
    pub latency_ns: u64,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
    pub target_pending_wakeups: u32,
    pub observed_runnable_depth: u32,
    pub switch_prev_pid: u32,
    pub switch_prev_state: i64,
    pub switch_prev_state_label: String,
    pub scx_ops: Option<String>,
    pub primary_cause: Option<String>,
    pub cause_tags: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FrameDiagnosis {
    pub frame_elapsed_ms: u64,
    pub frametime_ms: f64,
    pub diagnosis: Diagnosis,
}

impl TextReportCorrelationSection {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            lines: Vec::new(),
        }
    }

    pub fn push_line(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegressionMetric {
    P99,
    Max,
}


#[cfg(test)]
mod tests {
    use stutter_core::{ids::RunId, paths::LogicalPath, units::UnixNanoseconds};

    use super::ReportModel;

    #[test]
    fn report_model_tracks_minimal_identity_source_and_generation_time() {
        let model = ReportModel::new()
            .with_run_id(RunId::new("run-001"))
            .with_source_path(LogicalPath::new("runs/run-001"))
            .with_generated_at_unix_nanos(UnixNanoseconds::new(123));

        assert_eq!(
            model.run_id().map(|run_id| run_id.as_str()),
            Some("run-001")
        );
        assert_eq!(
            model.source_path().map(|path| path.as_str()),
            Some("runs/run-001")
        );
        assert_eq!(
            model
                .generated_at_unix_nanos()
                .map(UnixNanoseconds::as_u128),
            Some(123)
        );
    }
}