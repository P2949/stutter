use std::{
    collections::{BTreeMap, BTreeSet, BinaryHeap},
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::{
    diagnosis::{
        ClusterAnchorKind, Diagnosis, FrameDiagnosis, diagnose_cluster, select_anchor_for_diagnosis,
    },
    metrics::format_latency,
    process_tree::TaskClass,
    recorder::{
        FocusEvent, ForegroundEvent, FrameEvent, GpuSample, IntervalRecord, RecordedSpike,
        SESSION_SCHEMA_VERSION, SessionFile, SessionTask, SpikeEvent,
    },
    session_io::{self, ArtifactLoadOptions},
    summary::{self, RunDiffSummary, TaskDeltaSummary, format_latency_signed},
};

pub(crate) fn event_stream_warning(
    event_stream_write_errors: u64,
    first_event_stream_write_error: Option<&str>,
) -> Option<String> {
    if event_stream_write_errors == 0 {
        return None;
    }

    let first = first_event_stream_write_error
        .filter(|s| !s.is_empty())
        .unwrap_or("first error was not recorded");

    Some(format!(
        "WARNING: recording event streams had {event_stream_write_errors} write error(s); \
         event artifact files may be incomplete. First error: {first}"
    ))
}

const MIN_CLUSTER_TASKS: usize = 3;
const MAX_INLINE_CLUSTER_POINTS: usize = 8;
const MAX_CLUSTER_CANDIDATES: usize = 4096;
const PRESSURE_NOTE_CPU_SOME: f64 = 50.0;
const PRESSURE_NOTE_MEM_SOME: f64 = 20.0;
const PRESSURE_NOTE_MEM_FULL: f64 = 5.0;
const PRESSURE_NOTE_IO_SOME: f64 = 20.0;
const PRESSURE_NOTE_IO_FULL: f64 = 5.0;
const MAX_PRESSURE_PEAK_WINDOWS: usize = 8;

#[derive(Debug, Clone, Serialize)]
pub struct SpikeDensityBucket {
    pub start_ms: u64,
    pub end_ms: u64,
    pub count: u64,
    pub max_latency_ms: f64,
    pub p99_latency_ms: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub(crate) struct SpikePoint {
    pub(crate) task: u32,
    pub(crate) class: TaskClass,
    pub(crate) process_pid: Option<u32>,
    pub(crate) comm: String,
    pub(crate) cpu: u32,
    pub(crate) wakeup_target_cpu: u32,
    pub(crate) latency_ns: u64,
    pub(crate) wakeup_ns: u64,
    pub(crate) switch_ns: u64,
    pub(crate) target_pending_wakeups: u32,
    pub(crate) observed_runnable_depth: u32,
    pub(crate) switch_prev_pid: u32,
    pub(crate) switch_prev_state: i64,
    pub(crate) switch_prev_state_label: String,
    pub(crate) elapsed_ms: Option<u64>,
    pub(crate) scx_ops: Option<String>,
    pub(crate) scx_state: Option<String>,
    pub(crate) waker_tid: u32,
    pub(crate) waker_comm: String,
    pub(crate) cause_tags: Vec<String>,
    pub(crate) primary_cause: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub(crate) struct SpikeCluster {
    pub(crate) points: Vec<SpikePoint>,
    pub(crate) distinct_tasks: usize,
    pub(crate) min_switch_ns: u64,
    pub(crate) max_switch_ns: u64,
    pub(crate) max_latency_ns: u64,
    pub(crate) diagnosis: Option<Diagnosis>,
    pub(crate) anchor_task: Option<u32>,
    pub(crate) anchor_class: Option<TaskClass>,
    pub(crate) anchor_comm: Option<String>,
    pub(crate) anchor_kind: Option<ClusterAnchorKind>,
    pub(crate) foreground_pid: Option<u32>,
    pub(crate) foreground_app_id: Option<String>,
    pub(crate) foreground_class: Option<String>,
    pub(crate) foreground_confidence: Option<f32>,
    pub(crate) wake_graph: Vec<WakeGraphEdge>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct WakeGraphEdge {
    pub(crate) waker_tid: u32,
    pub(crate) waker_comm: String,
    pub(crate) wakee_tid: u32,
    pub(crate) wakee_comm: String,
    pub(crate) count: u64,
    pub(crate) max_latency_ns: u64,
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
    pub interval_record_count: u64,
    pub scx_event_count: u64,
    pub focus_event_count: u64,
    pub foreground_event_count: u64,
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

fn foreground_report_summary(
    session: &SessionFile,
    foreground_events: &[ForegroundEvent],
) -> ForegroundReportSummary {
    let final_event = foreground_events.last();

    let enabled = session.config.foreground_window || session.core.foreground_event_count > 0;
    let source = final_event
        .map(|event| format!("{:?}", event.source).to_ascii_lowercase())
        .or_else(|| session.core.foreground_source.clone())
        .or_else(|| {
            (!session.config.foreground_source.is_empty())
                .then(|| session.config.foreground_source.clone())
        });

    let final_pid = final_event
        .and_then(|event| event.pid)
        .or(session.core.final_foreground_pid);
    let final_app_id = final_event
        .and_then(|event| event.app_id.clone())
        .or_else(|| session.core.final_foreground_app_id.clone());
    let final_class = final_event
        .and_then(|event| event.class.clone())
        .or_else(|| session.core.final_foreground_class.clone());
    let final_title = final_event.and_then(|event| event.title.clone());
    let final_workspace = final_event.and_then(|event| event.workspace.clone());
    let confidence = final_event.map(|event| event.confidence);
    let provider_status =
        final_event.map(|event| format!("{:?}", event.status).to_ascii_lowercase());
    let reasons = final_event
        .map(|event| vec![event.reason.clone()])
        .unwrap_or_default();

    ForegroundReportSummary {
        enabled,
        source,
        final_pid,
        final_app_id,
        final_class,
        final_title,
        final_workspace,
        event_count: session
            .core
            .foreground_event_count
            .max(foreground_events.len() as u64),
        confidence,
        provider_status,
        reasons,
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

fn focus_report_summary(session: &SessionFile, focus_events: &[FocusEvent]) -> FocusReportSummary {
    let final_event = focus_events
        .iter()
        .rev()
        .find(|event| event.action == "changed" || event.kind.is_some());

    let mode = session
        .core
        .focus_mode
        .clone()
        .or_else(|| session.config.auto_focus.then(|| "auto-focus".to_owned()));

    let final_focus = final_event
        .and_then(|event| event.kind.clone())
        .or_else(|| session.core.final_focus_kind.clone());

    let situation =
        final_event.and_then(|event| event.situation.map(|situation| format!("{situation:?}")));

    let confidence = final_event.map(|event| event.confidence);
    let score = final_event.map(|event| event.score);
    let roots = final_event
        .map(|event| event.root_pids.clone())
        .unwrap_or_default();
    let member_pids = final_event
        .map(|event| event.member_pids.clone())
        .unwrap_or_default();
    let reasons = final_event
        .map(|event| event.reasons.clone())
        .unwrap_or_default();

    let display_name = final_focus.clone();

    FocusReportSummary {
        mode,
        final_focus,
        display_name,
        situation,
        confidence,
        score,
        roots,
        member_pids,
        focus_switches: session.core.focus_switch_count,
        reasons,
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportAnalysisJson {
    pub session: SessionFile,
    pub cluster_analysis: SpikeClusterAnalysis,
    pub frame_diagnoses: Vec<FrameDiagnosis>,
    pub pressure_timeline: PressureTimelineSummary,
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
}

#[derive(Debug, Clone, Serialize)]
pub struct HtmlReportModel {
    pub session: SessionFile,
    pub data_quality: DataQualitySummary,
    pub cluster_analysis: SpikeClusterAnalysis,
    pub frame_diagnoses: Vec<FrameDiagnosis>,
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
struct SpikeClusterCandidate {
    start_idx: usize,
    end_idx: usize,
    distinct_tasks: usize,
    min_switch_ns: u64,
    max_switch_ns: u64,
    max_latency_ns: u64,
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

struct ReportBuildResult {
    analysis: ReportAnalysisJson,
    artifacts: session_io::RunArtifacts,
}

fn artifacts_summary_from_session(session: &SessionFile) -> ArtifactsSummary {
    ArtifactsSummary {
        spike_count: session
            .core
            .spike_events_retained_count
            .max(session.top_spikes.len() as u64),
        frame_count: session.core.frame_event_count,
        irq_event_count: session.core.irq_event_count,
        gpu_sample_count: session.core.gpu_sample_count,
        frame_event_count: session.core.frame_event_count,
        migration_event_count: session.core.migration_event_count.unwrap_or(0),
        cpu_freq_sample_count: session.core.cpu_freq_sample_count.unwrap_or(0),
        block_io_event_count: session.core.block_io_event_count,
        interval_record_count: session.core.interval_record_count,
        scx_event_count: session.core.scx_event_count,
        focus_event_count: session.core.focus_event_count,
        foreground_event_count: session.core.foreground_event_count,
    }
}

fn log_run_validation(path: &Path, validation: &session_io::RunValidationReport) {
    if !validation.is_ok() {
        for err in &validation.errors {
            log::error!("run_dir_validation_error path={} err={err}", path.display());
        }
    }
    for warning in &validation.warnings {
        log::warn!(
            "run_dir_validation_warning path={} warn={warning}",
            path.display()
        );
    }
}

pub fn build_report_analysis(
    path: &Path,
    top: usize,
    cluster_window_ms: u64,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<ReportAnalysisJson> {
    Ok(build_report_analysis_with_artifacts(path, top, cluster_window_ms, filter_class)?.analysis)
}

// Keep near-term report improvements view-only: derive new report fields from
// existing RunArtifacts/session data. Do not require new live probes here.
fn build_report_analysis_with_artifacts(
    path: &Path,
    top: usize,
    cluster_window_ms: u64,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<ReportBuildResult> {
    let validation = session_io::validate_run_dir_shallow(path)?;
    log_run_validation(path, &validation);

    let mut artifacts = session_io::load_run_artifacts(path, ArtifactLoadOptions::REPORT)?;
    let session = artifacts.session.clone();

    let median_frametime = calculate_median_frametime(&artifacts.frame_events);
    let frame_spikes = identify_frame_spikes(&artifacts.frame_events, median_frametime);

    let cluster_window_ns = cluster_window_ms.saturating_mul(1_000_000);
    let spike_events_ref = if !artifacts.spikes.is_empty() {
        Some(&artifacts.spikes[..])
    } else {
        None
    };

    let cluster_analysis = spike_cluster_analysis(
        &session,
        spike_events_ref,
        cluster_window_ns,
        top,
        filter_class,
    );

    let windows = compute_correlation_windows(
        &session,
        &cluster_analysis.clusters,
        &frame_spikes,
        cluster_window_ns,
    );
    artifacts.load_correlations(windows)?;

    let mut cluster_analysis = cluster_analysis;
    perform_diagnosis(
        &mut cluster_analysis.clusters,
        &artifacts,
        cluster_window_ns,
    );

    annotate_clusters_with_foreground(
        &mut cluster_analysis.clusters,
        &artifacts.foreground_events,
        session.config.foreground_max_stale_ms.max(2_500),
    );

    let all_spike_points = if !artifacts.spikes.is_empty() {
        flatten_spike_events(&session, &artifacts.spikes)
    } else {
        flatten_top_spikes(&session)
    };

    let frame_diagnoses = perform_frame_diagnosis(
        &session,
        &frame_spikes,
        &all_spike_points,
        &artifacts,
        cluster_window_ns,
    );

    let data_quality = data_quality_summary(&session, &artifacts.validation);
    let artifacts_summary = artifacts_summary_from_session(&session);
    let pressure_timeline = build_pressure_timeline(
        &artifacts.intervals,
        &cluster_analysis.clusters,
        cluster_window_ms,
    );

    let focus_summary = focus_report_summary(&session, &artifacts.focus_events);
    let foreground_summary = foreground_report_summary(&session, &artifacts.foreground_events);

    Ok(ReportBuildResult {
        analysis: ReportAnalysisJson {
            session,
            cluster_analysis,
            frame_diagnoses,
            pressure_timeline,
            artifacts_summary,
            data_quality,
            focus_summary,
            foreground_summary,
        },
        artifacts,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn print_report(
    path: &Path,
    json: bool,
    analysis_json: bool,
    json_summary: bool,
    top: usize,
    cluster_window_ms: u64,
    filter_class: Option<TaskClass>,
    flamegraph: Option<PathBuf>,
) -> anyhow::Result<()> {
    if json {
        let validation = session_io::validate_run_dir_shallow(path)?;
        log_run_validation(path, &validation);
        let session = session_io::load_session(path)?;
        println!("{}", serde_json::to_string_pretty(&session)?);
        return Ok(());
    }

    if json_summary {
        let summary = summary::build_compact_run_summary(path, top, filter_class)?;
        println!("{}", serde_json::to_string_pretty(&summary)?);
        return Ok(());
    }

    if analysis_json {
        let analysis = build_report_analysis(path, top, cluster_window_ms, filter_class)?;
        println!("{}", serde_json::to_string_pretty(&analysis)?);
        return Ok(());
    }

    let ReportBuildResult {
        analysis,
        artifacts,
    } = build_report_analysis_with_artifacts(path, top, cluster_window_ms, filter_class)?;

    if let Some(flamegraph_path) = flamegraph {
        crate::flamegraph::write_latency_flamegraph_svg(&artifacts.spikes, &flamegraph_path)?;
    }

    print!(
        "{}",
        render_report(
            path,
            &analysis.session,
            &analysis.cluster_analysis,
            &analysis.frame_diagnoses,
            &artifacts,
            &analysis.focus_summary,
            &analysis.foreground_summary,
            top,
            cluster_window_ms,
            filter_class,
        )
    );

    Ok(())
}

pub fn render_diff_report(
    path_a: &Path,
    path_b: &Path,
    top: usize,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<String> {
    let diff = summary::build_run_diff_summary(path_a, path_b, filter_class)?;
    Ok(render_run_diff_summary(&diff, top))
}

fn render_focus_summary_text(focus: &FocusReportSummary) -> String {
    let mut output = String::new();

    if !focus.is_visible() {
        return output;
    }

    pushln(&mut output, "Auto focus:");
    pushln(
        &mut output,
        format!("  mode: {}", focus.mode.as_deref().unwrap_or("unknown")),
    );
    pushln(
        &mut output,
        format!(
            "  final focus: {}",
            focus.final_focus.as_deref().unwrap_or("none")
        ),
    );
    pushln(
        &mut output,
        format!(
            "  situation: {}",
            focus.situation.as_deref().unwrap_or("unknown")
        ),
    );

    if let Some(confidence) = focus.confidence {
        pushln(&mut output, format!("  confidence: {:.2}", confidence));
    } else {
        pushln(&mut output, "  confidence: unknown");
    }

    pushln(&mut output, format!("  roots: {:?}", focus.roots));
    pushln(
        &mut output,
        format!("  focus switches: {}", focus.focus_switches),
    );

    if !focus.reasons.is_empty() {
        pushln(&mut output, "  reasons:");
        for reason in &focus.reasons {
            pushln(&mut output, format!("    - {reason}"));
        }
    }

    pushln(&mut output, "");
    output
}

fn render_foreground_summary_text(foreground: &ForegroundReportSummary) -> String {
    let mut output = String::new();

    if !foreground.is_visible() {
        return output;
    }

    pushln(&mut output, "Foreground window:");
    pushln(
        &mut output,
        format!(
            "  source: {}",
            foreground.source.as_deref().unwrap_or("unknown")
        ),
    );

    if let Some(pid) = foreground.final_pid {
        pushln(&mut output, format!("  final pid: {pid}"));
    } else {
        pushln(&mut output, "  final pid: none");
    }

    let app_or_class = foreground
        .final_app_id
        .as_deref()
        .or(foreground.final_class.as_deref())
        .unwrap_or("unknown");
    pushln(&mut output, format!("  app_id/class: {app_or_class}"));

    if let Some(workspace) = foreground.final_workspace.as_deref() {
        pushln(&mut output, format!("  workspace: {workspace}"));
    } else {
        pushln(&mut output, "  workspace: unknown");
    }

    if let Some(title) = foreground.final_title.as_deref() {
        pushln(&mut output, format!("  title: {title}"));
    } else if foreground.enabled || foreground.event_count > 0 {
        pushln(
            &mut output,
            "  title: redacted (pass --foreground-include-title to record it)",
        );
    }

    if let Some(confidence) = foreground.confidence {
        pushln(&mut output, format!("  confidence: {:.2}", confidence));
    } else {
        pushln(&mut output, "  confidence: unknown");
    }

    if let Some(status) = foreground.provider_status.as_deref() {
        pushln(&mut output, format!("  provider status: {status}"));
    }

    pushln(&mut output, format!("  events: {}", foreground.event_count));

    if !foreground.reasons.is_empty() {
        pushln(&mut output, "  reasons:");
        for reason in &foreground.reasons {
            pushln(&mut output, format!("    - {reason}"));
        }
    }

    pushln(&mut output, "");
    output
}

fn render_run_diff_summary(diff: &RunDiffSummary, top: usize) -> String {
    let mut output = String::new();
    pushln(&mut output, "stutter diff report");
    pushln(&mut output, "===================");
    pushln(
        &mut output,
        format!(
            "run_a: {} ({}ms)",
            diff.baseline_run_name.as_deref().unwrap_or("-"),
            diff.baseline_duration_ms,
        ),
    );
    pushln(
        &mut output,
        format!(
            "run_b: {} ({}ms)",
            diff.current_run_name.as_deref().unwrap_or("-"),
            diff.current_duration_ms,
        ),
    );
    pushln(&mut output, "");

    pushln(&mut output, "summary highlights");
    pushln(&mut output, "------------------");
    if let Some(worst) = &diff.worst_max_regression {
        let pct = if worst.baseline_max_ns > 0 {
            format!(
                " (+{:.1}%)",
                (worst.delta_max_ns as f64 / worst.baseline_max_ns as f64) * 100.0
            )
        } else {
            String::new()
        };
        pushln(
            &mut output,
            format!(
                "biggest regression:  {} on comm={} process={}{}",
                format_latency_signed(worst.delta_max_ns),
                worst.identity.comm,
                worst.identity.process_comm,
                pct
            ),
        );
    }
    if let Some(best) = diff.improvements.first() {
        let pct = if best.baseline_max_ns > 0 {
            format!(
                " ({:.1}%)",
                (best.delta_max_ns as f64 / best.baseline_max_ns as f64) * 100.0
            )
        } else {
            String::new()
        };
        pushln(
            &mut output,
            format!(
                "biggest improvement: {} on comm={} process={}{}",
                format_latency_signed(best.delta_max_ns),
                best.identity.comm,
                best.identity.process_comm,
                pct
            ),
        );
    }
    pushln(&mut output, "");

    pushln(&mut output, "regressions (worse in run_b)");
    pushln(&mut output, "---------------------------");
    if diff.regressions.is_empty() {
        pushln(&mut output, "none");
    }
    for d in diff.regressions.iter().take(top) {
        pushln(
            &mut output,
            format!(
                "class={:?} comm={} process={} max: {} -> {} (delta={}) p99_delta={} over_1ms_delta={}",
                d.identity.class,
                d.identity.comm,
                d.identity.process_comm,
                format_latency(d.baseline_max_ns),
                format_latency(d.current_max_ns),
                format_latency_signed(d.delta_max_ns),
                format_latency_signed(d.delta_p99_ns),
                if d.delta_over_1ms >= 0 {
                    format!("+{}", d.delta_over_1ms)
                } else {
                    d.delta_over_1ms.to_string()
                },
            ),
        );
    }
    pushln(&mut output, "");

    pushln(&mut output, "improvements (better in run_b)");
    pushln(&mut output, "-----------------------------");
    if diff.improvements.is_empty() {
        pushln(&mut output, "none");
    }
    for d in diff.improvements.iter().take(top) {
        pushln(
            &mut output,
            format!(
                "class={:?} comm={} process={} max: {} -> {} (delta={}) p99_delta={} over_1ms_delta={}",
                d.identity.class,
                d.identity.comm,
                d.identity.process_comm,
                format_latency(d.baseline_max_ns),
                format_latency(d.current_max_ns),
                format_latency_signed(d.delta_max_ns),
                format_latency_signed(d.delta_p99_ns),
                if d.delta_over_1ms >= 0 {
                    format!("+{}", d.delta_over_1ms)
                } else {
                    d.delta_over_1ms.to_string()
                },
            ),
        );
    }
    pushln(&mut output, "");

    if !diff.new_tasks.is_empty() {
        pushln(&mut output, "new tasks (only in run_b)");
        pushln(&mut output, "------------------------");
        for task in diff.new_tasks.iter().take(top) {
            pushln(
                &mut output,
                format!(
                    "comm={} process={} class={:?}",
                    task.identity.comm, task.identity.process_comm, task.identity.class
                ),
            );
        }
        pushln(&mut output, "");
    }

    if !diff.removed_tasks.is_empty() {
        pushln(&mut output, "removed tasks (only in run_a)");
        pushln(&mut output, "----------------------------");
        for task in diff.removed_tasks.iter().take(top) {
            pushln(
                &mut output,
                format!(
                    "comm={} process={} class={:?}",
                    task.identity.comm, task.identity.process_comm, task.identity.class
                ),
            );
        }
        pushln(&mut output, "");
    }

    output
}

#[allow(dead_code)]
pub fn check_percentile_regression(
    path_baseline: &Path,
    path_current: &Path,
    max_regression_p99_ms: f64,
) -> anyhow::Result<()> {
    check_regression(
        path_baseline,
        path_current,
        Some(max_regression_p99_ms),
        None,
        false,
        10,
        None,
    )
}

#[derive(Clone, Debug, Serialize)]
pub struct RegressionCheckSummary {
    pub passed: bool,
    pub baseline_path: PathBuf,
    pub current_path: PathBuf,
    pub max_regression_p99_ms: Option<f64>,
    pub max_max_regression_ms: Option<f64>,
    pub violations: Vec<RegressionViolation>,
    pub diff: RunDiffSummary,
}

#[derive(Clone, Debug, Serialize)]
pub struct RegressionViolation {
    pub metric: RegressionMetric,
    pub comm: String,
    pub process_comm: String,
    pub class: TaskClass,
    pub delta_ns: i64,
    pub threshold_ns: i64,
    pub new_task: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegressionMetric {
    P99,
    Max,
}

pub fn check_regression(
    path_baseline: &Path,
    path_current: &Path,
    max_regression_p99_ms: Option<f64>,
    max_max_regression_ms: Option<f64>,
    json: bool,
    top: usize,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<()> {
    let diff = summary::build_run_diff_summary(path_baseline, path_current, filter_class)?;
    let mut violations = Vec::new();

    if let Some(threshold_ms) = max_regression_p99_ms {
        let threshold_ns = ms_to_ns_i64(threshold_ms);
        for delta in diff
            .regressions
            .iter()
            .filter(|delta| delta.delta_p99_ns > threshold_ns)
        {
            violations.push(violation_from_delta(
                RegressionMetric::P99,
                delta,
                delta.delta_p99_ns,
                threshold_ns,
            ));
        }
        for task in diff
            .new_scored_tasks
            .iter()
            .filter(|task| task.p99_ns as i64 > threshold_ns)
        {
            violations.push(RegressionViolation {
                metric: RegressionMetric::P99,
                comm: task.identity.comm.clone(),
                process_comm: task.identity.process_comm.clone(),
                class: task.identity.class,
                delta_ns: task.p99_ns as i64,
                threshold_ns,
                new_task: true,
            });
        }
    }

    if let Some(threshold_ms) = max_max_regression_ms {
        let threshold_ns = ms_to_ns_i64(threshold_ms);
        for delta in diff
            .regressions
            .iter()
            .filter(|delta| delta.delta_max_ns > threshold_ns)
        {
            violations.push(violation_from_delta(
                RegressionMetric::Max,
                delta,
                delta.delta_max_ns,
                threshold_ns,
            ));
        }
        for task in diff
            .new_scored_tasks
            .iter()
            .filter(|task| task.max_ns as i64 > threshold_ns)
        {
            violations.push(RegressionViolation {
                metric: RegressionMetric::Max,
                comm: task.identity.comm.clone(),
                process_comm: task.identity.process_comm.clone(),
                class: task.identity.class,
                delta_ns: task.max_ns as i64,
                threshold_ns,
                new_task: true,
            });
        }
    }

    violations.sort_by_key(|violation| std::cmp::Reverse(violation.delta_ns));
    let passed = violations.is_empty();
    let output = RegressionCheckSummary {
        passed,
        baseline_path: path_baseline.to_path_buf(),
        current_path: path_current.to_path_buf(),
        max_regression_p99_ms,
        max_max_regression_ms,
        violations,
        diff: diff.limited(top),
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        print!("{}", render_check_summary(&output, top));
    }

    if !output.passed {
        if let Some(first) = output.violations.first() {
            anyhow::bail!(
                "regression_check_failed metric={:?} regressed_by={} comm={} process={} max_allowed={}",
                first.metric,
                format_latency_signed(first.delta_ns),
                first.comm,
                first.process_comm,
                format_latency(first.threshold_ns as u64)
            );
        }
        anyhow::bail!("regression_check_failed");
    }

    Ok(())
}

fn violation_from_delta(
    metric: RegressionMetric,
    delta: &TaskDeltaSummary,
    delta_ns: i64,
    threshold_ns: i64,
) -> RegressionViolation {
    RegressionViolation {
        metric,
        comm: delta.identity.comm.clone(),
        process_comm: delta.identity.process_comm.clone(),
        class: delta.identity.class,
        delta_ns,
        threshold_ns,
        new_task: false,
    }
}

fn render_check_summary(summary: &RegressionCheckSummary, top: usize) -> String {
    let mut output = String::new();
    pushln(&mut output, "stutter check");
    pushln(&mut output, "=============");
    pushln(
        &mut output,
        format!("baseline: {}", summary.baseline_path.display()),
    );
    pushln(
        &mut output,
        format!("current: {}", summary.current_path.display()),
    );
    pushln(
        &mut output,
        format!(
            "result: {}",
            if summary.passed { "passed" } else { "failed" }
        ),
    );
    if let Some(threshold) = summary.max_regression_p99_ms {
        pushln(&mut output, format!("max_regression_p99_ms: {threshold}"));
    }
    if let Some(threshold) = summary.max_max_regression_ms {
        pushln(&mut output, format!("max_max_regression_ms: {threshold}"));
    }

    if let Some(worst) = &summary.diff.worst_p99_regression {
        pushln(
            &mut output,
            format!(
                "worst_p99_regression: {} on comm={} process={}",
                format_latency_signed(worst.delta_p99_ns),
                worst.identity.comm,
                worst.identity.process_comm
            ),
        );
    } else {
        pushln(&mut output, "worst_p99_regression: none");
    }

    if let Some(worst) = &summary.diff.worst_max_regression {
        pushln(
            &mut output,
            format!(
                "worst_max_regression: {} on comm={} process={}",
                format_latency_signed(worst.delta_max_ns),
                worst.identity.comm,
                worst.identity.process_comm
            ),
        );
    } else {
        pushln(&mut output, "worst_max_regression: none");
    }

    if !summary.violations.is_empty() {
        pushln(&mut output, "");
        pushln(&mut output, "violations");
        pushln(&mut output, "----------");
        for violation in summary.violations.iter().take(top) {
            pushln(
                &mut output,
                format!(
                    "metric={:?} class={:?} comm={} process={} delta={} threshold={} new_task={}",
                    violation.metric,
                    violation.class,
                    violation.comm,
                    violation.process_comm,
                    format_latency_signed(violation.delta_ns),
                    format_latency(violation.threshold_ns as u64),
                    violation.new_task
                ),
            );
        }
    }

    if !summary.diff.regressions.is_empty() {
        pushln(&mut output, "");
        pushln(&mut output, "top regressions");
        pushln(&mut output, "---------------");
        for delta in summary.diff.regressions.iter().take(top) {
            pushln(
                &mut output,
                format!(
                    "class={:?} comm={} process={} p99_delta={} max_delta={} over_1ms_delta={}",
                    delta.identity.class,
                    delta.identity.comm,
                    delta.identity.process_comm,
                    format_latency_signed(delta.delta_p99_ns),
                    format_latency_signed(delta.delta_max_ns),
                    delta.delta_over_1ms
                ),
            );
        }
    }

    if !summary.diff.improvements.is_empty() {
        pushln(&mut output, "");
        pushln(&mut output, "top improvements");
        pushln(&mut output, "----------------");
        for delta in summary.diff.improvements.iter().take(top) {
            pushln(
                &mut output,
                format!(
                    "class={:?} comm={} process={} p99_delta={} max_delta={} over_1ms_delta={}",
                    delta.identity.class,
                    delta.identity.comm,
                    delta.identity.process_comm,
                    format_latency_signed(delta.delta_p99_ns),
                    format_latency_signed(delta.delta_max_ns),
                    delta.delta_over_1ms
                ),
            );
        }
    }

    output
}

fn ms_to_ns_i64(value: f64) -> i64 {
    (value * 1_000_000.0).ceil() as i64
}

pub fn print_diff_report(
    path_a: &Path,
    path_b: &Path,
    top: usize,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<()> {
    print!("{}", render_diff_report(path_a, path_b, top, filter_class)?);
    Ok(())
}

pub fn print_batch_report(
    batch_dir: &Path,
    baseline_path: Option<&Path>,
    json_summary: bool,
    top: usize,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<()> {
    let summary = summary::build_batch_run_summary(batch_dir, baseline_path, top, filter_class)?;
    if json_summary {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        print!("{}", summary::render_batch_run_summary(&summary, top));
    }
    Ok(())
}

pub fn build_html_report_model(
    session: &SessionFile,
    artifacts: &session_io::RunArtifacts,
    analysis: &ReportAnalysisJson,
    top: usize,
    filter_class: Option<TaskClass>,
    legacy_text_report: Option<String>,
) -> anyhow::Result<HtmlReportModel> {
    let spike_events = if artifacts.spikes.is_empty() {
        None
    } else {
        Some(artifacts.spikes.clone())
    };

    let duration_ms = session.core.duration_ms.max(1);
    let bucket_ms = (duration_ms / 500).clamp(1, 1000);
    let spike_density = spike_events
        .as_deref()
        .map(|spikes| build_spike_density(spikes, bucket_ms))
        .unwrap_or_default();

    Ok(HtmlReportModel {
        session: session.clone(),
        data_quality: analysis.data_quality.clone(),
        cluster_analysis: analysis.cluster_analysis.clone(),
        frame_diagnoses: analysis.frame_diagnoses.clone(),
        artifacts_summary: analysis.artifacts_summary.clone(),
        focus_summary: analysis.focus_summary.clone(),
        foreground_summary: analysis.foreground_summary.clone(),
        top_tasks_by_max: top_task_rows_by_max_latency(session, top, filter_class),
        top_tasks_by_p99: top_task_rows_by_p99_latency(session, top, filter_class),
        spike_density,
        top_limit: top,
        spike_events,
        chart_artifacts: HtmlChartArtifacts {
            gpu_samples: artifacts.gpu_samples.clone(),
        },
        legacy_text_report,
    })
}

fn top_task_rows_by_max_latency(
    session: &SessionFile,
    top: usize,
    filter_class: Option<TaskClass>,
) -> Vec<TaskHtmlRow> {
    let mut tasks = filtered_latency_tasks(session, filter_class);
    tasks.sort_by_key(|task| std::cmp::Reverse(task.latency.max_ns));
    tasks.into_iter().take(top).map(task_html_row).collect()
}

fn top_task_rows_by_p99_latency(
    session: &SessionFile,
    top: usize,
    filter_class: Option<TaskClass>,
) -> Vec<TaskHtmlRow> {
    let mut tasks = filtered_latency_tasks(session, filter_class);
    tasks.sort_by_key(|task| {
        (
            std::cmp::Reverse(task.latency.p99_ns),
            std::cmp::Reverse(task.latency.max_ns),
        )
    });
    tasks.into_iter().take(top).map(task_html_row).collect()
}

fn filtered_latency_tasks(
    session: &SessionFile,
    filter_class: Option<TaskClass>,
) -> Vec<&SessionTask> {
    session
        .tasks
        .iter()
        .filter(|task| task.latency.samples > 0)
        .filter(|task| filter_class.is_none_or(|class| task.class == class))
        .collect()
}

fn task_html_row(task: &SessionTask) -> TaskHtmlRow {
    TaskHtmlRow {
        task: task.task,
        active: task.active,
        class: task.class,
        process_pid: task.process_pid,
        process_comm: task.process_comm.to_string(),
        comm: task.comm.clone(),
        samples: task.latency.samples,
        spike_count: task.latency.over_1ms,
        max_latency_ms: ns_to_ms(task.latency.max_ns),
        p99_latency_ms: ns_to_ms(task.latency.p99_ns),
        avg_latency_ms: ns_to_ms(task.latency.avg_ns),
        over_1ms: task.latency.over_1ms,
        over_2ms: task.latency.over_2ms,
        over_5ms: task.latency.over_5ms,
    }
}

fn ns_to_ms(ns: u64) -> f64 {
    ns as f64 / 1_000_000.0
}

pub fn write_html_report(
    path: &Path,
    html_path: &Path,
    top: usize,
    cluster_window_ms: u64,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<()> {
    let ReportBuildResult {
        analysis,
        artifacts,
    } = build_report_analysis_with_artifacts(path, top, cluster_window_ms, filter_class)?;

    let text_report = render_report(
        path,
        &analysis.session,
        &analysis.cluster_analysis,
        &analysis.frame_diagnoses,
        &artifacts,
        &analysis.focus_summary,
        &analysis.foreground_summary,
        top,
        cluster_window_ms,
        filter_class,
    );
    let model = build_html_report_model(
        &analysis.session,
        &artifacts,
        &analysis,
        top,
        filter_class,
        Some(text_report),
    )?;
    let html = render_html_report(&model)?;
    if let Some(parent) = html_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(html_path, html)
        .with_context(|| format!("failed to write HTML report {}", html_path.display()))?;
    Ok(())
}

pub fn render_html_report(model: &HtmlReportModel) -> anyhow::Result<String> {
    let model_json = escape_json_for_script_tag(
        &serde_json::to_string(model).context("failed to serialize HTML report model")?,
    );
    let session_json = escape_json_for_script_tag(
        &serde_json::to_string(&model.session).context("failed to serialize HTML session data")?,
    );
    let spike_events_json = escape_json_for_script_tag(
        &serde_json::to_string(&model.spike_events)
            .context("failed to serialize HTML spike event data")?,
    );
    let spike_density_json = escape_json_for_script_tag(
        &serde_json::to_string(&model.spike_density)
            .context("failed to serialize HTML spike density data")?,
    );
    let artifacts_json = escape_json_for_script_tag(
        &serde_json::to_string(&model.chart_artifacts)
            .context("failed to serialize HTML chart artifact data")?,
    );
    let cluster_analysis_json = escape_json_for_script_tag(
        &serde_json::to_string(&model.cluster_analysis)
            .context("failed to serialize HTML cluster data")?,
    );

    let template = include_str!("report_template.html");

    Ok(template
        .replace("{html_report_model_json}", &model_json)
        .replace("{session_json}", &session_json)
        .replace("{spike_events_json}", &spike_events_json)
        .replace("{spike_density_json}", &spike_density_json)
        .replace("{artifacts_json}", &artifacts_json)
        .replace("{cluster_analysis_json}", &cluster_analysis_json)
        .replace("{top}", &model.top_limit.to_string()))
}

pub fn build_spike_density(spikes: &[SpikeEvent], bucket_ms: u64) -> Vec<SpikeDensityBucket> {
    if spikes.is_empty() {
        return Vec::new();
    }

    let bucket_ms = bucket_ms.max(1);

    #[derive(Default)]
    struct BucketAccum {
        start_ms: u64,
        end_ms: u64,
        count: u64,
        max_latency_ms: f64,
        latencies_ms: Vec<f64>,
    }

    let mut buckets: BTreeMap<u64, BucketAccum> = BTreeMap::new();

    for spike in spikes {
        let elapsed_ms = spike.elapsed_ms.unwrap_or(0);
        let latency_ms = spike.latency_ns as f64 / 1_000_000.0;

        let bucket_idx = elapsed_ms / bucket_ms;
        let start_ms = bucket_idx * bucket_ms;
        let end_ms = start_ms + bucket_ms;

        let bucket = buckets.entry(bucket_idx).or_insert_with(|| BucketAccum {
            start_ms,
            end_ms,
            count: 0,
            max_latency_ms: 0.0,
            latencies_ms: Vec::new(),
        });

        bucket.count += 1;
        if latency_ms.is_finite() {
            bucket.max_latency_ms = bucket.max_latency_ms.max(latency_ms);
            bucket.latencies_ms.push(latency_ms);
        }
    }

    buckets
        .into_values()
        .map(|mut bucket| {
            let p99_latency_ms = percentile_f64(&mut bucket.latencies_ms, 0.99);
            SpikeDensityBucket {
                start_ms: bucket.start_ms,
                end_ms: bucket.end_ms,
                count: bucket.count,
                max_latency_ms: bucket.max_latency_ms,
                p99_latency_ms,
            }
        })
        .collect()
}

fn percentile_f64(values: &mut [f64], percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }

    values.sort_by(|a, b| a.total_cmp(b));

    let len = values.len();
    let rank = ((len as f64 - 1.0) * percentile).round() as usize;
    values[rank.min(len - 1)]
}

fn escape_json_for_script_tag(s: &str) -> String {
    s.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace("</", "<\\/")
}

// sched_switch.prev_state values are Linux task-state bits and can vary across
// kernels. These labels are conservative. prev_state describes the task switched
// out, not the task switched in.
pub fn classify_switch_prev_state(prev_state: i64) -> &'static str {
    const TASK_INTERRUPTIBLE: i64 = 1;
    const TASK_UNINTERRUPTIBLE: i64 = 2;

    if prev_state == 0 {
        "preempted_or_runnable"
    } else if prev_state & TASK_INTERRUPTIBLE != 0 {
        "voluntary_sleep_interruptible"
    } else if prev_state & TASK_UNINTERRUPTIBLE != 0 {
        "voluntary_sleep_uninterruptible"
    } else {
        "other_sleep"
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn render_report(
    path: &Path,
    session: &SessionFile,
    cluster_analysis: &SpikeClusterAnalysis,
    frame_diagnoses: &[FrameDiagnosis],
    artifacts: &session_io::RunArtifacts,
    focus_summary: &FocusReportSummary,
    foreground_summary: &ForegroundReportSummary,
    top: usize,
    cluster_window_ms: u64,
    filter_class: Option<TaskClass>,
) -> String {
    let mut output = String::new();
    let data_quality = data_quality_summary(session, &artifacts.validation);

    pushln(&mut output, "stutter report");
    pushln(&mut output, "==============");

    output.push_str(&render_focus_summary_text(focus_summary));
    output.push_str(&render_foreground_summary_text(foreground_summary));

    pushln(&mut output, format!("file: {}", path.display()));
    pushln(
        &mut output,
        format!("schema: {}", session.core.schema_version),
    );
    pushln(
        &mut output,
        format!("expected_schema: {}", SESSION_SCHEMA_VERSION),
    );
    pushln(
        &mut output,
        format!("run: {}", session.core.run_name.as_deref().unwrap_or("-")),
    );
    pushln(
        &mut output,
        format!("duration_ms: {}", session.core.duration_ms),
    );
    pushln(&mut output, format!("stop_reason: {}", session.stop_reason));
    pushln(
        &mut output,
        format!("manual_pids: {:?}", session.config.manual_pids),
    );
    pushln(
        &mut output,
        format!("tree_roots: {:?}", session.config.tree_roots),
    );
    pushln(
        &mut output,
        format!("include_comm: {:?}", session.config.include_comm),
    );
    pushln(
        &mut output,
        format!("exclude_comm: {:?}", session.config.exclude_comm),
    );

    if let Some(warning) = event_stream_warning(
        session.core.event_stream_write_errors,
        session.core.first_event_stream_write_error.as_deref(),
    ) {
        pushln(&mut output, warning);
        pushln(&mut output, "");
    }
    pushln(
        &mut output,
        format!(
            "watch_process: {}",
            session.config.watch_process.as_deref().unwrap_or("-")
        ),
    );
    pushln(
        &mut output,
        format!("persistent: {}", session.config.persistent),
    );
    pushln(
        &mut output,
        format!(
            "csv_stream: {}",
            match &session.config.csv_stream {
                Some(crate::cli::CsvStreamTarget::File(path)) => path.display().to_string(),
                Some(crate::cli::CsvStreamTarget::Stdout) => "stdout".to_owned(),
                None => "-".to_owned(),
            }
        ),
    );
    pushln(
        &mut output,
        format!(
            "active_tasks_at_end: {}",
            session.core.active_target_pids_count
        ),
    );
    pushln(&mut output, "");

    pushln(&mut output, "data quality");
    pushln(&mut output, "------------");
    pushln(&mut output, format!("level: {:?}", data_quality.level));
    pushln(
        &mut output,
        format!(
            "schema: {} expected={}",
            data_quality.schema_version, data_quality.expected_schema_version
        ),
    );
    pushln(
        &mut output,
        format!(
            "event_stream_write_errors: {}",
            data_quality.event_stream_write_errors
        ),
    );
    pushln(
        &mut output,
        format!(
            "spike_events: retained={} dropped={} truncated={}",
            data_quality.spike_events_retained_count,
            data_quality.spike_events_dropped_count,
            data_quality.spike_events_truncated
        ),
    );
    pushln(
        &mut output,
        format!("interval_records: {}", data_quality.interval_record_count),
    );
    pushln(
        &mut output,
        format!(
            "active_target_pids: {}",
            data_quality.active_target_pids_count
        ),
    );
    pushln(
        &mut output,
        format!(
            "drop_counters_nonzero: {}",
            data_quality.drop_counters_nonzero
        ),
    );
    pushln(
        &mut output,
        format!(
            "percentile_scope_counts: {:?}",
            data_quality.percentile_scope_counts
        ),
    );
    pushln(
        &mut output,
        format!(
            "block_io_correlation_basis: {}",
            data_quality.block_io_correlation_basis
        ),
    );
    pushln(
        &mut output,
        format!(
            "frame_timestamp_alignment: {}",
            data_quality.frame_timestamp_alignment
        ),
    );
    pushln(
        &mut output,
        format!(
            "cpu_perf: requested={} open_errors={} read_errors={} skipped_tasks={}",
            data_quality.cpu_perf_requested,
            data_quality.cpu_perf_open_errors,
            data_quality.cpu_perf_read_errors,
            data_quality.cpu_perf_skipped_tasks
        ),
    );

    for reason in &data_quality.reasons {
        pushln(&mut output, format!("reason: {reason}"));
    }

    if !data_quality.missing_optional_files.is_empty() {
        pushln(
            &mut output,
            format!(
                "missing_optional_files: {:?}",
                data_quality.missing_optional_files
            ),
        );
    }

    if !data_quality.validation_warnings.is_empty() {
        for warning in &data_quality.validation_warnings {
            pushln(&mut output, format!("validation_warning: {warning}"));
        }
    }

    if !data_quality.validation_errors.is_empty() {
        for error in &data_quality.validation_errors {
            pushln(&mut output, format!("validation_error: {error}"));
        }
    }

    pushln(&mut output, "");

    let pressure_timeline = build_pressure_timeline(
        &artifacts.intervals,
        &cluster_analysis.clusters,
        cluster_window_ms,
    );
    if pressure_timeline_has_pressure(&pressure_timeline) {
        output.push_str(&render_pressure_timeline_summary(&pressure_timeline));
        pushln(&mut output, "");
    }

    if session.core.spike_events_truncated {
        pushln(&mut output, "spike event warning");
        pushln(&mut output, "-------------------");
        pushln(
            &mut output,
            format!(
                "spike_events_truncated=true retained_spike_events={} note=spike_events.json is capped; top_spikes and threshold counters remain available",
                session.core.spike_events_retained_count
            ),
        );
        pushln(&mut output, "");
    }

    if session.core.scx_event_count > 0 {
        pushln(
            &mut output,
            format!("scx_events: {}", session.core.scx_event_count),
        );
        pushln(&mut output, "");
    }
    if session.core.irq_event_count > 0
        || session.core.gpu_sample_count > 0
        || session.core.frame_event_count > 0
        || session.core.block_io_event_count > 0
        || session.core.migration_event_count.unwrap_or(0) > 0
        || session.core.cpu_freq_sample_count.unwrap_or(0) > 0
    {
        pushln(&mut output, "correlation artifacts");
        pushln(&mut output, "---------------------");
        pushln(
            &mut output,
            format!("irq_events: {}", session.core.irq_event_count),
        );
        pushln(
            &mut output,
            format!("gpu_samples: {}", session.core.gpu_sample_count),
        );
        pushln(
            &mut output,
            format!("frame_events: {}", session.core.frame_event_count),
        );
        if session.core.frame_event_count > 0 {
            let alignment = if session.core.mangohud_first_frame_monotonic_ns.is_some() {
                "monotonic_observed"
            } else {
                "approximate_first_row"
            };
            pushln(
                &mut output,
                format!("frame_timestamp_alignment={}", alignment),
            );
        }
        pushln(
            &mut output,
            format!(
                "migration_events: {}",
                session.core.migration_event_count.unwrap_or(0)
            ),
        );
        pushln(
            &mut output,
            format!(
                "cpu_freq_samples: {}",
                session.core.cpu_freq_sample_count.unwrap_or(0)
            ),
        );
        pushln(
            &mut output,
            format!(
                "io_events: {} ({}{})",
                session.core.block_io_event_count,
                block_io_correlation_basis(session),
                if block_io_correlation_basis(session) == "dev+sector" {
                    " correlated (advisory, approximate)"
                } else {
                    " correlated"
                },
            ),
        );
        pushln(&mut output, "");

        if session.core.block_io_event_count > 0
            && block_io_correlation_basis(session) == "dev+sector"
        {
            pushln(&mut output, "block i/o correlation warning");
            pushln(&mut output, "----------------------------");
            pushln(
                &mut output,
                "note: Block I/O correlation uses dev+sector hashing. Attribution to specific",
            );
            pushln(
                &mut output,
                "      tasks is best-effort and may collide if multiple concurrent requests",
            );
            pushln(
                &mut output,
                "      target the same device and sector. Exact attribution is not guaranteed.",
            );
            pushln(&mut output, "");
        }
    }

    let truncated = session
        .tasks
        .iter()
        .filter(|task| task.latency.truncated_samples > 0)
        .collect::<Vec<_>>();

    if !truncated.is_empty() {
        pushln(&mut output, "percentile warnings");
        pushln(&mut output, "-------------------");
        for task in truncated.iter().take(top) {
            pushln(
                &mut output,
                format!(
                    "task={} comm={} truncated_samples={} percentile_scope={} note={}",
                    task.task,
                    task.comm,
                    task.latency.truncated_samples,
                    task.latency.percentile_scope,
                    percentile_warning_note(&task.latency.percentile_scope)
                ),
            );
        }
        pushln(&mut output, "");
    }

    let mut tasks = session
        .tasks
        .iter()
        .filter(|task| task.latency.samples > 0)
        .filter(|task| filter_class.is_none_or(|c| task.class == c))
        .collect::<Vec<_>>();

    tasks.sort_by_key(|task| std::cmp::Reverse(task.latency.max_ns));

    pushln(&mut output, "top tasks by max latency");
    pushln(&mut output, "------------------------");
    let duration_secs = session.core.duration_ms as f64 / 1000.0;
    for task in tasks.iter().take(top) {
        let spike_rate = if duration_secs > 0.0 {
            task.latency.over_1ms as f64 / duration_secs
        } else {
            0.0
        };
        pushln(
            &mut output,
            format!(
                "task={} active={} class={:?} comm={} process_pid={:?} samples={} max={} over_1ms={} over_2ms={} over_5ms={} spike_rate_per_s={:.1} percentile_scope={}{}",
                task.task,
                task.active,
                task.class,
                task.comm,
                task.process_pid,
                task.latency.samples,
                format_latency(task.latency.max_ns),
                task.latency.over_1ms,
                task.latency.over_2ms,
                task.latency.over_5ms,
                spike_rate,
                task.latency.percentile_scope,
                format_task_cpu_perf(task),
            ),
        );
    }
    pushln(&mut output, "");

    tasks.sort_by_key(|task| {
        (
            std::cmp::Reverse(task.latency.over_5ms),
            std::cmp::Reverse(task.latency.over_2ms),
            std::cmp::Reverse(task.latency.over_1ms),
            std::cmp::Reverse(task.latency.max_ns),
        )
    });

    pushln(&mut output, "top tasks by threshold counters");
    pushln(&mut output, "-------------------------------");
    for task in tasks.iter().take(top) {
        let spike_rate = if duration_secs > 0.0 {
            task.latency.over_1ms as f64 / duration_secs
        } else {
            0.0
        };
        pushln(
            &mut output,
            format!(
                "task={} active={} class={:?} comm={} over_5ms={} over_2ms={} over_1ms={} spike_rate_per_s={:.1} max={}",
                task.task,
                task.active,
                task.class,
                task.comm,
                task.latency.over_5ms,
                task.latency.over_2ms,
                task.latency.over_1ms,
                spike_rate,
                format_latency(task.latency.max_ns),
            ),
        );
    }
    pushln(&mut output, "");

    pushln(&mut output, "top spikes");
    pushln(&mut output, "----------");
    for spike in session.top_spikes.iter().take(top) {
        pushln(
            &mut output,
            format!(
                "task={} active={} class={:?} comm={} cpu={} wakeup_target_cpu={} latency={} wakeup_ns={} switch_ns={} observed_runnable_depth={} target_pending_wakeups={}(diagnostic) switch_prev_pid={} switch_prev_state={} switch_prev_state_label={}",
                spike.task,
                spike.active,
                spike.class,
                spike.comm,
                spike.cpu,
                spike.wakeup_target_cpu,
                format_latency(spike.latency_ns),
                spike.wakeup_ns,
                spike.switch_ns,
                spike.observed_runnable_depth,
                spike.target_pending_wakeups,
                spike.switch_prev_pid,
                spike.switch_prev_state,
                classify_switch_prev_state(spike.switch_prev_state),
            ),
        );
    }
    pushln(&mut output, "");

    let cluster_window_ns = cluster_window_ms.saturating_mul(1_000_000);

    pushln(&mut output, "spike clusters");
    pushln(&mut output, "--------------");
    pushln(
        &mut output,
        render_cluster_source(cluster_analysis, cluster_window_ms),
    );
    pushln(
        &mut output,
        "observed_runnable_depth is an approximation of runnable pressure on the CPU reconstructed",
    );
    pushln(
        &mut output,
        "from sched tracepoints. target_pending_wakeups is diagnostic-only monitored-target backlog.",
    );
    pushln(
        &mut output,
        "It is not kernel runqueue depth and must not be used for scoring or tuning decisions.",
    );
    pushln(&mut output, "");
    if cluster_analysis.clusters.is_empty() {
        pushln(
            &mut output,
            format!(
                "none min_tasks={} window_ms={}",
                MIN_CLUSTER_TASKS, cluster_window_ms
            ),
        );
    } else {
        for (rank, cluster) in cluster_analysis.clusters.iter().take(top).enumerate() {
            pushln(&mut output, render_cluster(rank + 1, cluster));
        }
    }
    pushln(&mut output, "");

    if !frame_diagnoses.is_empty() {
        pushln(&mut output, "frame spike diagnoses");
        pushln(&mut output, "---------------------");
        for (rank, diag) in frame_diagnoses.iter().take(top).enumerate() {
            pushln(&mut output, render_frame_diagnosis(rank + 1, diag));
        }
        pushln(&mut output, "");
    }

    render_correlation_sections(
        &mut output,
        &cluster_analysis.clusters,
        artifacts,
        block_io_correlation_basis(session),
        cluster_window_ns,
        top,
    );

    output
}

fn pushln(output: &mut String, line: impl AsRef<str>) {
    output.push_str(line.as_ref());
    output.push('\n');
}

fn pressure_timeline_has_pressure(summary: &PressureTimelineSummary) -> bool {
    summary.sample_count > 0
        && (summary.max_cpu_some > 0.0
            || summary.max_mem_some.unwrap_or(0.0) > 0.0
            || summary.max_mem_full.unwrap_or(0.0) > 0.0
            || summary.max_io_some.unwrap_or(0.0) > 0.0
            || summary.max_io_full.unwrap_or(0.0) > 0.0)
}

fn render_pressure_timeline_summary(summary: &PressureTimelineSummary) -> String {
    let mut output = String::new();
    let windows_near_spikes = summary
        .windows
        .iter()
        .filter(|window| window.near_spike)
        .count();

    pushln(&mut output, "pressure timeline");
    pushln(&mut output, "-----------------");
    pushln(
        &mut output,
        format!(
            "samples={} windows_near_spikes={} max_cpu_some={:.2}",
            summary.sample_count, windows_near_spikes, summary.max_cpu_some
        ),
    );
    pushln(
        &mut output,
        format!(
            "max_mem_some={} max_mem_full={} max_io_some={} max_io_full={}",
            format_pressure_option(summary.max_mem_some),
            format_pressure_option(summary.max_mem_full),
            format_pressure_option(summary.max_io_some),
            format_pressure_option(summary.max_io_full),
        ),
    );

    output
}

fn format_pressure_option(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.2}"))
        .unwrap_or_else(|| "-".to_owned())
}

fn block_io_correlation_basis(session: &SessionFile) -> &str {
    if session.core.block_io_correlation_basis.is_empty() {
        "dev+sector"
    } else {
        &session.core.block_io_correlation_basis
    }
}

pub(crate) fn data_quality_summary(
    session: &SessionFile,
    validation: &crate::session_io::RunValidationReport,
) -> DataQualitySummary {
    let mut reasons = Vec::new();
    let mut level = DataQualityLevel::High;

    if !validation.errors.is_empty() {
        level = DataQualityLevel::Low;
        reasons.push("run directory has validation errors".to_owned());
    }

    if session.core.schema_version != SESSION_SCHEMA_VERSION {
        if session.core.schema_version > SESSION_SCHEMA_VERSION {
            level = DataQualityLevel::Low;
            reasons.push("session schema is newer than this stutter binary".to_owned());
        } else {
            level = downgrade_quality(level, DataQualityLevel::Medium);
            reasons.push("session schema is older than this stutter binary".to_owned());
        }
    }

    if session.core.event_stream_write_errors > 0 {
        level = DataQualityLevel::Low;
        reasons.push("recording stream had write errors".to_owned());
    }

    let missing_non_focus_optional = validation
        .missing_optional_files
        .iter()
        .filter(|f| {
            *f != crate::session_io::FOCUS_EVENTS_FILE
                && *f != crate::session_io::FOREGROUND_EVENTS_FILE
        })
        .collect::<Vec<_>>();

    if !missing_non_focus_optional.is_empty() {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("optional correlation artifacts are missing".to_owned());
    }

    if session.core.spike_events_truncated {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("spike event stream was truncated".to_owned());
    }

    if session.core.spike_events_dropped_count > 0 {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("spike events were dropped".to_owned());
    }

    if session.core.interval_record_count == 0 {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("no interval records are available".to_owned());
    }

    if session.core.active_target_pids_count == 0 {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("no active target tasks were present at end of run".to_owned());
    }

    let drop_counters_nonzero = session.core.drop_counters.total() > 0;

    if drop_counters_nonzero {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("eBPF drop counters are non-zero".to_owned());
    }

    let mut percentile_scope_counts = BTreeMap::new();
    for task in &session.tasks {
        *percentile_scope_counts
            .entry(task.latency.percentile_scope.clone())
            .or_insert(0) += 1;
    }

    if percentile_scope_counts.contains_key("histogram") {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("some percentile values are histogram-estimated".to_owned());
    }

    if percentile_scope_counts.contains_key("capped_prefix") {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("some percentile values are based on capped prefix samples".to_owned());
    }

    let block_io_correlation_basis = block_io_correlation_basis(session).to_owned();
    if session.core.block_io_event_count > 0 && block_io_correlation_basis == "dev+sector" {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("block I/O correlation is approximate dev+sector matching".to_owned());
    }

    let frame_timestamp_alignment = if session.core.frame_event_count == 0 {
        "none".to_owned()
    } else if session.core.mangohud_first_frame_monotonic_ns.is_some() {
        "monotonic_observed".to_owned()
    } else {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("MangoHud frame timestamp alignment is approximate".to_owned());
        "approximate_first_row".to_owned()
    };

    let cpu_perf_requested = session.config.cpu_perf;
    let task_cpu_perf_count = session
        .tasks
        .iter()
        .filter(|task| task.cpu_perf.is_some())
        .count();
    if cpu_perf_requested && task_cpu_perf_count == 0 {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("CPU perf was requested but no counters were recorded".to_owned());
    }
    if session.core.cpu_perf_open_errors > 0 || session.core.cpu_perf_read_errors > 0 {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("CPU perf counters had open/read errors".to_owned());
    }
    if session.core.cpu_perf_skipped_tasks > 0 {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push(format!(
            "CPU perf skipped {} active tasks due to cpu_perf_max_tasks limit",
            session.core.cpu_perf_skipped_tasks
        ));
    }
    if session
        .tasks
        .iter()
        .filter_map(|task| task.cpu_perf.as_ref())
        .any(|perf| perf.multiplexed)
    {
        level = downgrade_quality(level, DataQualityLevel::Medium);
        reasons.push("CPU perf counters were multiplexed; values are scaled estimates".to_owned());
    }

    if reasons.is_empty() {
        reasons.push("no data-quality problems detected".to_owned());
    }

    DataQualitySummary {
        level,
        reasons,
        missing_optional_files: validation.missing_optional_files.clone(),
        validation_errors: validation.errors.clone(),
        validation_warnings: validation.warnings.clone(),
        schema_version: session.core.schema_version,
        expected_schema_version: SESSION_SCHEMA_VERSION,
        event_stream_write_errors: session.core.event_stream_write_errors,
        spike_events_truncated: session.core.spike_events_truncated,
        spike_events_retained_count: session.core.spike_events_retained_count,
        spike_events_dropped_count: session.core.spike_events_dropped_count,
        interval_record_count: session.core.interval_record_count,
        active_target_pids_count: session.core.active_target_pids_count,
        drop_counters_nonzero,
        percentile_scope_counts,
        block_io_correlation_basis,
        frame_timestamp_alignment,
        cpu_perf_requested,
        cpu_perf_open_errors: session.core.cpu_perf_open_errors,
        cpu_perf_read_errors: session.core.cpu_perf_read_errors,
        cpu_perf_skipped_tasks: session.core.cpu_perf_skipped_tasks,
    }
}

fn build_pressure_timeline(
    intervals: &[IntervalRecord],
    clusters: &[SpikeCluster],
    cluster_window_ms: u64,
) -> PressureTimelineSummary {
    if intervals.is_empty() {
        return PressureTimelineSummary {
            sample_count: 0,
            max_cpu_some: 0.0,
            max_mem_some: None,
            max_mem_full: None,
            max_io_some: None,
            max_io_full: None,
            windows: Vec::new(),
            peak_windows: Vec::new(),
            pressure_notes: vec![
                "No interval records loaded; pressure timeline unavailable".to_owned(),
            ],
            coverage: PressureTimelineCoverage::default(),
        };
    }

    let mut sorted_intervals = intervals.iter().collect::<Vec<_>>();
    sorted_intervals.sort_by_key(|record| record.elapsed_ms);

    let mut windows = Vec::with_capacity(sorted_intervals.len());
    let mut peak_windows = Vec::new();

    let mut max_cpu_some = 0.0_f64;
    let mut max_mem_some = 0.0_f64;
    let mut max_mem_full = 0.0_f64;
    let mut max_io_some = 0.0_f64;
    let mut max_io_full = 0.0_f64;

    let mut has_mem_psi = false;
    let mut has_io_psi = false;
    let mut has_near_spike_windows = false;

    for record in sorted_intervals {
        let near_spike = pressure_window_near_spike(record.elapsed_ms, clusters, cluster_window_ms);

        has_near_spike_windows |= near_spike;
        has_mem_psi |= record.mem_psi_some > 0.0 || record.mem_psi_full > 0.0;
        has_io_psi |= record.io_psi_some > 0.0 || record.io_psi_full > 0.0;

        max_cpu_some = max_cpu_some.max(record.cpu_psi_some);
        max_mem_some = max_mem_some.max(record.mem_psi_some);
        max_mem_full = max_mem_full.max(record.mem_psi_full);
        max_io_some = max_io_some.max(record.io_psi_some);
        max_io_full = max_io_full.max(record.io_psi_full);

        push_pressure_peak_window(
            &mut peak_windows,
            PressurePeakWindow {
                elapsed_ms: record.elapsed_ms,
                pressure_kind: PressureKind::CpuSome,
                value: record.cpu_psi_some,
                near_spike,
            },
        );
        push_pressure_peak_window(
            &mut peak_windows,
            PressurePeakWindow {
                elapsed_ms: record.elapsed_ms,
                pressure_kind: PressureKind::MemSome,
                value: record.mem_psi_some,
                near_spike,
            },
        );
        push_pressure_peak_window(
            &mut peak_windows,
            PressurePeakWindow {
                elapsed_ms: record.elapsed_ms,
                pressure_kind: PressureKind::MemFull,
                value: record.mem_psi_full,
                near_spike,
            },
        );
        push_pressure_peak_window(
            &mut peak_windows,
            PressurePeakWindow {
                elapsed_ms: record.elapsed_ms,
                pressure_kind: PressureKind::IoSome,
                value: record.io_psi_some,
                near_spike,
            },
        );
        push_pressure_peak_window(
            &mut peak_windows,
            PressurePeakWindow {
                elapsed_ms: record.elapsed_ms,
                pressure_kind: PressureKind::IoFull,
                value: record.io_psi_full,
                near_spike,
            },
        );

        windows.push(PressureWindow {
            elapsed_ms: record.elapsed_ms,
            cpu_some: record.cpu_psi_some,
            mem_some: Some(record.mem_psi_some),
            mem_full: Some(record.mem_psi_full),
            io_some: Some(record.io_psi_some),
            io_full: Some(record.io_psi_full),
            near_spike,
        });
    }

    peak_windows.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.elapsed_ms.cmp(&b.elapsed_ms))
    });
    peak_windows.truncate(MAX_PRESSURE_PEAK_WINDOWS);

    let max_mem_some = has_mem_psi.then_some(max_mem_some);
    let max_mem_full = has_mem_psi.then_some(max_mem_full);
    let max_io_some = has_io_psi.then_some(max_io_some);
    let max_io_full = has_io_psi.then_some(max_io_full);

    let coverage = PressureTimelineCoverage {
        interval_records_loaded: windows.len(),
        has_cpu_psi: true,
        has_mem_psi,
        has_io_psi,
        has_near_spike_windows,
    };

    let pressure_notes = build_pressure_notes(
        max_cpu_some,
        max_mem_some,
        max_mem_full,
        max_io_some,
        max_io_full,
        has_mem_psi,
        has_io_psi,
        &peak_windows,
    );

    PressureTimelineSummary {
        sample_count: windows.len(),
        max_cpu_some,
        max_mem_some,
        max_mem_full,
        max_io_some,
        max_io_full,
        windows,
        peak_windows,
        pressure_notes,
        coverage,
    }
}

fn pressure_window_near_spike(
    elapsed_ms: u64,
    clusters: &[SpikeCluster],
    cluster_window_ms: u64,
) -> bool {
    clusters.iter().any(|cluster| {
        cluster
            .points
            .iter()
            .filter_map(|point| point.elapsed_ms)
            .any(|cluster_elapsed_ms| elapsed_ms.abs_diff(cluster_elapsed_ms) <= cluster_window_ms)
    })
}

fn push_pressure_peak_window(
    peak_windows: &mut Vec<PressurePeakWindow>,
    peak_window: PressurePeakWindow,
) {
    if peak_window.value <= 0.0 {
        return;
    }

    peak_windows.push(peak_window);
}

#[allow(clippy::too_many_arguments)]
fn build_pressure_notes(
    max_cpu_some: f64,
    max_mem_some: Option<f64>,
    max_mem_full: Option<f64>,
    max_io_some: Option<f64>,
    max_io_full: Option<f64>,
    has_mem_psi: bool,
    has_io_psi: bool,
    peak_windows: &[PressurePeakWindow],
) -> Vec<String> {
    let mut notes = Vec::new();

    push_pressure_note_if_above(
        &mut notes,
        PressureKind::CpuSome,
        max_cpu_some,
        PRESSURE_NOTE_CPU_SOME,
        peak_windows,
        "CPU pressure",
    );

    if let Some(value) = max_mem_some {
        push_pressure_note_if_above(
            &mut notes,
            PressureKind::MemSome,
            value,
            PRESSURE_NOTE_MEM_SOME,
            peak_windows,
            "Memory pressure",
        );
    }
    if let Some(value) = max_mem_full {
        push_pressure_note_if_above(
            &mut notes,
            PressureKind::MemFull,
            value,
            PRESSURE_NOTE_MEM_FULL,
            peak_windows,
            "Memory full pressure",
        );
    }
    if let Some(value) = max_io_some {
        push_pressure_note_if_above(
            &mut notes,
            PressureKind::IoSome,
            value,
            PRESSURE_NOTE_IO_SOME,
            peak_windows,
            "I/O pressure",
        );
    }
    if let Some(value) = max_io_full {
        push_pressure_note_if_above(
            &mut notes,
            PressureKind::IoFull,
            value,
            PRESSURE_NOTE_IO_FULL,
            peak_windows,
            "I/O full pressure",
        );
    }

    if !has_mem_psi {
        notes.push("Memory PSI fields were not present in loaded intervals".to_owned());
    }
    if !has_io_psi {
        notes.push("I/O PSI fields were not present in loaded intervals".to_owned());
    }

    notes
}

fn push_pressure_note_if_above(
    notes: &mut Vec<String>,
    pressure_kind: PressureKind,
    value: f64,
    threshold: f64,
    peak_windows: &[PressurePeakWindow],
    label: &str,
) {
    if value < threshold {
        return;
    }

    let near_spike = peak_windows.iter().any(|peak_window| {
        pressure_kind_label(&peak_window.pressure_kind) == pressure_kind_label(&pressure_kind)
            && (peak_window.value - value).abs() <= f64::EPSILON
            && peak_window.near_spike
    });

    if near_spike {
        notes.push(format!(
            "{label} reached {:.1}% near a scheduler spike",
            value
        ));
    } else {
        notes.push(format!("{label} reached {:.1}%", value));
    }
}

fn pressure_kind_label(pressure_kind: &PressureKind) -> &'static str {
    match pressure_kind {
        PressureKind::CpuSome => "cpu_some",
        PressureKind::MemSome => "mem_some",
        PressureKind::MemFull => "mem_full",
        PressureKind::IoSome => "io_some",
        PressureKind::IoFull => "io_full",
    }
}

fn downgrade_quality(current: DataQualityLevel, candidate: DataQualityLevel) -> DataQualityLevel {
    use DataQualityLevel::{High, Low, Medium};

    match (current, candidate) {
        (Low, _) | (_, Low) => Low,
        (Medium, _) | (_, Medium) => Medium,
        (High, High) => High,
    }
}

pub(crate) fn spike_cluster_analysis(
    session: &SessionFile,
    spike_events: Option<&[SpikeEvent]>,
    cluster_window_ns: u64,
    top: usize,
    filter_class: Option<TaskClass>,
) -> SpikeClusterAnalysis {
    let (source, mut points) = match spike_events {
        Some(spike_events) => (
            SpikeClusterSource::SpikeEvents,
            flatten_spike_events(session, spike_events),
        ),
        None => (
            SpikeClusterSource::TopSpikesFallback,
            flatten_top_spikes(session),
        ),
    };

    if let Some(class) = filter_class {
        points.retain(|p| p.class == class);
    }

    let source_count = points.len();

    SpikeClusterAnalysis {
        source,
        source_count,
        clusters: spike_clusters_from_points(points, cluster_window_ns, top),
    }
}

fn spike_clusters_from_points(
    mut points: Vec<SpikePoint>,
    cluster_window_ns: u64,
    top: usize,
) -> Vec<SpikeCluster> {
    points.sort_by_key(|point| point.switch_ns);

    let mut candidates = BinaryHeap::new();
    let mut task_counts = BTreeMap::<u32, usize>::new();
    let mut max_latency_candidates = std::collections::VecDeque::<usize>::new();
    let mut left_idx = 0;

    for right_idx in 0..points.len() {
        *task_counts.entry(points[right_idx].task).or_default() += 1;

        while max_latency_candidates
            .back()
            .is_some_and(|idx| points[*idx].latency_ns <= points[right_idx].latency_ns)
        {
            max_latency_candidates.pop_back();
        }
        max_latency_candidates.push_back(right_idx);

        while left_idx <= right_idx
            && points[right_idx]
                .switch_ns
                .saturating_sub(points[left_idx].switch_ns)
                > cluster_window_ns
        {
            decrement_task_count(&mut task_counts, points[left_idx].task);
            if max_latency_candidates.front() == Some(&left_idx) {
                max_latency_candidates.pop_front();
            }
            left_idx += 1;
        }

        if task_counts.len() >= MIN_CLUSTER_TASKS {
            let max_latency_ns = max_latency_candidates
                .front()
                .map(|idx| points[*idx].latency_ns)
                .unwrap_or(0);

            let candidate = SpikeClusterCandidate {
                start_idx: left_idx,
                end_idx: right_idx + 1,
                distinct_tasks: task_counts.len(),
                min_switch_ns: points[left_idx].switch_ns,
                max_switch_ns: points[right_idx].switch_ns,
                max_latency_ns,
            };

            if candidates.len() < MAX_CLUSTER_CANDIDATES {
                candidates.push(std::cmp::Reverse(candidate));
            } else if let Some(mut worst) = candidates.peek_mut()
                && candidate > worst.0
            {
                *worst = std::cmp::Reverse(candidate);
            }
        }
    }

    let mut candidates_vec: Vec<_> = candidates.into_iter().map(|r| r.0).collect();
    candidates_vec.sort_by(|a, b| b.cmp(a));

    let mut selected_candidates = Vec::new();
    let max_selected = top.saturating_mul(4).min(MAX_CLUSTER_CANDIDATES);

    // Sweep-line: track selected intervals by max_switch_ns in a BTreeSet
    // for O(log n) overlap checking instead of O(n) per candidate.
    let mut selected_intervals: BTreeSet<(u64, u64)> = BTreeSet::new(); // (max_switch_ns, min_switch_ns)

    for candidate in candidates_vec {
        // Check for overlap: we need intervals where
        //   existing.min_switch_ns <= candidate.max_switch_ns AND
        //   existing.max_switch_ns >= candidate.min_switch_ns
        //
        // Since intervals are stored as (max_switch_ns, min_switch_ns),
        // we look for entries whose max_switch_ns >= candidate.min_switch_ns.
        let overlaps = selected_intervals
            .range((candidate.min_switch_ns, 0)..)
            .any(|(_, min_ns)| *min_ns <= candidate.max_switch_ns);

        if !overlaps {
            selected_intervals.insert((candidate.max_switch_ns, candidate.min_switch_ns));
            selected_candidates.push(candidate);
            if selected_candidates.len() >= max_selected {
                break;
            }
        }
    }

    selected_candidates
        .into_iter()
        .map(|candidate| {
            cluster_from_points(
                points[candidate.start_idx..candidate.end_idx].to_vec(),
                candidate.distinct_tasks,
            )
        })
        .collect()
}

fn flatten_spike_events(session: &SessionFile, spike_events: &[SpikeEvent]) -> Vec<SpikePoint> {
    spike_events
        .iter()
        .map(|spike| SpikePoint {
            task: spike.task,
            class: spike.class,
            process_pid: spike.process_pid,
            comm: spike.comm.clone(),
            cpu: spike.cpu,
            wakeup_target_cpu: spike.wakeup_target_cpu,
            latency_ns: spike.latency_ns,
            wakeup_ns: spike.wakeup_ns,
            switch_ns: spike.switch_ns,
            target_pending_wakeups: spike.target_pending_wakeups,
            observed_runnable_depth: spike.observed_runnable_depth,
            switch_prev_pid: spike.switch_prev_pid,
            switch_prev_state: spike.switch_prev_state,
            switch_prev_state_label: classify_switch_prev_state(spike.switch_prev_state).to_owned(),
            elapsed_ms: elapsed_ms(session.core.monotonic_start_ns, spike.switch_ns)
                .or(spike.elapsed_ms),
            scx_ops: spike.scx_ops.clone(),
            scx_state: spike.scx_state.clone(),
            waker_tid: spike.waker_tid,
            waker_comm: spike.waker_comm.clone(),
            cause_tags: spike.cause_tags.clone(),
            primary_cause: spike.primary_cause.clone(),
        })
        .collect()
}

fn flatten_top_spikes(session: &SessionFile) -> Vec<SpikePoint> {
    let mut points = Vec::new();

    for task in &session.tasks {
        for spike in &task.top_spikes {
            points.push(spike_point_from_task(
                task,
                spike,
                elapsed_ms(session.core.monotonic_start_ns, spike.switch_ns),
            ));
        }
    }

    points
}

fn spike_point_from_task(
    task: &SessionTask,
    spike: &RecordedSpike,
    elapsed_ms: Option<u64>,
) -> SpikePoint {
    SpikePoint {
        task: task.task,
        class: spike.class,
        process_pid: spike.process_pid,
        comm: task.comm.clone(),
        cpu: spike.cpu,
        wakeup_target_cpu: spike.wakeup_target_cpu,
        switch_prev_pid: spike.switch_prev_pid,
        switch_prev_state: spike.switch_prev_state,
        switch_prev_state_label: classify_switch_prev_state(spike.switch_prev_state).to_owned(),
        latency_ns: spike.latency_ns,
        wakeup_ns: spike.wakeup_ns,
        switch_ns: spike.switch_ns,
        target_pending_wakeups: spike.target_pending_wakeups,
        observed_runnable_depth: spike.observed_runnable_depth,
        elapsed_ms,
        scx_ops: spike.scx_ops.clone(),
        scx_state: spike.scx_state.clone(),
        waker_tid: spike.waker_tid,
        waker_comm: spike.waker_comm.clone(),
        cause_tags: spike.cause_tags.clone(),
        primary_cause: spike.primary_cause.clone(),
    }
}

fn format_task_cpu_perf(task: &SessionTask) -> String {
    let Some(perf) = &task.cpu_perf else {
        return String::new();
    };

    let mut parts = Vec::new();
    if let Some(ipc) = perf.ipc {
        parts.push(format!("ipc={ipc:.2}"));
    }
    if let Some(cache_mpki) = perf.cache_mpki {
        parts.push(format!("cache_mpki={cache_mpki:.1}"));
    }
    if let Some(cache_miss_rate) = perf.cache_miss_rate {
        parts.push(format!("cache_miss_rate={:.1}%", cache_miss_rate * 100.0));
    }
    if let Some(cycles) = perf.cycles {
        parts.push(format!("cycles={cycles}"));
    }
    if let Some(instructions) = perf.instructions {
        parts.push(format!("instructions={instructions}"));
    }
    parts.push(format!("multiplexed={}", perf.multiplexed));

    format!(" cpu_perf: {}", parts.join(" "))
}

fn elapsed_ms(monotonic_start_ns: Option<u64>, switch_ns: u64) -> Option<u64> {
    let start_ns = monotonic_start_ns?;
    switch_ns
        .checked_sub(start_ns)
        .map(|elapsed_ns| elapsed_ns / 1_000_000)
}

fn decrement_task_count(task_counts: &mut BTreeMap<u32, usize>, task: u32) {
    let Some(count) = task_counts.get_mut(&task) else {
        return;
    };
    *count -= 1;
    if *count == 0 {
        task_counts.remove(&task);
    }
}

// retain_cluster_candidate removed as it's replaced by BinaryHeap logic above

// compare_cluster_candidates removed as it's replaced by Ord implementation

fn cluster_from_points(mut points: Vec<SpikePoint>, distinct_tasks: usize) -> SpikeCluster {
    points.sort_by_key(|point| (point.switch_ns, std::cmp::Reverse(point.latency_ns)));

    let min_switch_ns = points.first().map(|point| point.switch_ns).unwrap_or(0);
    let max_switch_ns = points.last().map(|point| point.switch_ns).unwrap_or(0);
    let max_latency_ns = points
        .iter()
        .map(|point| point.latency_ns)
        .max()
        .unwrap_or(0);

    let wake_graph = build_wake_graph(&points);

    SpikeCluster {
        points,
        distinct_tasks,
        min_switch_ns,
        max_switch_ns,
        max_latency_ns,
        diagnosis: None,
        anchor_task: None,
        anchor_class: None,
        anchor_comm: None,
        anchor_kind: None,
        foreground_pid: None,
        foreground_app_id: None,
        foreground_class: None,
        foreground_confidence: None,
        wake_graph,
    }
}

fn build_wake_graph(points: &[SpikePoint]) -> Vec<WakeGraphEdge> {
    let mut edges = BTreeMap::<(u32, String, u32, String), (u64, u64)>::new();

    for point in points {
        if point.waker_tid != 0 {
            let key = (
                point.waker_tid,
                point.waker_comm.clone(),
                point.task,
                point.comm.clone(),
            );
            let entry = edges.entry(key).or_insert((0, 0));
            entry.0 += 1;
            entry.1 = entry.1.max(point.latency_ns);
        }
    }

    let mut result: Vec<_> = edges
        .into_iter()
        .map(
            |((waker_tid, waker_comm, wakee_tid, wakee_comm), (count, max_latency_ns))| {
                WakeGraphEdge {
                    waker_tid,
                    waker_comm,
                    wakee_tid,
                    wakee_comm,
                    count,
                    max_latency_ns,
                }
            },
        )
        .collect();

    // Sort by count desc, then max_latency_ns desc
    result.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| b.max_latency_ns.cmp(&a.max_latency_ns))
    });

    result.truncate(16);
    result
}

fn perform_diagnosis(
    clusters: &mut [SpikeCluster],
    artifacts: &session_io::RunArtifacts,
    cluster_window_ns: u64,
) {
    for cluster in clusters {
        let diagnosis = diagnose_cluster(cluster, artifacts, cluster_window_ns);
        let anchor = select_anchor_for_diagnosis(cluster, &diagnosis);
        cluster.anchor_task = Some(anchor.task);
        cluster.anchor_class = Some(anchor.class);
        cluster.anchor_comm = Some(anchor.comm);
        cluster.anchor_kind = Some(anchor.kind);
        cluster.diagnosis = Some(diagnosis);
    }
}

fn annotate_clusters_with_foreground(
    clusters: &mut [SpikeCluster],
    foreground_events: &[ForegroundEvent],
    max_stale_ms: u64,
) {
    for cluster in clusters {
        if let Some(event) = foreground_for_cluster(cluster, foreground_events, max_stale_ms) {
            cluster.foreground_pid = event.pid;
            cluster.foreground_app_id = event.app_id.clone();
            cluster.foreground_class = event.class.clone();
            cluster.foreground_confidence = Some(event.confidence);
        }
    }
}

fn foreground_for_cluster<'a>(
    cluster: &SpikeCluster,
    foreground_events: &'a [ForegroundEvent],
    max_stale_ms: u64,
) -> Option<&'a ForegroundEvent> {
    let cluster_ms = cluster_elapsed_ms(cluster)?;
    foreground_events
        .iter()
        .filter(|event| event.elapsed_ms <= cluster_ms)
        .filter(|event| cluster_ms.saturating_sub(event.elapsed_ms) <= max_stale_ms)
        .max_by_key(|event| event.elapsed_ms)
}

fn cluster_elapsed_ms(cluster: &SpikeCluster) -> Option<u64> {
    cluster
        .points
        .iter()
        .filter_map(|point| point.elapsed_ms)
        .min()
}

fn render_cluster_source(analysis: &SpikeClusterAnalysis, cluster_window_ms: u64) -> String {
    let source = match analysis.source {
        SpikeClusterSource::SpikeEvents => "source=spike_events",
        SpikeClusterSource::TopSpikesFallback => "source=top_spikes fallback",
    };
    format!(
        "{source} count={} window_ms={} min_tasks={}",
        analysis.source_count, cluster_window_ms, MIN_CLUSTER_TASKS
    )
}

struct CorrelationCtx<'a> {
    output: &'a mut String,
    clusters: &'a [SpikeCluster],
    top: usize,
}

fn render_correlation<T, LP, MP, R>(
    ctx: &mut CorrelationCtx,
    title: &str,
    in_memory: &[T],
    mut load_predicate: LP,
    mut match_predicate: MP,
    mut render_closure: R,
) where
    T: Clone,
    LP: FnMut(&T) -> bool,
    MP: FnMut(&SpikeCluster, &T) -> bool,
    R: FnMut(&mut String, usize, &SpikeCluster, &[&T]),
{
    let pool: Vec<_> = in_memory
        .iter()
        .filter(|item| load_predicate(item))
        .collect();

    if pool.is_empty() {
        return;
    }

    pushln(ctx.output, title);
    pushln(ctx.output, "-".repeat(title.len()));

    for (rank, cluster) in ctx.clusters.iter().take(ctx.top).enumerate() {
        let matches: Vec<&T> = pool
            .iter()
            .copied()
            .filter(|item| match_predicate(cluster, item))
            .collect();

        if !matches.is_empty() {
            render_closure(ctx.output, rank, cluster, &matches);
        }
    }
    pushln(ctx.output, "");
}

fn render_correlation_sections(
    output: &mut String,
    clusters: &[SpikeCluster],
    artifacts: &session_io::RunArtifacts,
    block_io_correlation_basis: &str,
    cluster_window_ns: u64,
    top: usize,
) {
    let mut ctx = CorrelationCtx {
        output,
        clusters,
        top,
    };

    // 1. IRQ events
    let min_overall = clusters
        .iter()
        .map(|c| c.min_switch_ns.saturating_sub(cluster_window_ns))
        .min()
        .unwrap_or(0);
    let max_overall = clusters
        .iter()
        .map(|c| c.max_switch_ns.saturating_add(cluster_window_ns))
        .max()
        .unwrap_or(0);

    render_correlation(
        &mut ctx,
        "irq overlap",
        &artifacts.irq_events,
        |e| e.exit_ns >= min_overall && e.enter_ns <= max_overall,
        |cluster, event| {
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            event.exit_ns >= min_ns && event.enter_ns <= max_ns
        },
        |output, rank, cluster, matches| {
            let max_duration = matches.iter().map(|e| e.duration_ns).max().unwrap_or(0);
            let irq_list = matches
                .iter()
                .map(|e| e.irq)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|irq| irq.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            pushln(
                output,
                format!(
                    "cluster=#{} matches={} irqs={} max_duration={} window_ns={}..{}",
                    rank + 1,
                    matches.len(),
                    irq_list,
                    format_latency(max_duration),
                    min_ns,
                    max_ns
                ),
            );
        },
    );

    // 2. GPU samples
    let min_overall_opt = clusters.iter().filter_map(cluster_elapsed).min();
    let max_overall_opt = clusters.iter().filter_map(cluster_elapsed).max();
    if let (Some(min_overall), Some(max_overall)) = (min_overall_opt, max_overall_opt) {
        let lower = min_overall.saturating_sub(50);
        let upper = max_overall.saturating_add(50);
        render_correlation(
            &mut ctx,
            "gpu near clusters",
            &artifacts.gpu_samples,
            |s| s.elapsed_ms >= lower && s.elapsed_ms <= upper,
            |cluster, sample| {
                cluster_elapsed(cluster)
                    .is_some_and(|elapsed| sample.elapsed_ms.abs_diff(elapsed) <= 50)
            },
            |output, rank, cluster, matches| {
                let elapsed = cluster_elapsed(cluster).unwrap();
                let sample = matches
                    .iter()
                    .min_by_key(|s| s.elapsed_ms.abs_diff(elapsed))
                    .unwrap();
                pushln(
                    output,
                    format!(
                        "cluster=#{} sample_elapsed={} gpu_busy={} gpu_clock_mhz={} mem_clock_mhz={} temp_mC={} power_uW={}",
                        rank + 1,
                        format_elapsed(Some(sample.elapsed_ms)),
                        format_option(sample.gpu_busy_percent),
                        format_option(sample.gpu_clock_mhz),
                        format_option(sample.mem_clock_mhz),
                        format_option(sample.temp_millidegrees),
                        format_option(sample.power_microwatts),
                    ),
                );
            },
        );
    }

    // 3. Frame events
    let padding_ms = (cluster_window_ns / 1_000_000).max(1);
    let min_overall_opt = clusters
        .iter()
        .filter_map(|c| cluster_elapsed_range(c).map(|(min, _)| min))
        .min();
    let max_overall_opt = clusters
        .iter()
        .filter_map(|c| cluster_elapsed_range(c).map(|(_, max)| max))
        .max();
    if let (Some(min_overall), Some(max_overall)) = (min_overall_opt, max_overall_opt) {
        let lower = min_overall.saturating_sub(padding_ms);
        let upper = max_overall.saturating_add(padding_ms);
        render_correlation(
            &mut ctx,
            "frame overlap",
            &artifacts.frame_events,
            |f| f.elapsed_ms >= lower && f.elapsed_ms <= upper,
            |cluster, frame| {
                cluster_elapsed_range(cluster).is_some_and(|(min_e, max_e)| {
                    let min_e = min_e.saturating_sub(padding_ms);
                    let max_e = max_e.saturating_add(padding_ms);
                    frame.elapsed_ms >= min_e && frame.elapsed_ms <= max_e
                })
            },
            |output, rank, cluster, matches| {
                let (min_e, max_e) = cluster_elapsed_range(cluster).unwrap();
                let min_e = min_e.saturating_sub(padding_ms);
                let max_e = max_e.saturating_add(padding_ms);
                let max_frame = matches
                    .iter()
                    .map(|f| f.frametime_ms)
                    .fold(0.0_f64, f64::max);
                pushln(
                    output,
                    format!(
                        "cluster=#{} frames={} max_frametime_ms={:.3} elapsed={}..{}",
                        rank + 1,
                        matches.len(),
                        max_frame,
                        min_e,
                        max_e
                    ),
                );
            },
        );
    }

    // 4. Migration events
    let min_overall = clusters
        .iter()
        .map(|c| c.min_switch_ns.saturating_sub(cluster_window_ns))
        .min()
        .unwrap_or(0);
    let max_overall = clusters
        .iter()
        .map(|c| c.max_switch_ns.saturating_add(cluster_window_ns))
        .max()
        .unwrap_or(0);

    render_correlation(
        &mut ctx,
        "migration overlap",
        &artifacts.migration_events,
        |e| e.timestamp_ns >= min_overall && e.timestamp_ns <= max_overall,
        |cluster, event| {
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            event.timestamp_ns >= min_ns && event.timestamp_ns <= max_ns
        },
        |output, rank, cluster, matches| {
            let tids = matches
                .iter()
                .map(|e| e.tid)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|tid| tid.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            pushln(
                output,
                format!(
                    "cluster=#{} matches={} tids={} window_ns={}..{}",
                    rank + 1,
                    matches.len(),
                    tids,
                    min_ns,
                    max_ns
                ),
            );
        },
    );

    // 5. CPU freq samples
    let min_overall_opt = clusters.iter().filter_map(cluster_elapsed).min();
    let max_overall_opt = clusters.iter().filter_map(cluster_elapsed).max();
    if let (Some(min_overall), Some(max_overall)) = (min_overall_opt, max_overall_opt) {
        let lower = min_overall.saturating_sub(50);
        let upper = max_overall.saturating_add(50);
        render_correlation(
            &mut ctx,
            "cpu freq near clusters",
            &artifacts.cpu_freq_events,
            |s| s.elapsed_ms >= lower && s.elapsed_ms <= upper,
            |cluster, sample| {
                cluster_elapsed(cluster)
                    .is_some_and(|elapsed| sample.elapsed_ms.abs_diff(elapsed) <= 50)
            },
            |output, rank, _, matches| {
                let max_freq = matches.iter().map(|s| s.freq_khz).max().unwrap_or(0);
                pushln(
                    output,
                    format!(
                        "cluster=#{} cpu_freq_samples={} max_freq_khz={}",
                        rank + 1,
                        matches.len(),
                        max_freq
                    ),
                );
            },
        );
    }

    // 6. Block I/O events
    let min_overall = clusters
        .iter()
        .map(|c| c.min_switch_ns.saturating_sub(cluster_window_ns))
        .min()
        .unwrap_or(0);
    let max_overall = clusters
        .iter()
        .map(|c| c.max_switch_ns.saturating_add(cluster_window_ns))
        .max()
        .unwrap_or(0);

    let io_title = if block_io_correlation_basis == "dev+sector" {
        "block i/o overlap (advisory, approximate; correlated by dev+sector)"
    } else {
        "block i/o overlap (correlated by request-pointer)"
    };

    render_correlation(
        &mut ctx,
        io_title,
        &artifacts.block_io_events,
        |e| {
            e.timestamp_ns >= min_overall
                && e.timestamp_ns.saturating_sub(e.duration_ns) <= max_overall
        },
        |cluster, event| {
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            event.timestamp_ns >= min_ns
                && event.timestamp_ns.saturating_sub(event.duration_ns) <= max_ns
        },
        |output, rank, cluster, matches| {
            let max_duration = matches.iter().map(|e| e.duration_ns).max().unwrap_or(0);
            let tids = matches
                .iter()
                .map(|e| e.tid)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .map(|tid| tid.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
            let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
            pushln(
                output,
                format!(
                    "cluster=#{} matches={} tids={}{} max_duration={} window_ns={}..{}",
                    rank + 1,
                    matches.len(),
                    tids,
                    if block_io_correlation_basis == "dev+sector" {
                        " (approximate)"
                    } else {
                        ""
                    },
                    format_latency(max_duration),
                    min_ns,
                    max_ns
                ),
            );
        },
    );

    // 7. SCX transitions
    let min_overall_opt = clusters.iter().filter_map(cluster_elapsed).min();
    let max_overall_opt = clusters.iter().filter_map(cluster_elapsed).max();
    if let (Some(min_overall), Some(max_overall)) = (min_overall_opt, max_overall_opt) {
        let lower = min_overall.saturating_sub(2000);
        let upper = max_overall.saturating_add(2000);
        render_correlation(
            &mut ctx,
            "scx transitions near clusters",
            &artifacts.scx_events,
            |e| e.elapsed_ms >= lower && e.elapsed_ms <= upper,
            |cluster, event| {
                cluster_elapsed(cluster)
                    .is_some_and(|elapsed| event.elapsed_ms.abs_diff(elapsed) <= 2000)
            },
            |output, rank, cluster, matches| {
                let elapsed = cluster_elapsed(cluster).unwrap();
                for event in matches {
                    pushln(
                        output,
                        format!(
                            "cluster=#{} SCX transition near spike: ops={} state={} at elapsed={}ms (cluster_elapsed={}ms)",
                            rank + 1,
                            event.ops.as_deref().unwrap_or("-"),
                            event.state.as_deref().unwrap_or("-"),
                            event.elapsed_ms,
                            elapsed
                        ),
                    );
                }
            },
        );
    }
}

fn cluster_elapsed_range(cluster: &SpikeCluster) -> Option<(u64, u64)> {
    let mut elapsed = cluster.points.iter().filter_map(|point| point.elapsed_ms);
    let first = elapsed.next()?;
    let mut min_elapsed = first;
    let mut max_elapsed = first;
    for value in elapsed {
        min_elapsed = min_elapsed.min(value);
        max_elapsed = max_elapsed.max(value);
    }
    Some((min_elapsed, max_elapsed))
}

fn format_option<T: std::fmt::Display>(value: Option<T>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

fn render_cluster(rank: usize, cluster: &SpikeCluster) -> String {
    let labels = cluster_labels(cluster);
    let labels = if labels.is_empty() {
        "-".to_owned()
    } else {
        labels.join(",")
    };
    let span_ns = cluster.max_switch_ns.saturating_sub(cluster.min_switch_ns);
    let elapsed = cluster_elapsed(cluster);
    let cpu_list = cluster
        .points
        .iter()
        .map(|point| point.cpu)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|cpu| cpu.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let shown_points = cluster.points.len().min(MAX_INLINE_CLUSTER_POINTS);
    let omitted_points = cluster.points.len().saturating_sub(shown_points);
    let points = cluster
        .points
        .iter()
        .take(MAX_INLINE_CLUSTER_POINTS)
        .map(render_cluster_point)
        .collect::<Vec<_>>()
        .join(" ");

    let diagnosis_line = if let Some(d) = &cluster.diagnosis {
        format!("\n{}", render_diagnosis_lines(d, "  "))
    } else {
        String::new()
    };

    let wake_block = if cluster.wake_graph.is_empty() {
        String::new()
    } else {
        let mut wake_lines = Vec::new();
        wake_lines.push("\n  wake relationships:".to_owned());
        for edge in &cluster.wake_graph {
            wake_lines.push(format!(
                "    {} [{}] woke {} [{}] (count={}, max_lat={})",
                edge.waker_comm,
                edge.waker_tid,
                edge.wakee_comm,
                edge.wakee_tid,
                edge.count,
                format_latency(edge.max_latency_ns)
            ));
        }
        wake_lines.join("\n")
    };

    format!(
        "#{rank} elapsed={} span={} tasks={} total_spikes={} shown_points={} omitted_points={} cpus={} labels={} max={} switch_ns={}..{} points={}{}{}",
        format_elapsed(elapsed),
        format_latency(span_ns),
        cluster.distinct_tasks,
        cluster.points.len(),
        shown_points,
        omitted_points,
        cpu_list,
        labels,
        format_latency(cluster.max_latency_ns),
        cluster.min_switch_ns,
        cluster.max_switch_ns,
        points,
        diagnosis_line,
        wake_block
    )
}

fn render_diagnosis_lines(diagnosis: &Diagnosis, indent: &str) -> String {
    let mut output = String::new();
    pushln(
        &mut output,
        format!("{indent}diagnosis: {}", diagnosis.report_summary()),
    );
    output.push_str(&render_diagnosis_detail_lines(diagnosis, indent));
    output.trim_end().to_owned()
}

fn render_diagnosis_detail_lines(diagnosis: &Diagnosis, indent: &str) -> String {
    let mut output = String::new();
    if !diagnosis.secondary_causes.is_empty() {
        pushln(
            &mut output,
            format!(
                "{indent}diagnosis_secondary causes={:?}",
                diagnosis.secondary_causes
            ),
        );
    }

    for candidate in diagnosis.candidates.iter().take(3) {
        pushln(
            &mut output,
            format!(
                "{indent}diagnosis_candidate cause={:?} confidence={:?} score={:.2}",
                candidate.cause, candidate.confidence, candidate.score
            ),
        );

        for evidence in candidate.evidence.iter().take(6) {
            pushln(
                &mut output,
                format!(
                    "{indent}  evidence kind={:?} strength={:.2} msg={}",
                    evidence.kind, evidence.strength, evidence.message
                ),
            );
        }
    }

    for missing in diagnosis.missing_evidence.iter().take(6) {
        pushln(
            &mut output,
            format!("{indent}diagnosis_missing_evidence msg={missing}"),
        );
    }

    output.trim_end().to_owned()
}

fn render_cluster_point(point: &SpikePoint) -> String {
    let scx = if let Some(ops) = &point.scx_ops {
        format!(" scx_ops={ops}")
    } else {
        String::new()
    };
    let prev_label = classify_switch_prev_state(point.switch_prev_state);
    format!(
        "{}({:?}:{} cpu={} wakeup_target_cpu={} latency={} switch_ns={} process_pid={} wakeup_ns={} observed_runnable_depth={} target_pending_wakeups={}(diag) switch_prev_pid={} switch_prev_state={} switch_prev_state_label={}{}{}{})",
        point.task,
        point.class,
        point.comm,
        point.cpu,
        point.wakeup_target_cpu,
        format_latency(point.latency_ns),
        point.switch_ns,
        format_process_pid(point.process_pid),
        point.wakeup_ns,
        point.observed_runnable_depth,
        point.target_pending_wakeups,
        point.switch_prev_pid,
        point.switch_prev_state,
        prev_label,
        scx,
        if let Some(p) = &point.primary_cause {
            format!(" primary_cause={}", p)
        } else {
            String::new()
        },
        if !point.cause_tags.is_empty() {
            format!(" tags={}", point.cause_tags.join(","))
        } else {
            String::new()
        }
    )
}

fn format_process_pid(process_pid: Option<u32>) -> String {
    match process_pid {
        Some(process_pid) => process_pid.to_string(),
        None => "-".to_owned(),
    }
}

fn cluster_elapsed(cluster: &SpikeCluster) -> Option<u64> {
    cluster
        .points
        .iter()
        .filter_map(|point| point.elapsed_ms)
        .min()
}

fn format_elapsed(elapsed_ms: Option<u64>) -> String {
    match elapsed_ms {
        Some(elapsed_ms) => format!("{elapsed_ms}ms"),
        None => "-".to_owned(),
    }
}

fn cluster_labels(cluster: &SpikeCluster) -> Vec<&'static str> {
    let mut labels = Vec::new();

    if cluster
        .points
        .iter()
        .any(|point| point.comm == "RenderThread" || point.comm == "Main")
    {
        labels.push("render-main");
    }

    if cluster
        .points
        .iter()
        .any(|point| point.comm.starts_with("dxvk-"))
    {
        labels.push("dxvk");
    }

    if cluster
        .points
        .iter()
        .any(|point| point.comm == "wineserver" || point.comm.contains("winedevice"))
    {
        labels.push("wine");
    }

    if cluster
        .points
        .iter()
        .any(|point| point.comm == "AudioThread")
    {
        labels.push("audio");
    }

    labels
}

fn percentile_warning_note(percentile_scope: &str) -> &'static str {
    match percentile_scope {
        "histogram" => {
            "p95/p99 are approximate histogram estimates across the full session; max and threshold counters are exact"
        }
        "capped_prefix" | "capped" => {
            "p95/p99 are capped prefix estimates; prefer max and over_1ms/over_2ms/over_5ms"
        }
        _ => {
            "p95/p99 may be capped because this session predates histogram percentiles; prefer max and threshold counters"
        }
    }
}

// load_all_frame_events removed

fn calculate_median_frametime(frames: &[FrameEvent]) -> f64 {
    if frames.is_empty() {
        return 0.0;
    }
    let mut times: Vec<_> = frames.iter().map(|f| f.frametime_ms).collect();
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = times.len() / 2;
    if times.len() % 2 == 0 {
        (times[mid - 1] + times[mid]) / 2.0
    } else {
        times[mid]
    }
}

fn identify_frame_spikes(frames: &[FrameEvent], median: f64) -> Vec<FrameEvent> {
    let threshold = if median.is_finite() && median > 0.0 {
        (1.5 * median).min(33.3)
    } else {
        33.3
    };

    frames
        .iter()
        .filter(|f| f.frametime_ms.is_finite() && f.frametime_ms > threshold)
        .cloned()
        .collect()
}

fn perform_frame_diagnosis(
    session: &SessionFile,
    frame_spikes: &[FrameEvent],
    all_spike_points: &[SpikePoint],
    artifacts: &session_io::RunArtifacts,
    cluster_window_ns: u64,
) -> Vec<FrameDiagnosis> {
    let mut diagnoses = Vec::new();
    for frame in frame_spikes {
        let frame_monotonic_ns = if let Some(start_ns) = session.core.monotonic_start_ns {
            start_ns + (frame.elapsed_ms * 1_000_000)
        } else {
            0
        };

        let nearby_points: Vec<_> = all_spike_points
            .iter()
            .filter(|p| p.switch_ns.abs_diff(frame_monotonic_ns) <= cluster_window_ns)
            .cloned()
            .collect();

        let distinct_tasks = nearby_points
            .iter()
            .map(|p| p.task)
            .collect::<BTreeSet<_>>()
            .len();
        let cluster = cluster_from_points(nearby_points, distinct_tasks);

        // Let `diagnose_cluster` handle artifact filtering.

        let diagnosis = diagnose_cluster(&cluster, artifacts, cluster_window_ns);

        diagnoses.push(FrameDiagnosis {
            frame_elapsed_ms: frame.elapsed_ms,
            frametime_ms: frame.frametime_ms,
            diagnosis,
        });
    }

    diagnoses
}

fn render_frame_diagnosis(rank: usize, diag: &FrameDiagnosis) -> String {
    let mut output = String::new();
    pushln(
        &mut output,
        format!(
            "{}. elapsed={}ms frametime={:.1}ms diagnosis: {}",
            rank,
            diag.frame_elapsed_ms,
            diag.frametime_ms,
            diag.diagnosis.report_summary()
        ),
    );
    output.push_str(&render_diagnosis_detail_lines(&diag.diagnosis, "  "));
    output.trim_end().to_owned()
}

fn compute_correlation_windows(
    session: &SessionFile,
    clusters: &[SpikeCluster],
    frame_spikes: &[FrameEvent],
    cluster_window_ns: u64,
) -> session_io::CorrelationWindows {
    let mut windows = session_io::CorrelationWindows::default();
    let padding_ms = (cluster_window_ns / 1_000_000).max(1);

    for cluster in clusters {
        let min_ns = cluster.min_switch_ns.saturating_sub(cluster_window_ns);
        let max_ns = cluster.max_switch_ns.saturating_add(cluster_window_ns);
        windows.windows_ns.push((min_ns, max_ns));

        if let Some((min_e, max_e)) = cluster_elapsed_range(cluster) {
            // Padding for SCX (2000ms), CPU freq (50ms), GPU (50ms), intervals (1000ms)
            windows.windows_ms.push((
                min_e.saturating_sub(2000).max(padding_ms),
                max_e.saturating_add(2000).max(padding_ms),
            ));
        }
    }

    for frame in frame_spikes {
        if let Some(start_ns) = session.core.monotonic_start_ns {
            let frame_ns = start_ns + (frame.elapsed_ms * 1_000_000);
            windows.windows_ns.push((
                frame_ns.saturating_sub(cluster_window_ns),
                frame_ns.saturating_add(cluster_window_ns),
            ));
        }
        windows.windows_ms.push((
            frame.elapsed_ms.saturating_sub(2000).max(padding_ms),
            frame.elapsed_ms.saturating_add(2000).max(padding_ms),
        ));
    }

    windows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn foreground_event(
        elapsed_ms: u64,
        pid: Option<u32>,
        app_id: Option<&str>,
        class: Option<&str>,
        title: Option<&str>,
        workspace: Option<&str>,
        confidence: f32,
    ) -> ForegroundEvent {
        ForegroundEvent {
            elapsed_ms,
            source: crate::foreground::ForegroundSource::Sway,
            status: crate::foreground::ForegroundProviderStatus::Available,
            pid,
            app_id: app_id.map(str::to_owned),
            class: class.map(str::to_owned),
            title: title.map(str::to_owned),
            window_id: Some("7".to_owned()),
            workspace: workspace.map(str::to_owned),
            confidence,
            reason: "test foreground event".to_owned(),
        }
    }

    fn cluster_at(elapsed_ms: u64) -> SpikeCluster {
        SpikeCluster {
            points: vec![SpikePoint {
                elapsed_ms: Some(elapsed_ms),
                ..SpikePoint::default()
            }],
            distinct_tasks: 1,
            min_switch_ns: 0,
            max_switch_ns: 0,
            max_latency_ns: 0,
            diagnosis: None,
            anchor_task: None,
            anchor_class: None,
            anchor_comm: None,
            anchor_kind: None,
            foreground_pid: None,
            foreground_app_id: None,
            foreground_class: None,
            foreground_confidence: None,
            wake_graph: Vec::new(),
        }
    }

    #[test]
    fn report_includes_foreground_summary_when_events_present() {
        let mut session = minimal_session_for_report_test();
        session.config.foreground_window = true;
        session.config.foreground_source = "sway".to_owned();
        session.core.foreground_event_count = 1;

        let summary = foreground_report_summary(
            &session,
            &[foreground_event(
                1_000,
                Some(4242),
                Some("steam_app_379430"),
                Some("steam_app_379430"),
                None,
                Some("gaming"),
                0.95,
            )],
        );

        assert!(summary.enabled);
        assert_eq!(summary.source.as_deref(), Some("sway"));
        assert_eq!(summary.final_pid, Some(4242));
        assert_eq!(summary.final_app_id.as_deref(), Some("steam_app_379430"));
        assert_eq!(summary.final_class.as_deref(), Some("steam_app_379430"));
        assert_eq!(summary.final_workspace.as_deref(), Some("gaming"));
        assert_eq!(summary.event_count, 1);
        assert_eq!(summary.confidence, Some(0.95));
    }

    #[test]
    fn report_redacts_missing_title_cleanly() {
        let summary = ForegroundReportSummary {
            enabled: true,
            source: Some("sway".to_owned()),
            final_pid: Some(4242),
            final_app_id: Some("steam_app_379430".to_owned()),
            final_class: Some("steam_app_379430".to_owned()),
            final_title: None,
            final_workspace: Some("gaming".to_owned()),
            event_count: 1,
            confidence: Some(0.95),
            provider_status: Some("available".to_owned()),
            reasons: Vec::new(),
        };

        let text = render_foreground_summary_text(&summary);

        assert!(text.contains("Foreground window:"));
        assert!(text.contains("title: redacted (pass --foreground-include-title to record it)"));
        assert!(!text.contains("Private"));
    }

    #[test]
    fn spike_cluster_gets_nearest_foreground_context() {
        let mut clusters = vec![cluster_at(1_500)];
        let events = vec![
            foreground_event(
                1_000,
                Some(1111),
                Some("steamwebhelper"),
                Some("steamwebhelper"),
                None,
                None,
                0.60,
            ),
            foreground_event(
                1_400,
                Some(4242),
                Some("steam_app_379430"),
                Some("steam_app_379430"),
                None,
                Some("gaming"),
                0.95,
            ),
            foreground_event(
                1_600,
                Some(9999),
                Some("future"),
                Some("future"),
                None,
                None,
                0.95,
            ),
        ];

        annotate_clusters_with_foreground(&mut clusters, &events, 1_000);

        assert_eq!(clusters[0].foreground_pid, Some(4242));
        assert_eq!(
            clusters[0].foreground_app_id.as_deref(),
            Some("steam_app_379430")
        );
        assert_eq!(
            clusters[0].foreground_class.as_deref(),
            Some("steam_app_379430")
        );
        assert_eq!(clusters[0].foreground_confidence, Some(0.95));
    }

    #[test]
    fn foreground_report_summary_uses_final_event_and_redacted_title() {
        let mut session = minimal_session_for_report_test();
        session.config.foreground_window = true;
        session.config.foreground_source = "sway".to_owned();
        session.core.foreground_event_count = 2;
        session.core.foreground_source = Some("sway".to_owned());
        session.core.final_foreground_pid = Some(12345);
        session.core.final_foreground_app_id = Some("steam_app_379430".to_owned());
        session.core.final_foreground_class = Some("steam_app_379430".to_owned());

        let events = vec![
            foreground_event(
                100,
                Some(1000),
                Some("steam"),
                Some("Steam"),
                None,
                Some("gaming"),
                0.90,
            ),
            foreground_event(
                200,
                Some(12345),
                Some("steam_app_379430"),
                Some("steam_app_379430"),
                None,
                Some("gaming"),
                0.95,
            ),
        ];

        let summary = foreground_report_summary(&session, &events);

        assert!(summary.enabled);
        assert_eq!(summary.source.as_deref(), Some("sway"));
        assert_eq!(summary.final_pid, Some(12345));
        assert_eq!(summary.final_app_id.as_deref(), Some("steam_app_379430"));
        assert_eq!(summary.final_class.as_deref(), Some("steam_app_379430"));
        assert_eq!(summary.final_title, None);
        assert_eq!(summary.final_workspace.as_deref(), Some("gaming"));
        assert_eq!(summary.event_count, 2);
        assert_eq!(summary.confidence, Some(0.95));
        assert_eq!(summary.provider_status.as_deref(), Some("available"));
    }

    #[test]
    fn render_foreground_summary_text_mentions_redacted_title() {
        let summary = ForegroundReportSummary {
            enabled: true,
            source: Some("sway".to_owned()),
            final_pid: Some(12345),
            final_app_id: Some("steam_app_379430".to_owned()),
            final_class: Some("steam_app_379430".to_owned()),
            final_title: None,
            final_workspace: Some("gaming".to_owned()),
            event_count: 7,
            confidence: Some(0.95),
            provider_status: Some("available".to_owned()),
            reasons: vec!["focused Sway node from swaymsg get_tree".to_owned()],
        };

        let text = render_foreground_summary_text(&summary);

        assert!(text.contains("Foreground window:"));
        assert!(text.contains("  source: sway"));
        assert!(text.contains("  final pid: 12345"));
        assert!(text.contains("  app_id/class: steam_app_379430"));
        assert!(text.contains("  workspace: gaming"));
        assert!(text.contains("  confidence: 0.95"));
        assert!(text.contains("  events: 7"));
        assert!(text.contains("  title: redacted (pass --foreground-include-title to record it)"));
    }

    #[test]
    fn foreground_for_cluster_uses_nearest_event_at_or_before_cluster_time() {
        let cluster = cluster_at(1_500);
        let events = vec![
            foreground_event(500, Some(1), Some("old"), Some("Old"), None, None, 0.50),
            foreground_event(
                1_200,
                Some(2),
                Some("game"),
                Some("Game"),
                None,
                Some("gaming"),
                0.95,
            ),
            foreground_event(
                1_600,
                Some(3),
                Some("future"),
                Some("Future"),
                None,
                None,
                0.95,
            ),
        ];

        let selected = foreground_for_cluster(&cluster, &events, 1_000).unwrap();

        assert_eq!(selected.pid, Some(2));
        assert_eq!(selected.app_id.as_deref(), Some("game"));
    }

    #[test]
    fn foreground_for_cluster_respects_max_stale_ms() {
        let cluster = cluster_at(2_000);
        let events = vec![foreground_event(
            500,
            Some(1),
            Some("old"),
            Some("Old"),
            None,
            None,
            0.50,
        )];

        assert!(foreground_for_cluster(&cluster, &events, 1_000).is_none());
    }

    #[test]
    fn annotate_clusters_with_foreground_sets_cluster_fields() {
        let mut clusters = vec![cluster_at(1_500)];
        let events = vec![foreground_event(
            1_200,
            Some(12345),
            Some("steam_app_379430"),
            Some("steam_app_379430"),
            None,
            Some("gaming"),
            0.95,
        )];

        annotate_clusters_with_foreground(&mut clusters, &events, 1_000);

        assert_eq!(clusters[0].foreground_pid, Some(12345));
        assert_eq!(
            clusters[0].foreground_app_id.as_deref(),
            Some("steam_app_379430")
        );
        assert_eq!(
            clusters[0].foreground_class.as_deref(),
            Some("steam_app_379430")
        );
        assert_eq!(clusters[0].foreground_confidence, Some(0.95));
    }

    #[test]
    fn report_analysis_json_contains_foreground_summary() {
        let mut session = minimal_session_for_report_test();
        session.config.foreground_window = true;
        session.config.foreground_source = "sway".to_owned();
        session.core.foreground_event_count = 1;
        let summary = foreground_report_summary(
            &session,
            &[foreground_event(
                100,
                Some(12345),
                Some("steam_app_379430"),
                Some("steam_app_379430"),
                None,
                Some("gaming"),
                0.95,
            )],
        );

        let json = serde_json::to_string(&summary).unwrap();

        assert!(json.contains("\"enabled\":true"));
        assert!(json.contains("\"source\":\"sway\""));
        assert!(json.contains("\"final_pid\":12345"));
        assert!(json.contains("\"event_count\":1"));
    }
    use crate::{
        autotune::state::SituationKind,
        recorder::{FocusEvent, RecordedConfig, SessionMetadataCore},
    };

    #[test]
    fn focus_report_summary_prefers_latest_changed_focus_event() {
        let session = SessionFile {
            core: SessionMetadataCore {
                focus_mode: Some("auto-focus".to_owned()),
                final_focus_kind: Some("Browser".to_owned()),
                focus_switch_count: 2,
                ..Default::default()
            },
            config: RecordedConfig {
                auto_focus: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let events = vec![
            FocusEvent {
                elapsed_ms: 100,
                action: "changed".to_owned(),
                kind: Some("Browser".to_owned()),
                confidence: 0.62,
                situation: Some(SituationKind::BrowserFocused),
                root_pids: vec![111],
                member_pids: vec![111, 112],
                reasons: vec!["browser parent with active renderer".to_owned()],
                ..Default::default()
            },
            FocusEvent {
                elapsed_ms: 200,
                action: "changed".to_owned(),
                kind: Some("Compile".to_owned()),
                confidence: 0.87,
                score: 0.91,
                situation: Some(SituationKind::CompileLoad),
                root_pids: vec![1234],
                member_pids: vec![1234, 1235],
                reasons: vec![
                    "cargo root with 14 active compiler descendants".to_owned(),
                    "linker/write IO evidence observed".to_owned(),
                ],
                ..Default::default()
            },
        ];

        let summary = focus_report_summary(&session, &events);

        assert_eq!(summary.mode.as_deref(), Some("auto-focus"));
        assert_eq!(summary.final_focus.as_deref(), Some("Compile"));
        assert_eq!(summary.situation.as_deref(), Some("CompileLoad"));
        assert_eq!(summary.confidence, Some(0.87));
        assert_eq!(summary.score, Some(0.91));
        assert_eq!(summary.roots, vec![1234]);
        assert_eq!(summary.member_pids, vec![1234, 1235]);
        assert_eq!(summary.focus_switches, 2);
        assert_eq!(summary.reasons.len(), 2);
    }

    #[test]
    fn render_focus_summary_text_includes_visible_reasons() {
        let summary = FocusReportSummary {
            mode: Some("auto-focus".to_owned()),
            final_focus: Some("Compile".to_owned()),
            display_name: Some("cargo build".to_owned()),
            situation: Some("CompileLoad".to_owned()),
            confidence: Some(0.87),
            score: Some(0.91),
            roots: vec![1234],
            member_pids: vec![1234, 1235],
            focus_switches: 2,
            reasons: vec![
                "cargo root with 14 active compiler descendants".to_owned(),
                "CPU delta 780% over 1s".to_owned(),
            ],
        };

        let text = render_focus_summary_text(&summary);

        assert!(text.contains("Auto focus:"));
        assert!(text.contains("  mode: auto-focus"));
        assert!(text.contains("  final focus: Compile"));
        assert!(text.contains("  situation: CompileLoad"));
        assert!(text.contains("  confidence: 0.87"));
        assert!(text.contains("  roots: [1234]"));
        assert!(text.contains("  focus switches: 2"));
        assert!(text.contains("    - cargo root with 14 active compiler descendants"));
        assert!(text.contains("    - CPU delta 780% over 1s"));
    }

    #[test]
    fn event_stream_warning_is_absent_without_errors() {
        assert!(event_stream_warning(0, None).is_none());
    }

    #[test]
    fn event_stream_warning_includes_count_and_first_error() {
        let warning =
            event_stream_warning(2, Some("spike_events: No space left on device")).unwrap();

        assert!(warning.contains("2 write error"));
        assert!(warning.contains("event artifact files may be incomplete"));
        assert!(warning.contains("spike_events: No space left on device"));
    }

    #[test]
    fn event_stream_warning_handles_missing_first_error() {
        let warning = event_stream_warning(1, None).unwrap();

        assert!(warning.contains("1 write error"));
        assert!(warning.contains("first error was not recorded"));
    }

    #[test]
    fn classify_switch_prev_state_zero_is_preempted_or_runnable() {
        assert_eq!(classify_switch_prev_state(0), "preempted_or_runnable");
    }

    #[test]
    fn classify_switch_prev_state_interruptible() {
        assert_eq!(
            classify_switch_prev_state(1),
            "voluntary_sleep_interruptible"
        );
    }

    #[test]
    fn classify_switch_prev_state_uninterruptible() {
        assert_eq!(
            classify_switch_prev_state(2),
            "voluntary_sleep_uninterruptible"
        );
    }

    #[test]
    fn classify_switch_prev_state_other_sleep() {
        assert_eq!(classify_switch_prev_state(8), "other_sleep");
    }

    #[test]
    fn classify_switch_prev_state_interruptible_wins_when_multiple_bits_set() {
        assert_eq!(
            classify_switch_prev_state(3),
            "voluntary_sleep_interruptible"
        );
    }

    #[test]
    fn test_build_wake_graph_grouping_and_sorting() {
        let points = vec![
            SpikePoint {
                task: 101,
                comm: "wakee1".to_owned(),
                waker_tid: 201,
                waker_comm: "waker1".to_owned(),
                latency_ns: 1000,
                ..Default::default()
            },
            SpikePoint {
                task: 101,
                comm: "wakee1".to_owned(),
                waker_tid: 201,
                waker_comm: "waker1".to_owned(),
                latency_ns: 2000,
                ..Default::default()
            },
            SpikePoint {
                task: 102,
                comm: "wakee2".to_owned(),
                waker_tid: 201,
                waker_comm: "waker1".to_owned(),
                latency_ns: 500,
                ..Default::default()
            },
            SpikePoint {
                task: 101,
                comm: "wakee1".to_owned(),
                waker_tid: 202,
                waker_comm: "waker2".to_owned(),
                latency_ns: 5000,
                ..Default::default()
            },
        ];

        let graph = build_wake_graph(&points);

        // Should have 3 edges:
        // 1. (201, waker1) -> (101, wakee1) count=2 max_lat=2000
        // 2. (202, waker2) -> (101, wakee1) count=1 max_lat=5000
        // 3. (201, waker1) -> (102, wakee2) count=1 max_lat=500

        // Sorted by count desc, then max_lat desc
        assert_eq!(graph.len(), 3);

        assert_eq!(graph[0].waker_tid, 201);
        assert_eq!(graph[0].wakee_tid, 101);
        assert_eq!(graph[0].count, 2);
        assert_eq!(graph[0].max_latency_ns, 2000);

        assert_eq!(graph[1].waker_tid, 202);
        assert_eq!(graph[1].count, 1);
        assert_eq!(graph[1].max_latency_ns, 5000);

        assert_eq!(graph[2].waker_tid, 201);
        assert_eq!(graph[2].wakee_tid, 102);
        assert_eq!(graph[2].count, 1);
        assert_eq!(graph[2].max_latency_ns, 500);
    }

    fn minimal_session_for_report_test() -> SessionFile {
        SessionFile {
            core: crate::recorder::SessionMetadataCore {
                schema_version: SESSION_SCHEMA_VERSION,
                duration_ms: 1000,
                interval_record_count: 1,
                active_target_pids_count: 1,
                block_io_correlation_basis: "request-pointer".to_owned(),
                ..Default::default()
            },
            stop_reason: "test".to_owned(),
            ..Default::default()
        }
    }

    fn spike_point_for_report_test(
        task: u32,
        class: TaskClass,
        comm: &str,
        latency_ns: u64,
    ) -> SpikePoint {
        SpikePoint {
            task,
            class,
            process_pid: Some(task),
            comm: comm.to_owned(),
            latency_ns,
            wakeup_ns: 10_000_000,
            switch_ns: 10_000_000 + latency_ns,
            elapsed_ms: Some(100),
            ..Default::default()
        }
    }

    fn pressure_interval(
        elapsed_ms: u64,
        cpu_some: f64,
        mem_some: f64,
        mem_full: f64,
        io_some: f64,
        io_full: f64,
    ) -> IntervalRecord {
        IntervalRecord {
            elapsed_ms,
            task: 42,
            active: true,
            class: TaskClass::Unknown,
            comm: "worker".to_owned(),
            process_pid: Some(42),
            process_comm: "worker".into(),
            cpu_psi_some: cpu_some,
            mem_psi_some: mem_some,
            mem_psi_full: mem_full,
            io_psi_some: io_some,
            io_psi_full: io_full,
            percentile_scope: "exact".to_owned(),
            ..Default::default()
        }
    }

    fn pressure_cluster() -> SpikeCluster {
        cluster_from_points(
            vec![
                SpikePoint {
                    elapsed_ms: Some(100),
                    ..spike_point_for_report_test(1, TaskClass::Unknown, "worker-a", 2_000_000)
                },
                SpikePoint {
                    elapsed_ms: Some(110),
                    ..spike_point_for_report_test(2, TaskClass::Unknown, "worker-b", 2_000_000)
                },
            ],
            2,
        )
    }

    #[test]
    fn data_quality_is_high_for_clean_minimal_session() {
        let session = minimal_session_for_report_test();
        let validation = crate::session_io::RunValidationReport::default();

        let summary = data_quality_summary(&session, &validation);

        assert_eq!(summary.level, DataQualityLevel::High);
        assert!(
            summary
                .reasons
                .iter()
                .any(|reason| reason.contains("no data-quality problems"))
        );
    }

    #[test]
    fn data_quality_is_low_for_validation_errors() {
        let session = minimal_session_for_report_test();
        let validation = crate::session_io::RunValidationReport {
            errors: vec!["bad session".to_owned()],
            ..Default::default()
        };

        let summary = data_quality_summary(&session, &validation);

        assert_eq!(summary.level, DataQualityLevel::Low);
        assert!(
            summary
                .reasons
                .iter()
                .any(|reason| reason.contains("validation errors"))
        );
    }

    #[test]
    fn data_quality_warns_on_truncated_spikes() {
        let mut session = minimal_session_for_report_test();
        session.core.spike_events_truncated = true;
        session.core.spike_events_retained_count = 500_000;
        session.core.spike_events_dropped_count = 1;

        let validation = crate::session_io::RunValidationReport::default();

        let summary = data_quality_summary(&session, &validation);

        assert_eq!(summary.level, DataQualityLevel::Medium);
        assert!(
            summary
                .reasons
                .iter()
                .any(|reason| reason.contains("spike event stream was truncated"))
        );
    }

    #[test]
    fn data_quality_warns_on_missing_optional_artifacts() {
        let session = minimal_session_for_report_test();
        let validation = crate::session_io::RunValidationReport {
            missing_optional_files: vec!["frame_correlation.json".to_owned()],
            ..Default::default()
        };

        let summary = data_quality_summary(&session, &validation);

        assert_eq!(summary.level, DataQualityLevel::Medium);
        assert!(
            summary
                .reasons
                .iter()
                .any(|reason| reason.contains("optional correlation artifacts"))
        );
    }

    #[test]
    fn data_quality_warns_on_cpu_perf_errors() {
        let mut session = minimal_session_for_report_test();
        session.config.cpu_perf = true;
        session.core.cpu_perf_open_errors = 1;
        session.core.cpu_perf_skipped_tasks = 2;
        let validation = crate::session_io::RunValidationReport::default();

        let summary = data_quality_summary(&session, &validation);

        assert_eq!(summary.level, DataQualityLevel::Medium);
        assert!(summary.cpu_perf_requested);
        assert_eq!(summary.cpu_perf_open_errors, 1);
        assert!(
            summary
                .reasons
                .iter()
                .any(|reason| reason.contains("CPU perf counters had open/read errors"))
        );
        assert!(
            summary
                .reasons
                .iter()
                .any(|reason| reason.contains("CPU perf skipped 2 active tasks"))
        );
    }

    #[test]
    fn render_report_includes_data_quality_section() {
        let session = minimal_session_for_report_test();
        let output = render_report(
            Path::new("session.json"),
            &session,
            &SpikeClusterAnalysis {
                source: SpikeClusterSource::TopSpikesFallback,
                source_count: 0,
                clusters: vec![],
            },
            &[],
            &session_io::RunArtifacts::default(),
            &FocusReportSummary::default(),
            &ForegroundReportSummary::default(),
            10,
            500,
            None,
        );

        assert!(output.contains("data quality"));
        assert!(output.contains("level: High"));
    }

    #[test]
    fn pressure_timeline_empty_without_intervals() {
        let summary = build_pressure_timeline(&[], &[pressure_cluster()], 5);

        assert_eq!(summary.sample_count, 0);
        assert_eq!(summary.max_cpu_some, 0.0);
        assert_eq!(summary.max_mem_some, None);
        assert!(summary.windows.is_empty());
    }

    #[test]
    fn pressure_timeline_marks_near_spike() {
        let intervals = vec![
            pressure_interval(96, 10.0, 0.0, 0.0, 0.0, 0.0),
            pressure_interval(120, 20.0, 0.0, 0.0, 0.0, 0.0),
        ];

        let summary = build_pressure_timeline(&intervals, &[pressure_cluster()], 5);

        assert!(summary.windows[0].near_spike);
        assert!(!summary.windows[1].near_spike);
    }

    #[test]
    fn pressure_timeline_sorts_windows() {
        let intervals = vec![
            pressure_interval(300, 1.0, 0.0, 0.0, 0.0, 0.0),
            pressure_interval(100, 2.0, 0.0, 0.0, 0.0, 0.0),
        ];

        let summary = build_pressure_timeline(&intervals, &[], 5);

        assert_eq!(
            summary
                .windows
                .iter()
                .map(|window| window.elapsed_ms)
                .collect::<Vec<_>>(),
            vec![100, 300]
        );
    }

    #[test]
    fn pressure_timeline_max_cpu_some() {
        let intervals = vec![
            pressure_interval(100, 1.0, 0.0, 0.0, 0.0, 0.0),
            pressure_interval(200, 42.0, 0.0, 0.0, 0.0, 0.0),
            pressure_interval(300, 3.0, 0.0, 0.0, 0.0, 0.0),
        ];

        let summary = build_pressure_timeline(&intervals, &[], 5);

        assert_eq!(summary.max_cpu_some, 42.0);
    }

    #[test]
    fn pressure_timeline_includes_memory_io_fields() {
        let intervals = vec![pressure_interval(100, 1.0, 2.0, 3.0, 4.0, 5.0)];

        let summary = build_pressure_timeline(&intervals, &[], 5);
        let window = &summary.windows[0];

        assert_eq!(summary.max_mem_some, Some(2.0));
        assert_eq!(summary.max_mem_full, Some(3.0));
        assert_eq!(summary.max_io_some, Some(4.0));
        assert_eq!(summary.max_io_full, Some(5.0));
        assert_eq!(window.mem_some, Some(2.0));
        assert_eq!(window.mem_full, Some(3.0));
        assert_eq!(window.io_some, Some(4.0));
        assert_eq!(window.io_full, Some(5.0));
    }

    #[test]
    fn render_report_includes_pressure_timeline_when_pressure_present() {
        let session = minimal_session_for_report_test();
        let cluster_analysis = SpikeClusterAnalysis {
            source: SpikeClusterSource::TopSpikesFallback,
            source_count: 2,
            clusters: vec![pressure_cluster()],
        };
        let artifacts = session_io::RunArtifacts {
            intervals: vec![pressure_interval(100, 40.0, 2.0, 0.0, 0.0, 0.0)],
            ..Default::default()
        };

        let output = render_report(
            Path::new("session.json"),
            &session,
            &cluster_analysis,
            &[],
            &artifacts,
            &FocusReportSummary::default(),
            &ForegroundReportSummary::default(),
            10,
            5,
            None,
        );

        assert!(output.contains("pressure timeline"));
        assert!(output.contains("samples=1"));
        assert!(output.contains("windows_near_spikes=1"));
        assert!(output.contains("max_cpu_some=40.00"));
    }

    #[test]
    fn analysis_json_contains_pressure_timeline() {
        let session = minimal_session_for_report_test();
        let validation = crate::session_io::RunValidationReport::default();
        let analysis = ReportAnalysisJson {
            session: session.clone(),
            cluster_analysis: SpikeClusterAnalysis {
                source: SpikeClusterSource::TopSpikesFallback,
                source_count: 0,
                clusters: vec![],
            },
            frame_diagnoses: vec![],
            pressure_timeline: build_pressure_timeline(
                &[pressure_interval(100, 10.0, 0.0, 0.0, 0.0, 0.0)],
                &[],
                5,
            ),
            artifacts_summary: artifacts_summary_from_session(&session),
            data_quality: data_quality_summary(&session, &validation),
            focus_summary: FocusReportSummary::default(),
            foreground_summary: ForegroundReportSummary::default(),
        };

        let value = serde_json::to_value(&analysis).unwrap();

        assert!(value.get("pressure_timeline").is_some());
        assert_eq!(value["pressure_timeline"]["sample_count"].as_u64(), Some(1));
    }

    fn test_html_report_model() -> HtmlReportModel {
        let mut session = minimal_session_for_report_test();
        session.tasks.push(SessionTask {
            task: 42,
            active: true,
            class: TaskClass::Game,
            process_pid: Some(42),
            process_comm: "test-game".into(),
            comm: "test-game".to_owned(),
            latency: crate::recorder::RecordedLatency {
                samples: 100,
                stored_samples: 100,
                percentile_scope: "exact".to_owned(),
                avg_ns: 750_000,
                p99_ns: 2_000_000,
                max_ns: 5_000_000,
                over_1ms: 7,
                over_2ms: 3,
                over_5ms: 1,
                ..Default::default()
            },
            ..Default::default()
        });

        let validation = crate::session_io::RunValidationReport::default();
        let analysis = ReportAnalysisJson {
            session: session.clone(),
            cluster_analysis: SpikeClusterAnalysis {
                source: SpikeClusterSource::TopSpikesFallback,
                source_count: 0,
                clusters: vec![],
            },
            frame_diagnoses: vec![],
            pressure_timeline: PressureTimelineSummary::default(),
            artifacts_summary: artifacts_summary_from_session(&session),
            data_quality: data_quality_summary(&session, &validation),
            focus_summary: FocusReportSummary::default(),
            foreground_summary: ForegroundReportSummary::default(),
        };

        build_html_report_model(
            &session,
            &session_io::RunArtifacts::default(),
            &analysis,
            10,
            None,
            Some("stutter report\n==============".to_owned()),
        )
        .unwrap()
    }

    #[test]
    fn render_html_report_uses_structured_sections() {
        let model = test_html_report_model();

        let html = render_html_report(&model).unwrap();

        assert!(html.contains(r#"id="summary-section""#));
        assert!(html.contains(r#"id="data-quality-section""#));
        assert!(html.contains(r#"id="top-tasks-section""#));
        assert!(html.contains(r#"id="spike-charts-section""#));
        assert!(html.contains(r#"id="cluster-analysis-section""#));
        assert!(html.contains(r#"id="frame-diagnoses-section""#));
        assert!(html.contains(r#"id="artifacts-section""#));
        assert!(html.contains(r#"id="data-report-model""#));
        assert!(html.contains("test-game"));
        assert!(html.contains("<summary>Legacy text report</summary>"));
        assert!(!html.contains("<pre>stutter report"));
    }

    #[test]
    fn render_cluster_uses_cautious_diagnosis_wording() {
        let mut cluster = cluster_from_points(
            vec![spike_point_for_report_test(
                456,
                TaskClass::Game,
                "RenderThread",
                8_000_000,
            )],
            1,
        );
        cluster.diagnosis = Some(diagnose_cluster(
            &cluster,
            &session_io::RunArtifacts::default(),
            0,
        ));

        let output = render_cluster(1, &cluster);

        assert!(output.contains("diagnosis: GameThreadSchedulerDelay: strong candidate"));
        assert!(output.contains("profiler inference"));
        assert!(output.contains("diagnosis_candidate cause=GameThreadSchedulerDelay"));
        assert!(output.contains("evidence kind=SchedulerDelay"));
        assert!(!output.contains("diagnosis: primary="));
    }

    #[test]
    fn test_identify_frame_spikes() {
        let frames = vec![
            FrameEvent {
                elapsed_ms: 0,
                frametime_ms: 16.0,
            },
            FrameEvent {
                elapsed_ms: 0,
                frametime_ms: 24.1,
            },
            FrameEvent {
                elapsed_ms: 0,
                frametime_ms: 30.0,
            },
            FrameEvent {
                elapsed_ms: 0,
                frametime_ms: f64::NAN,
            },
        ];

        // median 16.0 => threshold 24.0 (1.5 * 16 = 24.0, which is < 33.3)
        let spikes = identify_frame_spikes(&frames, 16.0);
        assert_eq!(spikes.len(), 2);
        assert_eq!(spikes[0].frametime_ms, 24.1);
        assert_eq!(spikes[1].frametime_ms, 30.0);

        // median 30.0 => threshold 33.3 (1.5 * 30 = 45.0, but capped at 33.3)
        let spikes = identify_frame_spikes(&frames, 30.0);
        assert!(spikes.is_empty());

        // median 0.0 => threshold 33.3
        let spikes = identify_frame_spikes(&frames, 0.0);
        assert!(spikes.is_empty());

        let frames_with_long = vec![FrameEvent {
            elapsed_ms: 0,
            frametime_ms: 33.4,
        }];
        let spikes = identify_frame_spikes(&frames_with_long, 0.0);
        assert_eq!(spikes.len(), 1);
        assert_eq!(spikes[0].frametime_ms, 33.4);
    }

    #[test]
    fn build_spike_density_counts_and_max_latency_by_bucket() {
        let spikes = vec![
            SpikeEvent {
                elapsed_ms: Some(0),
                latency_ns: 1_000_000,
                ..Default::default()
            }, // 1 ms latency, bucket 0
            SpikeEvent {
                elapsed_ms: Some(10),
                latency_ns: 5_000_000,
                ..Default::default()
            }, // 5 ms latency, bucket 0
            SpikeEvent {
                elapsed_ms: Some(99),
                latency_ns: 2_000_000,
                ..Default::default()
            }, // 2 ms latency, bucket 0
            SpikeEvent {
                elapsed_ms: Some(100),
                latency_ns: 7_000_000,
                ..Default::default()
            }, // 7 ms latency, bucket 1
        ];

        let buckets = build_spike_density(&spikes, 100);

        assert_eq!(buckets.len(), 2);

        assert_eq!(buckets[0].start_ms, 0);
        assert_eq!(buckets[0].end_ms, 100);
        assert_eq!(buckets[0].count, 3);
        assert_eq!(buckets[0].max_latency_ms, 5.0);
        // p99 of [1, 5, 2] -> sorted [1, 2, 5]. len=3. rank = (3-1)*0.99 = 1.98 -> round to 2. values[2] = 5.
        assert_eq!(buckets[0].p99_latency_ms, 5.0);

        assert_eq!(buckets[1].start_ms, 100);
        assert_eq!(buckets[1].end_ms, 200);
        assert_eq!(buckets[1].count, 1);
        assert_eq!(buckets[1].max_latency_ms, 7.0);
        assert_eq!(buckets[1].p99_latency_ms, 7.0);
    }
}
