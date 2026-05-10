use std::{
    borrow::Cow,
    collections::BTreeSet,
    env, fs, io,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use stutter_common::SchedulerEvent;

use crate::{
    artifacts::{ArtifactKind, ArtifactStreamRegistry},
    cli::{Config, RecordingConfig, TARGET_PIDS_MAX},
    ebpf_loader::DropCountersSnapshot,
    metadata::{SystemMetadata, collect_system_metadata},
    metrics::{
        CpuLine, CpuPerfRecord, CpuSnapshot, IntervalRecord as MetricsIntervalRecord,
        LatencyHistogramBucket, RuntimeSliceRecord as MetricsRuntimeSliceRecord, SpikeRecord,
        TaskStats,
    },
    process_tree::TaskClass,
    prometheus::PrometheusState,
};

pub type IntervalRecord = MetricsIntervalRecord;
pub type RuntimeSliceRecord = MetricsRuntimeSliceRecord;
pub use crate::{foreground::ForegroundEvent, scx::ScxEvent};
pub const MAX_SPIKE_EVENTS: usize = 500_000;

#[derive(Default, Debug)]
pub struct LiveBuffers {
    pub interval_records: Vec<IntervalRecord>,
    pub tree_events: Vec<TreeEvent>,
    pub spike_events: Option<SpikeEventBuffer>,
    pub irq_events: Vec<IrqEventRecord>,
    pub gpu_samples: Vec<GpuSample>,
    pub scx_events: Vec<crate::scx::ScxEvent>,
}

#[derive(Default, Debug)]
pub struct RecordingCounters {
    pub intervals_dropped: u64,
    pub scx_event_count: u64,
    pub irq_event_count: u64,
    pub migration_event_count: u64,
    pub cpu_freq_sample_count: u64,
    pub gpu_sample_count: u64,
    pub block_io_event_count: u64,
    pub runtime_slice_count: u64,
    pub runtime_slice_read_errors: u64,
    pub runtime_slice_skipped_tasks: u64,
    pub interval_record_count: u64,
    pub frame_event_count: u64,
    pub focus_event_count: u64,
    pub foreground_event_count: u64,
    pub process_scan_budget_exceeded_count: u64,
    pub thread_scan_limited_count: u64,

    #[allow(dead_code)]
    pub frame_events_dropped: u64,

    pub spike_event_count: u64,
    pub spike_events_dropped_count: u64,
    pub alert_events_dropped_count: u64,
    pub alert_channel_closed_count: u64,

    pub event_stream_write_errors: u64,
    pub first_event_stream_write_error: Option<String>,

    pub stdout_spike_stream_errors: u64,
}

#[derive(Default, Debug)]
pub struct ExporterState {
    pub prometheus_state: Option<Arc<PrometheusState>>,
    pub otel_spike_tx: Option<tokio::sync::mpsc::Sender<crate::otel::OtelSpike>>,
    pub otel_spans_dropped: Option<Arc<std::sync::atomic::AtomicU64>>,
}

#[derive(Default)]
pub struct LiveRecorder {
    pub run: Option<RecordingRun>,
    pub buffers: LiveBuffers,
    pub streams: ArtifactStreamRegistry,
    pub csv_writer: Option<IntervalCsvWriter>,
    pub stdout_spike_stream: Option<StdoutJsonStream>,
    pub counters: RecordingCounters,
    pub exporters: ExporterState,
    pub last_foreground_event: Option<ForegroundEvent>,
}

impl std::fmt::Debug for LiveRecorder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveRecorder")
            .field("run", &self.run)
            .field("buffers", &self.buffers)
            .field("counters", &self.counters)
            .field("exporters", &self.exporters)
            .field("last_foreground_event", &self.last_foreground_event)
            .finish_non_exhaustive()
    }
}

impl RecordingCounters {
    pub fn record_stream_write_error<E: std::fmt::Display>(&mut self, stream_name: &str, err: E) {
        self.event_stream_write_errors += 1;
        if self.first_event_stream_write_error.is_none() {
            self.first_event_stream_write_error = Some(format!("{stream_name}: {err}"));
        }
    }
}

impl LiveRecorder {
    pub fn push_spike_event_to_buffer(&mut self, spike_event: SpikeEvent) {
        if let Some(spike_events) = self.buffers.spike_events.as_mut() {
            match spike_events.push(spike_event) {
                SpikePushResult::Stored => {}
                SpikePushResult::Dropped => {
                    self.counters.spike_events_dropped_count += 1;
                }
            }
        }
    }

    pub fn enable_stdout_spike_stream(&mut self) {
        self.stdout_spike_stream = Some(StdoutJsonStream::new());
    }

    #[allow(dead_code)]
    pub fn write_foreground_event(&mut self, event: ForegroundEvent) -> anyhow::Result<()> {
        self.last_foreground_event = Some(event.clone());

        if self.streams.contains(ArtifactKind::ForegroundEvents) {
            self.streams.push(ArtifactKind::ForegroundEvents, &event)?;
            self.counters.foreground_event_count =
                self.counters.foreground_event_count.saturating_add(1);
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct RecordingRun {
    pub run_name: Option<String>,
    pub run_dir: PathBuf,
    pub started_at: SystemTime,
    pub started_instant: Instant,
    pub monotonic_start_ns: Option<u64>,
    pub mangohud_start_offset: Option<u64>,
    pub mangohud_first_frame_monotonic_ns: Option<u64>,
    pub mangohud_first_frame_raw_elapsed_ms: Option<u64>,
}

#[derive(Debug)]
pub struct NdjsonWriter {
    file: fs::File,
    wrote_any: bool,
    finished: bool,
    path: PathBuf,
}

pub enum CsvOutput {
    File(io::BufWriter<fs::File>),
    Stdout(io::BufWriter<io::Stdout>),
}

pub struct IntervalCsvWriter {
    output: CsvOutput,
    path_label: String,
    finished: bool,
}

impl std::fmt::Debug for IntervalCsvWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IntervalCsvWriter")
            .field("path", &self.path_label)
            .field("finished", &self.finished)
            .finish()
    }
}

impl IntervalCsvWriter {
    pub fn create_file(path: PathBuf) -> anyhow::Result<Self> {
        if path.file_name().is_none() {
            anyhow::bail!("CSV destination has no file name: {}", path.display());
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = fs::File::create(&path)
            .with_context(|| format!("failed to create interval CSV {}", path.display()))?;
        write_interval_csv_header(&mut file)?;
        Ok(Self {
            output: CsvOutput::File(io::BufWriter::new(file)),
            path_label: path.display().to_string(),
            finished: false,
        })
    }

    pub fn stdout() -> Self {
        let mut stdout = io::stdout();
        let _ = write_interval_csv_header(&mut stdout);
        Self {
            output: CsvOutput::Stdout(io::BufWriter::new(stdout)),
            path_label: "stdout".to_owned(),
            finished: false,
        }
    }

    pub fn push(&mut self, record: &IntervalRecord) -> anyhow::Result<()> {
        match &mut self.output {
            CsvOutput::File(writer) => write_interval_csv_row(writer, record),
            CsvOutput::Stdout(writer) => write_interval_csv_row(writer, record),
        }
        .with_context(|| format!("failed to write interval CSV {}", self.path_label))
    }

    pub fn finish(&mut self) -> anyhow::Result<()> {
        if self.finished {
            return Ok(());
        }
        match &mut self.output {
            CsvOutput::File(writer) => {
                writer.flush()?;
                writer.get_ref().sync_all()?;
            }
            CsvOutput::Stdout(writer) => {
                writer.flush()?;
            }
        }
        self.finished = true;
        Ok(())
    }
}

impl Drop for IntervalCsvWriter {
    fn drop(&mut self) {
        if let Err(err) = self.finish() {
            log::warn!(
                "interval_csv_finish_failed path={} err={err:#}",
                self.path_label
            );
        }
    }
}

impl NdjsonWriter {
    pub fn create(path: PathBuf) -> anyhow::Result<Self> {
        if path.file_name().is_none() {
            anyhow::bail!(
                "NDJSON stream destination has no file name: {}",
                path.display()
            );
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file = fs::File::create(&path)
            .with_context(|| format!("failed to create NDJSON stream {}", path.display()))?;

        Ok(Self {
            file,
            wrote_any: false,
            finished: false,
            path,
        })
    }

    pub fn push<T: Serialize>(&mut self, value: &T) -> anyhow::Result<()> {
        if self.finished {
            anyhow::bail!("NDJSON stream {} is already finalized", self.path.display());
        }

        serde_json::to_writer(&mut self.file, value)
            .with_context(|| format!("failed to write NDJSON stream {}", self.path.display()))?;
        self.file.write_all(b"\n")?;
        self.wrote_any = true;
        Ok(())
    }
    pub fn finish(&mut self) -> anyhow::Result<()> {
        if self.finished {
            return Ok(());
        }

        self.file
            .sync_all()
            .with_context(|| format!("failed to sync NDJSON stream {}", self.path.display()))?;
        self.finished = true;
        Ok(())
    }
}

pub struct StdoutJsonStream {
    stdout: std::io::Stdout,
}

impl StdoutJsonStream {
    pub fn new() -> Self {
        Self {
            stdout: std::io::stdout(),
        }
    }

    pub fn push<T: serde::Serialize>(&mut self, value: &T) -> anyhow::Result<()> {
        write_ndjson_value(&mut self.stdout, value)
    }
}

impl Default for StdoutJsonStream {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for StdoutJsonStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StdoutJsonStream").finish()
    }
}

pub fn write_ndjson_value<W, T>(writer: &mut W, value: &T) -> anyhow::Result<()>
where
    W: std::io::Write,
    T: serde::Serialize,
{
    serde_json::to_writer(&mut *writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

impl Drop for NdjsonWriter {
    fn drop(&mut self) {
        if let Err(err) = self.finish() {
            log::warn!(
                "ndjson_finish_failed path={} err={err:#}",
                self.path.display()
            );
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum SpikePushResult {
    Stored,
    Dropped,
}

#[derive(Debug)]
pub struct SpikeEventBuffer {
    events: Vec<SpikeEvent>,
    truncated: bool,
    max_events: u64,
}

impl SpikeEventBuffer {
    pub fn new(max_events: u64) -> Self {
        Self {
            events: Vec::with_capacity(1024.min(max_events as usize)),
            truncated: false,
            max_events,
        }
    }

    pub fn push(&mut self, event: SpikeEvent) -> SpikePushResult {
        if (self.events.len() as u64) < self.max_events {
            self.events.push(event);
            SpikePushResult::Stored
        } else {
            self.truncated = true;
            SpikePushResult::Dropped
        }
    }
    #[cfg(test)]
    pub fn truncate(&mut self) {
        self.truncated = true;
    }

    #[cfg(test)]
    pub fn with_max_events(max_events: u64) -> Self {
        Self {
            events: Vec::new(),
            truncated: false,
            max_events,
        }
    }
    #[cfg(test)]
    pub fn as_slice(&self) -> &[SpikeEvent] {
        &self.events
    }

    #[cfg(test)]
    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

impl Default for SpikeEventBuffer {
    fn default() -> Self {
        Self::new(MAX_SPIKE_EVENTS as u64)
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct FocusEvent {
    pub elapsed_ms: u64,
    pub action: String,
    pub old_kind: Option<String>,
    pub kind: Option<String>,
    pub root_pids: Vec<u32>,
    pub member_pids: Vec<u32>,
    pub confidence: f32,
    pub score: f32,
    pub situation: Option<crate::autotune::state::SituationKind>,
    pub reasons: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct TreeEvent {
    pub elapsed_ms: u64,
    pub action: String,
    pub tid: u32,
    pub process_pid: u32,
    pub process_ppid: u32,
    pub comm: String,
    pub process_comm: std::sync::Arc<str>,
    pub class: TaskClass,
    pub from_cgroup: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SessionMetadataCore {
    pub schema_version: u32,
    pub run_name: Option<String>,
    pub started_at: RecordedTime,
    pub ended_at: RecordedTime,
    pub monotonic_start_ns: Option<u64>,
    pub monotonic_end_ns: Option<u64>,
    pub duration_ms: u64,
    #[serde(default)]
    pub mangohud_start_offset: Option<u64>,
    #[serde(default)]
    pub mangohud_first_frame_monotonic_ns: Option<u64>,
    #[serde(default)]
    pub mangohud_first_frame_raw_elapsed_ms: Option<u64>,
    pub metadata: SystemMetadata,
    pub target_pids_max: u64,
    pub active_target_pids_count: u64,
    pub active_expanded_tasks: Vec<u32>,
    #[serde(default)]
    pub focus_mode: Option<String>,
    #[serde(default)]
    pub final_focus_kind: Option<String>,
    #[serde(default)]
    pub focus_switch_count: u64,
    #[serde(default)]
    pub focus_event_count: u64,
    #[serde(default)]
    pub foreground_event_count: u64,
    #[serde(default)]
    pub foreground_source: Option<String>,
    #[serde(default)]
    pub final_foreground_pid: Option<u32>,
    #[serde(default)]
    pub final_foreground_app_id: Option<String>,
    #[serde(default)]
    pub final_foreground_class: Option<String>,
    #[serde(default)]
    pub interval_record_count: u64,
    #[serde(default)]
    pub intervals_dropped: u64,
    #[serde(default)]
    pub spike_events_retained_count: u64,
    #[serde(default)]
    pub spike_events_dropped_count: u64,
    #[serde(default)]
    pub spike_events_truncated: bool,
    #[serde(default)]
    pub scx_event_count: u64,
    #[serde(default)]
    pub irq_event_count: u64,
    #[serde(default)]
    pub migration_event_count: Option<u64>,
    #[serde(default)]
    pub cpu_freq_sample_count: Option<u64>,
    #[serde(default)]
    pub gpu_sample_count: u64,
    #[serde(default)]
    pub frame_event_count: u64,
    #[serde(default)]
    pub block_io_event_count: u64,
    #[serde(default)]
    pub runtime_slice_count: u64,
    #[serde(default)]
    pub runtime_slice_read_errors: u64,
    #[serde(default)]
    pub runtime_slice_skipped_tasks: u64,
    #[serde(default)]
    pub runtime_slice_source: Option<String>,
    #[serde(default)]
    pub event_stream_write_errors: u64,
    #[serde(default)]
    pub alert_events_dropped_count: u64,
    #[serde(default)]
    pub alert_channel_closed_count: u64,
    #[serde(default)]
    pub first_event_stream_write_error: Option<String>,
    #[serde(default = "default_block_io_correlation_basis_string")]
    pub block_io_correlation_basis: String,
    #[serde(default)]
    pub drop_counters: DropCountersSnapshot,
    #[serde(default)]
    pub cpu_perf_sample_count: u64,
    #[serde(default)]
    pub cpu_perf_open_errors: u64,
    #[serde(default)]
    pub cpu_perf_read_errors: u64,
    #[serde(default)]
    pub cpu_perf_skipped_tasks: u64,
    #[serde(default)]
    pub cpu_perf_last_error: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SessionFile {
    #[serde(flatten)]
    pub core: SessionMetadataCore,
    pub stop_reason: String,
    pub config: RecordedConfig,
    pub tasks: Vec<SessionTask>,
    pub top_spikes: Vec<SessionSpike>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct MetadataFile {
    #[serde(flatten)]
    pub core: SessionMetadataCore,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct RecordedConfig {
    pub manual_pids: Vec<u32>,
    pub tree_roots: Vec<u32>,
    #[serde(default)]
    pub cgroupv2: Option<PathBuf>,
    #[serde(default)]
    pub exclude_tree_pids: Vec<u32>,
    #[serde(default)]
    pub include_comm: Vec<String>,
    #[serde(default)]
    pub exclude_comm: Vec<String>,
    #[serde(default)]
    pub watch_process: Option<String>,
    #[serde(default)]
    pub persistent: bool,
    #[serde(default)]
    pub keep_missing_pid: bool,
    #[serde(default)]
    pub watch_poll_ms: u64,
    #[serde(default)]
    pub watch_timeout_ms: Option<u64>,
    #[serde(default)]
    pub csv_stream: Option<crate::cli::CsvStreamTarget>,
    #[serde(default)]
    pub irq_latency: bool,
    #[serde(default)]
    pub irqs: Vec<u32>,
    #[serde(default)]
    pub hwmon: bool,
    #[serde(default)]
    pub hwmon_root: Option<PathBuf>,
    #[serde(default)]
    pub hwmon_drm_card: Option<String>,
    #[serde(default)]
    pub hwmon_render_node: Option<PathBuf>,
    #[serde(default)]
    pub mangohud_log: Option<PathBuf>,
    #[serde(default)]
    pub mangohud_log_live: bool,
    #[serde(default)]
    pub tui: bool,
    pub summary_period_ms: u64,
    #[serde(default)]
    pub epoch_period_ms: Option<u64>,
    #[serde(default)]
    pub retain_intervals: Option<usize>,
    #[serde(default = "default_recorded_max_tasks")]
    pub max_tasks: usize,
    pub spike_threshold_ns: u64,
    #[serde(default)]
    pub alert_threshold_ns: Option<u64>,
    #[serde(default)]
    pub alert_webhook_url: Option<String>,
    #[serde(default = "default_recorded_follow_exec")]
    pub follow_exec: bool,
    pub verbose: bool,
    #[serde(default)]
    pub faults: bool,
    #[serde(default)]
    pub cpu_perf: bool,
    #[serde(default)]
    pub cpu_perf_kernel: bool,
    #[serde(default = "default_recorded_cpu_perf_max_tasks")]
    pub cpu_perf_max_tasks: usize,
    #[serde(default)]
    pub cpu_perf_cache_refs: bool,
    #[serde(default)]
    pub block_io: bool,
    #[serde(default)]
    pub stat_wait: bool,
    #[serde(default)]
    pub runtime_slices: bool,
    #[serde(default)]
    pub runtime_slices_max_tasks: usize,
    #[serde(default)]
    pub otlp_endpoint: Option<String>,
    #[serde(default)]
    pub otel_service_name: String,
    #[serde(default)]
    pub auto_focus: bool,
    #[serde(default)]
    pub foreground_window: bool,
    #[serde(default)]
    pub focus_source: String,
    #[serde(default)]
    pub foreground_source: String,
    #[serde(default)]
    pub foreground_poll_ms: u64,
    #[serde(default)]
    pub foreground_max_stale_ms: u64,
    #[serde(default)]
    pub foreground_include_title: bool,
    #[serde(default)]
    pub auto_focus_poll_ms: u64,
    #[serde(default)]
    pub auto_focus_min_confidence: f32,
    #[serde(default)]
    pub auto_focus_switch_cooldown_ms: u64,
    #[serde(default)]
    pub auto_focus_switch_margin: f32,
    #[serde(default)]
    pub auto_focus_required_polls: u32,
    #[serde(default)]
    pub auto_focus_max_roots: usize,
}

fn default_recorded_max_tasks() -> usize {
    TARGET_PIDS_MAX
}

fn default_recorded_follow_exec() -> bool {
    true
}

fn focus_source_label(source: crate::cli::FocusSource) -> String {
    match source {
        crate::cli::FocusSource::Heuristic => "heuristic",
        crate::cli::FocusSource::Foreground => "foreground",
        crate::cli::FocusSource::Hybrid => "hybrid",
    }
    .to_owned()
}

fn foreground_source_arg_label(source: crate::cli::ForegroundSourceArg) -> String {
    match source {
        crate::cli::ForegroundSourceArg::Auto => "auto",
        crate::cli::ForegroundSourceArg::Sway => "sway",
        crate::cli::ForegroundSourceArg::Hyprland => "hyprland",
        crate::cli::ForegroundSourceArg::X11 => "x11",
    }
    .to_owned()
}

fn foreground_source_label(source: crate::foreground::ForegroundSource) -> String {
    match source {
        crate::foreground::ForegroundSource::Auto => "auto",
        crate::foreground::ForegroundSource::Sway => "sway",
        crate::foreground::ForegroundSource::Hyprland => "hyprland",
        crate::foreground::ForegroundSource::X11 => "x11",
        crate::foreground::ForegroundSource::Unsupported => "unsupported",
    }
    .to_owned()
}

fn default_recorded_cpu_perf_max_tasks() -> usize {
    128
}

#[cfg(test)]
mod focus_recording_tests {
    use super::*;

    #[test]
    fn foreground_event_serializes_without_title_by_default() {
        let event = ForegroundEvent::new(
            1_000,
            crate::foreground::ForegroundSource::Sway,
            crate::foreground::ForegroundProviderStatus::Available,
            Some(4242),
            Some("steam_app_379430".to_owned()),
            Some("steam_app_379430".to_owned()),
            Some("Private game or browser title".to_owned()),
            false,
            Some("7".to_owned()),
            Some("gaming".to_owned()),
            0.95,
            "focused Sway node from swaymsg get_tree",
        );

        let value = serde_json::to_value(&event).unwrap();

        assert_eq!(
            value.get("elapsed_ms").and_then(serde_json::Value::as_u64),
            Some(1_000)
        );
        assert_eq!(
            value.get("source").and_then(serde_json::Value::as_str),
            Some("sway")
        );
        assert_eq!(
            value.get("status").and_then(serde_json::Value::as_str),
            Some("available")
        );
        assert_eq!(
            value.get("pid").and_then(serde_json::Value::as_u64),
            Some(4242)
        );
        assert_eq!(
            value.get("app_id").and_then(serde_json::Value::as_str),
            Some("steam_app_379430")
        );
        assert_eq!(
            value.get("class").and_then(serde_json::Value::as_str),
            Some("steam_app_379430")
        );
        assert!(value.get("title").unwrap().is_null());
        assert_eq!(
            value.get("workspace").and_then(serde_json::Value::as_str),
            Some("gaming")
        );
    }

    #[test]
    fn foreground_event_serializes_expected_fields() {
        let event = ForegroundEvent {
            elapsed_ms: 1234,
            source: crate::foreground::ForegroundSource::Sway,
            status: crate::foreground::ForegroundProviderStatus::Available,
            pid: Some(4242),
            app_id: Some("steam".to_owned()),
            class: Some("Steam".to_owned()),
            title: None,
            window_id: Some("123".to_owned()),
            workspace: Some("games".to_owned()),
            confidence: 0.95,
            reason: "focused Sway node from swaymsg get_tree".to_owned(),
        };

        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains("\"elapsed_ms\":1234"));
        assert!(json.contains("\"source\":\"sway\""));
        assert!(json.contains("\"status\":\"available\""));
        assert!(json.contains("\"pid\":4242"));
        assert!(json.contains("\"app_id\":\"steam\""));
        assert!(json.contains("\"class\":\"Steam\""));
        assert!(json.contains("\"title\":null"));
        assert!(json.contains("\"window_id\":\"123\""));
        assert!(json.contains("\"workspace\":\"games\""));
        assert!(json.contains("\"confidence\":0.95"));
    }

    #[test]
    fn live_recorder_writes_foreground_event_to_dedicated_stream() {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-foreground-event-stream-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("foreground_events.json");

        let mut recorder = LiveRecorder::default();
        recorder
            .streams
            .create_stream(&dir, ArtifactKind::ForegroundEvents)
            .unwrap();

        let event = ForegroundEvent {
            elapsed_ms: 42,
            source: crate::foreground::ForegroundSource::X11,
            status: crate::foreground::ForegroundProviderStatus::Available,
            pid: Some(1000),
            app_id: Some("Navigator".to_owned()),
            class: Some("Firefox".to_owned()),
            title: None,
            window_id: Some("0x1200007".to_owned()),
            workspace: None,
            confidence: 0.90,
            reason: "active X11 window from xprop".to_owned(),
        };

        recorder.write_foreground_event(event.clone()).unwrap();
        recorder.streams.finish_all().unwrap();

        assert_eq!(recorder.counters.foreground_event_count, 1);
        assert_eq!(
            recorder.last_foreground_event.as_ref().unwrap().pid,
            Some(1000)
        );

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"source\":\"x11\""));
        assert!(text.contains("\"pid\":1000"));
        assert!(!text.contains("focus"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn focus_event_serializes_expected_fields() {
        let event = FocusEvent {
            elapsed_ms: 1234,
            action: "changed".to_owned(),
            old_kind: Some("Browser".to_owned()),
            kind: Some("Game".to_owned()),
            root_pids: vec![10],
            member_pids: vec![10, 11],
            confidence: 0.75,
            score: 0.82,
            situation: Some(crate::autotune::state::SituationKind::GameFocused),
            reasons: vec!["focus resolver selected game".to_owned()],
        };

        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains("\"elapsed_ms\":1234"));
        assert!(json.contains("\"action\":\"changed\""));
        assert!(json.contains("\"old_kind\":\"Browser\""));
        assert!(json.contains("\"kind\":\"Game\""));
        assert!(json.contains("\"root_pids\":[10]"));
        assert!(json.contains("\"member_pids\":[10,11]"));
        assert!(json.contains("\"confidence\":0.75"));
        assert!(json.contains("\"score\":0.82"));
        assert!(json.contains("\"situation\":\"GameFocused\""));
    }

    #[test]
    fn recorded_config_defaults_foreground_fields_for_old_sessions() {
        let config = RecordedConfig::default();

        assert!(!config.foreground_window);
        assert_eq!(config.foreground_source, "");
        assert_eq!(config.foreground_poll_ms, 0);
        assert_eq!(config.foreground_max_stale_ms, 0);
        assert!(!config.foreground_include_title);
    }

    #[test]
    fn recorded_config_defaults_auto_focus_fields_for_old_sessions() {
        let config = RecordedConfig::default();

        assert!(!config.auto_focus);
        assert!(!config.foreground_window);
        assert_eq!(config.focus_source, "");
        assert_eq!(config.foreground_source, "");
        assert_eq!(config.foreground_poll_ms, 0);
        assert_eq!(config.foreground_max_stale_ms, 0);
        assert!(!config.foreground_include_title);
        assert_eq!(config.auto_focus_poll_ms, 0);
        assert_eq!(config.auto_focus_min_confidence, 0.0);
        assert_eq!(config.auto_focus_switch_cooldown_ms, 0);
        assert_eq!(config.auto_focus_switch_margin, 0.0);
        assert_eq!(config.auto_focus_required_polls, 0);
        assert_eq!(config.auto_focus_max_roots, 0);
    }

    #[test]
    fn session_metadata_defaults_focus_fields_for_old_sessions() {
        let core = SessionMetadataCore::default();

        assert_eq!(core.focus_mode, None);
        assert_eq!(core.final_focus_kind, None);
        assert_eq!(core.focus_switch_count, 0);
        assert_eq!(core.focus_event_count, 0);
        assert_eq!(core.foreground_event_count, 0);
        assert_eq!(core.foreground_source, None);
        assert_eq!(core.final_foreground_pid, None);
        assert_eq!(core.final_foreground_app_id, None);
        assert_eq!(core.final_foreground_class, None);
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct RecordedTime {
    pub unix_seconds: u64,
    pub unix_nanos: u32,
    #[serde(alias = "local")]
    pub system_time_debug: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct SessionTask {
    pub task: u32,
    pub active: bool,
    pub first_seen_ms: u64,
    pub last_seen_ms: u64,
    pub removed_ms: Option<u64>,
    pub class: TaskClass,
    pub process_pid: Option<u32>,
    pub process_comm: std::sync::Arc<str>,
    #[serde(default)]
    pub process_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub task_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub exe_dev: Option<u64>,
    #[serde(default)]
    pub exe_ino: Option<u64>,
    pub comm: String,
    pub latency: RecordedLatency,
    pub cpu: RecordedCpuSnapshot,
    pub top_spikes: Vec<RecordedSpike>,
    #[serde(default)]
    pub migration_count: u64,
    #[serde(default)]
    pub cross_numa_migrations: u64,
    #[serde(default)]
    pub top_wakers: Vec<WakerEntry>,
    #[serde(default)]
    pub sched_policy: Option<String>,
    #[serde(default)]
    pub stat_wait_sum_ns: Option<u64>,
    #[serde(default)]
    pub stat_wait_sum_ns_saturated: bool,
    #[serde(default)]
    pub stat_wait_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_perf: Option<CpuPerfRecord>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct WakerEntry {
    pub waker_tid: u32,
    pub waker_comm: String,
    pub count: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct RecordedLatency {
    pub samples: u64,
    pub stored_samples: u64,
    pub truncated_samples: u64,
    pub percentile_scope: String,
    #[serde(default)]
    pub histogram: Vec<LatencyHistogramBucket>,
    pub min_ns: u64,
    pub avg_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct RecordedCpuSnapshot {
    pub busiest_cpu: Option<u32>,
    pub busiest_cpu_samples: u64,
    pub worst_cpu: Option<u32>,
    pub worst_cpu_max_ns: u64,
    pub spikiest_cpu: Option<u32>,
    pub spikiest_cpu_spikes: u64,
    pub per_cpu: Vec<CpuLine>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct RecordedSpike {
    pub class: TaskClass,
    pub process_pid: Option<u32>,
    pub process_comm: std::sync::Arc<str>,
    pub cpu: u32,
    #[serde(default)]
    pub wakeup_target_cpu: u32,
    pub prio: i32,
    pub latency_ns: u64,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
    #[serde(default)]
    pub switch_prev_pid: u32,
    #[serde(default)]
    pub switch_prev_state: i64,
    #[serde(default)]
    pub switch_prev_state_label: String,
    // Diagnostic-only count of monitored pending wakeups for this target/task.
    // This is not CPU runqueue depth and must not be used as true CPU contention.
    #[serde(alias = "target_runnable_depth")]
    pub target_pending_wakeups: u32,
    /// Approximate per-CPU runnable depth reconstructed from sched wakeup/switch
    /// tracepoints. This is not literal rq->nr_running.
    #[serde(default)]
    pub observed_runnable_depth: u32,
    #[serde(default)]
    pub major_faults: u64,
    #[serde(default)]
    pub minor_faults: u64,
    #[serde(default)]
    pub scx_ops: Option<String>,
    #[serde(default)]
    pub scx_state: Option<String>,
    #[serde(default)]
    pub scx_enable_seq: Option<String>,

    #[serde(default)]
    pub waker_tid: u32,
    #[serde(default)]
    pub waker_comm: String,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cause_tags: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_cause: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct SessionSpike {
    pub task: u32,
    pub active: bool,
    pub class: TaskClass,
    pub process_pid: Option<u32>,
    pub process_comm: std::sync::Arc<str>,
    pub comm: String,
    pub cpu: u32,
    #[serde(default)]
    pub wakeup_target_cpu: u32,
    pub prio: i32,
    pub latency_ns: u64,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
    #[serde(default)]
    pub switch_prev_pid: u32,
    #[serde(default)]
    pub switch_prev_state: i64,
    #[serde(default)]
    pub switch_prev_state_label: String,
    // Diagnostic-only count of monitored pending wakeups for this target/task.
    // This is not CPU runqueue depth and must not be used as true CPU contention.
    #[serde(alias = "target_runnable_depth")]
    pub target_pending_wakeups: u32,
    /// Approximate per-CPU runnable depth reconstructed from sched wakeup/switch
    /// tracepoints. This is not literal rq->nr_running.
    #[serde(default)]
    pub observed_runnable_depth: u32,
    #[serde(default)]
    pub major_faults: u64,
    #[serde(default)]
    pub minor_faults: u64,
    #[serde(default)]
    pub scx_ops: Option<String>,
    #[serde(default)]
    pub scx_state: Option<String>,
    #[serde(default)]
    pub scx_enable_seq: Option<String>,

    #[serde(default)]
    pub waker_tid: u32,
    #[serde(default)]
    pub waker_comm: String,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cause_tags: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_cause: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct SpikeEvent {
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
    pub task: u32,
    pub active: bool,
    pub class: TaskClass,
    pub process_pid: Option<u32>,
    pub process_comm: std::sync::Arc<str>,
    pub comm: String,
    pub cpu: u32,
    #[serde(default)]
    pub wakeup_target_cpu: u32,
    pub prio: i32,
    pub latency_ns: u64,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
    #[serde(default)]
    pub switch_prev_pid: u32,
    #[serde(default)]
    pub switch_prev_state: i64,
    #[serde(default)]
    pub switch_prev_state_label: String,
    // Diagnostic-only count of monitored pending wakeups for this target/task.
    // This is not CPU runqueue depth and must not be used as true CPU contention.
    #[serde(alias = "target_runnable_depth")]
    pub target_pending_wakeups: u32,
    /// Approximate per-CPU runnable depth reconstructed from sched wakeup/switch
    /// tracepoints. This is not literal rq->nr_running.
    #[serde(default)]
    pub observed_runnable_depth: u32,
    #[serde(default)]
    pub major_faults: u64,
    #[serde(default)]
    pub minor_faults: u64,
    #[serde(default)]
    pub scx_ops: Option<String>,
    #[serde(default)]
    pub scx_state: Option<String>,
    #[serde(default)]
    pub scx_enable_seq: Option<String>,

    #[serde(default)]
    pub waker_tid: u32,
    #[serde(default)]
    pub waker_comm: String,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cause_tags: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_cause: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct MigrationEventRecord {
    pub elapsed_ms: u64,
    pub tid: u32,
    pub from_cpu: u32,
    pub to_cpu: u32,
    pub timestamp_ns: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct CpuFreqRecord {
    pub elapsed_ms: u64,
    pub cpu: u32,
    pub freq_khz: u32,
    pub timestamp_ns: u64,
}

pub struct SpikeDiagnosticContext {
    pub scx_ops: Option<String>,
    pub scx_state: Option<String>,
    pub scx_enable_seq: Option<String>,
    pub cause_tags: Vec<String>,
    pub primary_cause: Option<String>,
    pub waker_tid: u32,
    pub waker_comm: String,
}

impl SpikeEvent {
    pub fn from_task_stats(
        monotonic_start_ns: Option<u64>,
        stats: &TaskStats,
        event: &SchedulerEvent,
        fault_deltas: (u64, u64),
        diag: SpikeDiagnosticContext,
    ) -> Self {
        Self {
            elapsed_ms: elapsed_ms_from_monotonic(monotonic_start_ns, event.switch_ns),
            task: event.tid,
            active: stats.active,
            class: stats.class,
            process_pid: stats.process_pid,
            process_comm: stats.process_comm.clone(),
            comm: stats.comm.clone(),
            cpu: event.cpu,
            wakeup_target_cpu: event.wakeup_target_cpu,
            prio: event.prio,
            latency_ns: event.latency_ns,
            wakeup_ns: event.wakeup_ns,
            switch_ns: event.switch_ns,
            switch_prev_pid: event.switch_prev_pid,
            switch_prev_state: event.switch_prev_state,
            switch_prev_state_label: crate::sched_state::classify_switch_prev_state(
                event.switch_prev_state,
            )
            .to_owned(),
            waker_tid: diag.waker_tid,
            waker_comm: diag.waker_comm,
            target_pending_wakeups: event.target_pending_wakeups,
            observed_runnable_depth: event.observed_runnable_depth,
            major_faults: fault_deltas.0,
            minor_faults: fault_deltas.1,
            scx_ops: diag.scx_ops,
            scx_state: diag.scx_state,
            scx_enable_seq: diag.scx_enable_seq,
            cause_tags: diag.cause_tags,
            primary_cause: diag.primary_cause,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct IrqEventRecord {
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
    pub irq: u32,
    pub cpu: u32,
    pub enter_ns: u64,
    pub exit_ns: u64,
    pub duration_ns: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct GpuSample {
    pub elapsed_ms: u64,
    pub gpu_busy_percent: Option<u32>,
    pub vram_used_bytes: Option<u64>,
    pub vram_total_bytes: Option<u64>,
    pub vram_used_percent: Option<u32>,
    pub gpu_clock_mhz: Option<u32>,
    pub mem_clock_mhz: Option<u32>,
    pub temp_millidegrees: Option<u32>,
    pub power_microwatts: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct FrameEvent {
    pub elapsed_ms: u64,
    pub frametime_ms: f64,
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct BlockIoRecord {
    pub elapsed_ms: u64,
    pub tid: u32,
    #[serde(default = "default_block_io_correlation_basis")]
    pub correlation_basis: Cow<'static, str>,
    pub dev: u32,
    pub nr_sector: u32,
    pub sector: u64,
    pub duration_ns: u64,
    pub timestamp_ns: u64,
    pub rwbs: String,
}

fn default_block_io_correlation_basis() -> Cow<'static, str> {
    Cow::Borrowed("dev+sector")
}

fn default_block_io_correlation_basis_string() -> String {
    default_block_io_correlation_basis().into_owned()
}

pub const SESSION_SCHEMA_VERSION: u32 = 21;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CpuPerfStatus {
    pub sample_count: u64,
    pub active_counter_tasks: u64,
    pub skipped_counter_tasks: u64,
    pub open_errors: u64,
    pub read_errors: u64,
    pub last_error: Option<String>,
}

pub struct FinalizeRecordingInput<'a> {
    pub recorder: &'a LiveRecorder,
    pub config: &'a Config,
    pub tree_pids: &'a [u32],
    pub stop_reason: &'a str,
    pub tasks: &'a crate::tasks::TaskTracker,
    pub frame_events: &'a [FrameEvent],
    pub block_io_correlation_basis: &'a str,
    pub drop_counters: crate::ebpf_loader::DropCountersSnapshot,
    pub cpu_perf_status: Option<CpuPerfStatus>,
    pub focus_mode: Option<String>,
    pub final_focus_kind: Option<String>,
    pub focus_switch_count: u64,
    #[allow(dead_code)]
    pub current_focus: Option<crate::focus::ResolvedFocus>,
    pub final_foreground_event: Option<ForegroundEvent>,
}

pub fn prepare_recording(config: &Config) -> anyhow::Result<Option<RecordingRun>> {
    let Some(recording) = &config.recording else {
        return Ok(None);
    };

    let started_at = SystemTime::now();
    let run_dir = resolve_run_dir(recording, started_at, env::var_os("HOME"));
    if let Err(err) = ensure_empty_dir(&run_dir) {
        return Err(err.context("record write failed"));
    }

    Ok(Some(RecordingRun {
        run_name: recording.run_name.clone(),
        run_dir,
        started_at,
        started_instant: Instant::now(),
        monotonic_start_ns: monotonic_now_ns(),
        mangohud_start_offset: None,
        mangohud_first_frame_monotonic_ns: None,
        mangohud_first_frame_raw_elapsed_ms: None,
    }))
}

pub fn recording_warnings(recorder: &LiveRecorder) -> Vec<String> {
    let mut warnings = Vec::new();

    if recorder.counters.intervals_dropped > 0 {
        warnings.push(format!(
            "warning: {} interval record(s) were dropped due to --retain-intervals; reports may not include full interval history",
            recorder.counters.intervals_dropped
        ));
    }

    if recorder.counters.spike_events_dropped_count > 0 {
        warnings.push(format!(
            "warning: {} spike event record(s) were dropped because the in-memory spike buffer was full; reports may not include every spike",
            recorder.counters.spike_events_dropped_count
        ));
    }

    if recorder.counters.event_stream_write_errors > 0 {
        let first_err_suffix = if let Some(first_error) =
            recorder.counters.first_event_stream_write_error.as_deref()
        {
            format!("; first error: {}", first_error)
        } else {
            "".to_owned()
        };
        warnings.push(format!(
            "warning: {} event stream write error(s) occurred while recording{}; one or more NDJSON artifact files may be incomplete",
            recorder.counters.event_stream_write_errors, first_err_suffix
        ));
    }

    if recorder.counters.process_scan_budget_exceeded_count > 0 {
        warnings.push(format!(
            "warning: process tree scan budget exceeded {} times; reports may be incomplete due to skipping task discovery",
            recorder.counters.process_scan_budget_exceeded_count
        ));
    }

    if recorder.counters.thread_scan_limited_count > 0 {
        warnings.push(format!(
            "warning: thread scan limit exceeded {} times; reports may be incomplete due to skipping thread discovery within massive processes",
            recorder.counters.thread_scan_limited_count
        ));
    }

    warnings
}

pub fn print_recording_warnings(recorder: &LiveRecorder) {
    for warning in recording_warnings(recorder) {
        eprintln!("{warning}");
    }
}

pub fn finalize_recording(input: FinalizeRecordingInput<'_>) -> anyhow::Result<()> {
    let FinalizeRecordingInput {
        recorder,
        config,
        tree_pids,
        stop_reason,
        tasks: task_tracker,
        frame_events,
        block_io_correlation_basis,
        drop_counters,
        cpu_perf_status,
        focus_mode,
        final_focus_kind,
        focus_switch_count,
        current_focus: _,
        final_foreground_event,
    } = input;

    let Some(recording) = recorder.run.as_ref() else {
        return Ok(());
    };

    let active_targets = &task_tracker.active_targets;
    let stats_by_task = &task_tracker.stats_by_task;
    let interval_records = &recorder.buffers.interval_records;
    let interval_record_count = recorder.counters.interval_record_count;
    let tree_events = &recorder.buffers.tree_events;
    let spike_events = recorder
        .buffers
        .spike_events
        .as_ref()
        .map(|s| s.events.as_slice())
        .unwrap_or(&[]);

    let irq_event_count = recorder.counters.irq_event_count;
    let gpu_sample_count = recorder.counters.gpu_sample_count;
    let ended_at = SystemTime::now();
    let monotonic_end_ns = monotonic_now_ns();
    let duration_ms = recording.started_instant.elapsed().as_millis() as u64;
    let metadata = collect_system_metadata();

    let mut active_expanded_tasks = active_targets.keys().copied().collect::<Vec<_>>();
    active_expanded_tasks.sort_unstable();

    let mut tasks = Vec::new();
    let mut top_spikes = Vec::new();

    for (task, stats) in stats_by_task {
        let mut session_latency = stats.session_latency.clone();
        let Some(latency) = session_latency.snapshot() else {
            continue;
        };

        let cpu = stats.session_cpu.snapshot();

        let (stat_wait_sum_ns, stat_wait_sum_ns_saturated) = if stats.stat_wait_count > 0 {
            let (sum, saturated) = saturating_u128_to_u64(stats.stat_wait_sum_ns);
            (Some(sum), saturated)
        } else {
            (None, false)
        };

        let stat_wait_count = if stats.stat_wait_count > 0 {
            Some(stats.stat_wait_count)
        } else {
            None
        };

        tasks.push(SessionTask {
            task: *task,
            active: stats.active,
            first_seen_ms: stats.first_seen_ms,
            last_seen_ms: stats.last_seen_ms,
            removed_ms: stats.removed_ms,
            class: stats.class,
            process_pid: stats.process_pid,
            process_comm: stats.process_comm.clone(),
            process_starttime_ticks: stats.process_starttime_ticks,
            task_starttime_ticks: stats.task_starttime_ticks,
            exe_dev: stats.exe_dev,
            exe_ino: stats.exe_ino,
            comm: stats.comm.clone(),
            latency: recorded_latency(latency),
            cpu: recorded_cpu(cpu),
            top_spikes: stats
                .top_spikes
                .iter()
                .map(|spike| recorded_spike(stats, spike))
                .collect(),
            migration_count: stats.migration_count,
            cross_numa_migrations: stats.cross_numa_migrations,
            top_wakers: stats
                .waker_counts
                .iter()
                .map(|(waker_tid, count)| WakerEntry {
                    waker_tid: *waker_tid,
                    waker_comm: stats_by_task
                        .get(waker_tid)
                        .map(|s| s.comm.clone())
                        .unwrap_or_else(|| "?".to_owned()),
                    count: *count,
                })
                .collect(),
            sched_policy: stats.sched_policy.map(|p| {
                crate::process_tree::sched_policy_name(p)
                    .map(|s| s.to_owned())
                    .unwrap_or_else(|| format!("UNKNOWN({})", p))
            }),
            stat_wait_sum_ns,
            stat_wait_sum_ns_saturated,
            stat_wait_count,
            cpu_perf: stats
                .session_cpu_perf
                .as_ref()
                .and_then(|perf| perf.snapshot()),
        });

        for spike in &stats.top_spikes {
            top_spikes.push(SessionSpike {
                task: *task,
                active: stats.active,
                class: stats.class,
                process_pid: stats.process_pid,
                process_comm: stats.process_comm.clone(),
                comm: stats.comm.clone(),
                cpu: spike.cpu,
                wakeup_target_cpu: spike.wakeup_target_cpu,
                prio: spike.prio,
                latency_ns: spike.latency_ns,
                wakeup_ns: spike.wakeup_ns,
                switch_ns: spike.switch_ns,
                switch_prev_pid: spike.switch_prev_pid,
                switch_prev_state: spike.switch_prev_state,
                switch_prev_state_label: spike.switch_prev_state_label.clone(),
                ..Default::default()
            });
        }
    }

    tasks.sort_by_key(|task| std::cmp::Reverse(task.latency.max_ns));
    top_spikes.sort_by_key(|spike| std::cmp::Reverse(spike.latency_ns));
    top_spikes.truncate(64);

    let core = SessionMetadataCore {
        schema_version: SESSION_SCHEMA_VERSION,
        run_name: recording.run_name.clone(),
        started_at: recorded_time(recording.started_at),
        ended_at: recorded_time(ended_at),
        monotonic_start_ns: recording.monotonic_start_ns,
        monotonic_end_ns,
        duration_ms,
        mangohud_start_offset: recording.mangohud_start_offset,
        mangohud_first_frame_monotonic_ns: recording.mangohud_first_frame_monotonic_ns,
        mangohud_first_frame_raw_elapsed_ms: recording.mangohud_first_frame_raw_elapsed_ms,
        metadata,
        target_pids_max: TARGET_PIDS_MAX as u64,
        active_target_pids_count: active_targets.len() as u64,
        active_expanded_tasks,
        focus_mode,
        final_focus_kind,
        focus_switch_count,
        focus_event_count: recorder.counters.focus_event_count,
        foreground_event_count: recorder.counters.foreground_event_count,
        foreground_source: final_foreground_event
            .as_ref()
            .map(|event| foreground_source_label(event.source)),
        final_foreground_pid: final_foreground_event.as_ref().and_then(|event| event.pid),
        final_foreground_app_id: final_foreground_event
            .as_ref()
            .and_then(|event| event.app_id.clone()),
        final_foreground_class: final_foreground_event
            .as_ref()
            .and_then(|event| event.class.clone()),
        interval_record_count,
        intervals_dropped: recorder.counters.intervals_dropped,
        spike_events_retained_count: if recorder.streams.contains(ArtifactKind::SpikeEvents) {
            recorder.counters.spike_event_count
        } else {
            spike_events.len() as u64
        },
        spike_events_dropped_count: recorder.counters.spike_events_dropped_count,
        spike_events_truncated: if recorder.streams.contains(ArtifactKind::SpikeEvents) {
            false
        } else {
            recorder
                .buffers
                .spike_events
                .as_ref()
                .map(|s| s.truncated)
                .unwrap_or(false)
        },
        scx_event_count: recorder.counters.scx_event_count,
        irq_event_count,
        migration_event_count: Some(recorder.counters.migration_event_count),
        cpu_freq_sample_count: Some(recorder.counters.cpu_freq_sample_count),
        gpu_sample_count,
        frame_event_count: if recorder.streams.contains(ArtifactKind::FrameEvents) {
            recorder.counters.frame_event_count
        } else {
            frame_events.len() as u64
        },
        block_io_event_count: recorder.counters.block_io_event_count,
        runtime_slice_count: recorder.counters.runtime_slice_count,
        runtime_slice_read_errors: recorder.counters.runtime_slice_read_errors,
        runtime_slice_skipped_tasks: recorder.counters.runtime_slice_skipped_tasks,
        runtime_slice_source: if recorder.counters.runtime_slice_count > 0 {
            Some("procfs".to_owned())
        } else {
            None
        },
        event_stream_write_errors: recorder.counters.event_stream_write_errors,
        alert_events_dropped_count: recorder.counters.alert_events_dropped_count,
        alert_channel_closed_count: recorder.counters.alert_channel_closed_count,
        first_event_stream_write_error: recorder.counters.first_event_stream_write_error.clone(),
        block_io_correlation_basis: block_io_correlation_basis.to_owned(),
        drop_counters: drop_counters.clone(),
        cpu_perf_sample_count: cpu_perf_status
            .as_ref()
            .map(|status| status.sample_count)
            .unwrap_or(0),
        cpu_perf_open_errors: cpu_perf_status
            .as_ref()
            .map(|status| status.open_errors)
            .unwrap_or(0),
        cpu_perf_read_errors: cpu_perf_status
            .as_ref()
            .map(|status| status.read_errors)
            .unwrap_or(0),
        cpu_perf_skipped_tasks: cpu_perf_status
            .as_ref()
            .map(|status| status.skipped_counter_tasks)
            .unwrap_or(0),
        cpu_perf_last_error: cpu_perf_status
            .as_ref()
            .and_then(|status| status.last_error.clone()),
    };

    let session = SessionFile {
        core: core.clone(),
        stop_reason: stop_reason.to_owned(),
        // TODO: replace with config::model::MonitorConfig → RecordedConfig conversion once MonitorConfig is the canonical config type.
        config: recorded_config(config, tree_pids),
        tasks,
        top_spikes,
    };

    let metadata_file = MetadataFile { core };

    // Map any write errors to a "record write failed" context so callers can decide
    // whether a failed recording should be treated as fatal.
    let map_write_err = |e: anyhow::Error| -> anyhow::Error { e.context("record write failed") };

    let mut sync_tracker = SyncTracker::default();

    write_json(
        recording.run_dir.join("session.json"),
        &session,
        &mut sync_tracker,
    )
    .map_err(map_write_err)?;
    write_json(
        recording.run_dir.join("metadata.json"),
        &metadata_file,
        &mut sync_tracker,
    )
    .map_err(map_write_err)?;

    if !recorder.streams.contains(ArtifactKind::Interval) {
        write_json_stream(
            recording.run_dir.join("interval.json"),
            interval_records,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }
    if !tree_events.is_empty() {
        write_json_stream(
            recording.run_dir.join("tree_events.json"),
            tree_events,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }
    if !recorder.streams.contains(ArtifactKind::SpikeEvents) && !spike_events.is_empty() {
        write_json_stream(
            recording.run_dir.join("spike_events.json"),
            spike_events,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }
    if !recorder.streams.contains(ArtifactKind::IrqEvents)
        && !recorder.buffers.irq_events.is_empty()
    {
        write_json_stream(
            recording.run_dir.join("irq_events.json"),
            &recorder.buffers.irq_events,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }
    if !recorder.streams.contains(ArtifactKind::GpuSamples)
        && !recorder.buffers.gpu_samples.is_empty()
    {
        write_json_stream(
            recording.run_dir.join("gpu_samples.json"),
            &recorder.buffers.gpu_samples,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }
    if !recorder.streams.contains(ArtifactKind::FrameEvents) && !frame_events.is_empty() {
        write_json_stream(
            recording.run_dir.join("frame_events.json"),
            frame_events,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }
    if !recorder.streams.contains(ArtifactKind::ScxEvents)
        && !recorder.buffers.scx_events.is_empty()
    {
        write_json_stream(
            recording.run_dir.join("scx_events.json"),
            &recorder.buffers.scx_events,
            &mut sync_tracker,
        )
        .map_err(map_write_err)?;
    }

    if !input.config.json_stream {
        println!("recording written to {}", recording.run_dir.display());
    }
    Ok(())
}

pub fn recorded_config(config: &Config, tree_pids: &[u32]) -> RecordedConfig {
    RecordedConfig {
        manual_pids: config.target_pids.clone(),
        tree_roots: tree_pids.to_vec(),
        cgroupv2: config.cgroupv2.clone(),
        exclude_tree_pids: config.exclude_tree_pids.clone(),
        include_comm: config
            .task_filters
            .include_comm
            .iter()
            .map(|p| p.raw().to_owned())
            .collect(),
        exclude_comm: config
            .task_filters
            .exclude_comm
            .iter()
            .map(|p| p.raw().to_owned())
            .collect(),
        watch_process: config.watch_process.clone(),
        persistent: config.persistent,
        keep_missing_pid: config.keep_missing_pid,
        watch_poll_ms: config.watch_poll_ms,
        watch_timeout_ms: config
            .watch_timeout
            .map(|timeout| timeout.as_millis() as u64),
        csv_stream: config.csv_stream.clone(),
        irq_latency: config.irq_latency,
        irqs: config.irqs.clone(),
        hwmon: config.hwmon,
        hwmon_root: config.hwmon_root.clone(),
        hwmon_drm_card: config.hwmon_drm_card.clone(),
        hwmon_render_node: config.hwmon_render_node.clone(),
        mangohud_log: config.mangohud_log.clone(),
        mangohud_log_live: config.mangohud_log_live,
        tui: config.tui,
        summary_period_ms: config.summary_period_ms,
        epoch_period_ms: config.epoch_period_ms,
        retain_intervals: config.retain_intervals,
        max_tasks: config.max_tasks,
        spike_threshold_ns: config.spike_threshold_ns,
        alert_threshold_ns: config.alert_threshold_ns,
        alert_webhook_url: config.alert_webhook_url.clone(),
        follow_exec: config.follow_exec,
        verbose: config.verbose,
        faults: config.faults,
        cpu_perf: config.cpu_perf,
        cpu_perf_kernel: config.cpu_perf_kernel,
        cpu_perf_max_tasks: config.cpu_perf_max_tasks,
        cpu_perf_cache_refs: config.cpu_perf_cache_refs,
        block_io: config.block_io,
        stat_wait: config.stat_wait,
        runtime_slices: config.runtime_slices,
        runtime_slices_max_tasks: config.runtime_slices_max_tasks,
        otlp_endpoint: config.otlp_endpoint.clone(),
        otel_service_name: config.otel_service_name.clone(),
        auto_focus: config.auto_focus,
        foreground_window: config.foreground_window,
        focus_source: focus_source_label(config.focus_source),
        foreground_source: foreground_source_arg_label(config.foreground_source),
        foreground_poll_ms: config.foreground_poll_ms,
        foreground_max_stale_ms: config.foreground_max_stale_ms,
        foreground_include_title: config.foreground_include_title,
        auto_focus_poll_ms: config.auto_focus_poll_ms,
        auto_focus_min_confidence: config.auto_focus_min_confidence,
        auto_focus_switch_cooldown_ms: config.auto_focus_switch_cooldown_ms,
        auto_focus_switch_margin: config.auto_focus_switch_margin,
        auto_focus_required_polls: config.auto_focus_required_polls,
        auto_focus_max_roots: config.auto_focus_max_roots,
    }
}

#[cfg(test)]
pub fn write_interval_csv(
    path: &std::path::Path,
    interval_records: &[IntervalRecord],
) -> anyhow::Result<()> {
    let mut writer = IntervalCsvWriter::create_file(path.to_path_buf())?;

    for record in interval_records {
        writer.push(record)?;
    }

    writer.finish()
}

fn write_interval_csv_header(file: &mut dyn io::Write) -> io::Result<()> {
    writeln!(
        file,
        "elapsed_ms,task,active,class,comm,process_pid,process_comm,samples,stored_samples,truncated_samples,min_ns,avg_ns,p95_ns,p99_ns,max_ns,over_1ms,over_2ms,over_5ms,busiest_cpu,busiest_cpu_samples,worst_cpu,worst_cpu_max_ns,spikiest_cpu,spikiest_cpu_spikes,percentile_scope,major_faults,minor_faults,cpu_psi_some,mem_psi_some,mem_psi_full,io_psi_some,io_psi_full,cumulative_drop_counters_total,cpu_cycles,cpu_instructions,cpu_ipc,cache_references,cache_misses,cache_miss_rate,cache_mpki,cpu_perf_multiplexed,cpu_perf_scaled,cpu_perf_unavailable_reason"
    )
}

fn write_interval_csv_row(file: &mut dyn io::Write, record: &IntervalRecord) -> io::Result<()> {
    let cpu_perf = record.cpu_perf.as_ref();
    writeln!(
        file,
        "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
        record.elapsed_ms,
        record.task,
        record.active,
        record.class,
        csv_escape(&record.comm),
        option_u32(record.process_pid),
        csv_escape(&record.process_comm),
        record.samples,
        record.stored_samples,
        record.truncated_samples,
        record.min_ns,
        record.avg_ns,
        record.p95_ns,
        record.p99_ns,
        record.max_ns,
        record.over_1ms,
        record.over_2ms,
        record.over_5ms,
        option_u32(record.busiest_cpu),
        record.busiest_cpu_samples,
        option_u32(record.worst_cpu),
        record.worst_cpu_max_ns,
        option_u32(record.spikiest_cpu),
        record.spikiest_cpu_spikes,
        csv_escape(&record.percentile_scope),
        record.major_faults,
        record.minor_faults,
        record.cpu_psi_some,
        record.mem_psi_some,
        record.mem_psi_full,
        record.io_psi_some,
        record.io_psi_full,
        record.drop_counters.total(),
        option_u64(cpu_perf.and_then(|perf| perf.cycles)),
        option_u64(cpu_perf.and_then(|perf| perf.instructions)),
        option_f64(cpu_perf.and_then(|perf| perf.ipc)),
        option_u64(cpu_perf.and_then(|perf| perf.cache_references)),
        option_u64(cpu_perf.and_then(|perf| perf.cache_misses)),
        option_f64(cpu_perf.and_then(|perf| perf.cache_miss_rate)),
        option_f64(cpu_perf.and_then(|perf| perf.cache_mpki)),
        option_bool(cpu_perf.map(|perf| perf.multiplexed)),
        option_bool(cpu_perf.map(|perf| perf.scaled)),
        csv_escape(
            cpu_perf
                .and_then(|perf| perf.unavailable_reason.as_deref())
                .unwrap_or("")
        ),
    )
}

fn recorded_latency(latency: crate::metrics::LatencySnapshot) -> RecordedLatency {
    RecordedLatency {
        samples: latency.count,
        stored_samples: latency.stored_samples,
        truncated_samples: latency.samples_truncated,
        percentile_scope: latency.percentile_scope,
        histogram: latency.histogram,
        min_ns: latency.min_ns,
        avg_ns: latency.avg_ns,
        p95_ns: latency.p95_ns,
        p99_ns: latency.p99_ns,
        max_ns: latency.max_ns,
        over_1ms: latency.over_1ms,
        over_2ms: latency.over_2ms,
        over_5ms: latency.over_5ms,
    }
}

pub(crate) fn saturating_u128_to_u64(value: u128) -> (u64, bool) {
    if value > u64::MAX as u128 {
        (u64::MAX, true)
    } else {
        (value as u64, false)
    }
}

fn recorded_cpu(cpu: CpuSnapshot) -> RecordedCpuSnapshot {
    RecordedCpuSnapshot {
        busiest_cpu: cpu.busiest_cpu,
        busiest_cpu_samples: cpu.busiest_cpu_samples,
        worst_cpu: cpu.worst_cpu,
        worst_cpu_max_ns: cpu.worst_cpu_max_ns,
        spikiest_cpu: cpu.spikiest_cpu,
        spikiest_cpu_spikes: cpu.spikiest_cpu_spikes,
        per_cpu: cpu.per_cpu,
    }
}

fn recorded_spike(stats: &TaskStats, spike: &SpikeRecord) -> RecordedSpike {
    RecordedSpike {
        class: stats.class,
        process_pid: stats.process_pid,
        process_comm: stats.process_comm.clone(),
        cpu: spike.cpu,
        wakeup_target_cpu: spike.wakeup_target_cpu,
        switch_prev_pid: spike.switch_prev_pid,
        switch_prev_state: spike.switch_prev_state,
        switch_prev_state_label: spike.switch_prev_state_label.clone(),
        prio: spike.prio,
        latency_ns: spike.latency_ns,
        wakeup_ns: spike.wakeup_ns,
        switch_ns: spike.switch_ns,
        waker_tid: 0, // Not currently persisted in SpikeRecord
        waker_comm: String::new(),
        target_pending_wakeups: spike.target_pending_wakeups,
        observed_runnable_depth: spike.observed_runnable_depth,
        major_faults: spike.major_faults,
        minor_faults: spike.minor_faults,
        scx_ops: spike.scx_ops.clone(),
        scx_state: spike.scx_state.clone(),
        scx_enable_seq: spike.scx_enable_seq.clone(),
        cause_tags: spike.cause_tags.clone(),
        primary_cause: spike.primary_cause.clone(),
    }
}

fn resolve_run_dir(
    recording: &RecordingConfig,
    started_at: SystemTime,
    home: Option<std::ffi::OsString>,
) -> PathBuf {
    if let Some(out_dir) = &recording.out_dir {
        return out_dir.clone();
    }

    let mut base = home
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.push(".local");
    base.push("state");
    base.push("stutter");
    base.push("runs");

    let run_name = recording.run_name.as_deref().unwrap_or("run");
    base.push(format!(
        "{}_{}",
        timestamp_for_path(started_at),
        sanitize_run_name(run_name)
    ));
    base
}

fn ensure_empty_dir(path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        anyhow::bail!("output directory already exists: {}", path.display());
    }

    fs::create_dir_all(path)?;
    Ok(())
}

pub fn recorded_time(time: SystemTime) -> RecordedTime {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();

    RecordedTime {
        unix_seconds: duration.as_secs(),
        unix_nanos: duration.subsec_nanos(),
        system_time_debug: format!("{time:?}"),
    }
}

fn timestamp_for_path(time: SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    format!("{}_{:09}", duration.as_secs(), duration.subsec_nanos())
}

fn sanitize_run_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

pub(crate) fn monotonic_now_ns() -> Option<u64> {
    static CLOCK_ID: std::sync::OnceLock<libc::clockid_t> = std::sync::OnceLock::new();
    let clock_id = CLOCK_ID.get_or_init(|| {
        if is_kernel_before_5_7() {
            libc::CLOCK_MONOTONIC_RAW
        } else {
            libc::CLOCK_MONOTONIC
        }
    });

    let mut timespec = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };

    // SAFETY: clock_gettime writes to the provided valid timespec pointer and
    // does not retain it after the call. We select CLOCK_MONOTONIC or
    // CLOCK_MONOTONIC_RAW based on the kernel version to match bpf_ktime_get_ns()
    // behavior, so recorded elapsed times line up with eBPF timestamps.
    let result = unsafe { libc::clock_gettime(*clock_id, &mut timespec) };
    if result != 0 {
        return None;
    }

    timespec_to_ns(timespec)
}

fn is_kernel_before_5_7() -> bool {
    let mut uts = std::mem::MaybeUninit::<libc::utsname>::uninit();
    // SAFETY: uts is a valid pointer to a libc::utsname struct.
    if unsafe { libc::uname(uts.as_mut_ptr()) } != 0 {
        return false;
    }
    // SAFETY: uname succeeded and initialized the struct.
    let uts = unsafe { uts.assume_init() };
    // SAFETY: release field is a null-terminated string.
    let release = unsafe { std::ffi::CStr::from_ptr(uts.release.as_ptr()) };
    let release_str = release.to_string_lossy();

    let mut parts = release_str.split('.');
    let major: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);

    major < 5 || (major == 5 && minor < 7)
}

fn timespec_to_ns(timespec: libc::timespec) -> Option<u64> {
    if timespec.tv_sec < 0 || timespec.tv_nsec < 0 {
        return None;
    }

    let seconds = u64::try_from(timespec.tv_sec).ok()?;
    let nanos = u64::try_from(timespec.tv_nsec).ok()?;
    if nanos >= 1_000_000_000 {
        return None;
    }

    seconds.checked_mul(1_000_000_000)?.checked_add(nanos)
}

fn elapsed_ms_from_monotonic(monotonic_start_ns: Option<u64>, switch_ns: u64) -> Option<u64> {
    let start_ns = monotonic_start_ns?;
    switch_ns
        .checked_sub(start_ns)
        .map(|elapsed_ns| elapsed_ns / 1_000_000)
}

fn option_u32(value: Option<u32>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn option_u64(value: Option<u64>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn option_f64(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite())
        .map(|value| format!("{value:.6}"))
        .unwrap_or_default()
}

fn option_bool(value: Option<bool>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

#[derive(Debug, Default)]
pub struct SyncTracker {
    synced_dirs: BTreeSet<PathBuf>,
}

impl SyncTracker {
    pub fn sync_parent_once(&mut self, path: &Path) -> anyhow::Result<()> {
        let Some(parent) = path.parent() else {
            return Ok(());
        };

        let parent = parent.to_path_buf();

        if self.synced_dirs.insert(parent.clone()) {
            let dir = fs::File::open(&parent).with_context(|| {
                format!(
                    "failed to open parent directory {} for sync",
                    parent.display()
                )
            })?;

            dir.sync_all()
                .with_context(|| format!("failed to sync parent directory {}", parent.display()))?;
        }

        Ok(())
    }

    #[cfg(test)]
    fn synced_dir_count_for_test(&self) -> usize {
        self.synced_dirs.len()
    }

    #[cfg(test)]
    fn mark_parent_for_test(&mut self, path: &Path) {
        if let Some(parent) = path.parent() {
            self.synced_dirs.insert(parent.to_path_buf());
        }
    }
}

fn write_json<T: ?Sized + Serialize>(
    path: PathBuf,
    value: &T,
    sync_tracker: &mut SyncTracker,
) -> anyhow::Result<()> {
    let file_name = path
        .file_name()
        .and_then(|name| {
            if name.is_empty() {
                None
            } else {
                Some(name.to_string_lossy())
            }
        })
        .ok_or_else(|| anyhow::anyhow!("JSON destination has no file name: {}", path.display()))?;
    let tmp_path = path.with_file_name(format!("{file_name}.tmp"));
    let mut file = fs::File::create(&tmp_path)
        .with_context(|| format!("failed to create temp JSON {}", tmp_path.display()))?;
    file.write_all(&serde_json::to_vec_pretty(value)?)
        .with_context(|| format!("failed to write temp JSON {}", tmp_path.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("failed to finalize temp JSON {}", tmp_path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to sync temp JSON {}", tmp_path.display()))?;
    drop(file);
    fs::rename(&tmp_path, &path)
        .with_context(|| format!("failed to rename temp JSON {}", tmp_path.display()))?;

    sync_tracker.sync_parent_once(&path)?;

    Ok(())
}

fn write_json_stream<T: Serialize>(
    path: PathBuf,
    values: &[T],
    sync_tracker: &mut SyncTracker,
) -> anyhow::Result<()> {
    let mut writer = NdjsonWriter::create(path.clone())?;
    for value in values {
        writer.push(value)?;
    }
    writer.finish()?;

    sync_tracker.sync_parent_once(&path)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ndjson_writer_outputs_valid_stream() {
        let dir = temp_dir("ndjson-writer");
        fs::create_dir_all(&dir).unwrap();
        let empty_path = dir.join("empty.json");
        {
            let mut writer = NdjsonWriter::create(empty_path.clone()).unwrap();
            writer.finish().unwrap();
        }
        assert!(fs::read_to_string(&empty_path).unwrap().is_empty());

        let single_path = dir.join("single.json");
        {
            let mut writer = NdjsonWriter::create(single_path.clone()).unwrap();
            writer.push(&serde_json::json!({"one": true})).unwrap();
            writer.finish().unwrap();
        }
        let single: Vec<serde_json::Value> =
            serde_json::Deserializer::from_reader(fs::File::open(&single_path).unwrap())
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        assert_eq!(single.len() as u64, 1);

        let path = dir.join("items.json");

        {
            let mut writer = NdjsonWriter::create(path.clone()).unwrap();
            writer.push(&serde_json::json!({"a": 1})).unwrap();
            writer.push(&serde_json::json!({"b": 2})).unwrap();
            writer.finish().unwrap();
        }

        let values: Vec<serde_json::Value> =
            serde_json::Deserializer::from_reader(fs::File::open(&path).unwrap())
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        assert_eq!(values.len() as u64, 2);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn json_array_writer_rejects_path_without_file_name() {
        let err = NdjsonWriter::create(PathBuf::from("/")).unwrap_err();
        assert!(err.to_string().contains("no file name"));
    }

    #[test]
    fn write_json_rejects_path_without_file_name() {
        let err = write_json(
            PathBuf::from("/"),
            &serde_json::json!({}),
            &mut SyncTracker::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("no file name"));
    }

    #[test]
    fn spike_event_defaults_switch_prev_fields_for_old_json() {
        // Populate a few required fields so serialization yields a full object.
        let s = SpikeEvent {
            task: 1,
            class: crate::process_tree::TaskClass::Game,
            comm: "test".to_owned(),
            process_comm: "test".into(),
            ..Default::default()
        };

        let mut val = serde_json::to_value(&s).unwrap();
        if let serde_json::Value::Object(ref mut map) = val {
            map.remove("switch_prev_pid");
            map.remove("switch_prev_state");
            map.remove("switch_prev_state_label");
        } else {
            panic!("expected object");
        }

        let json = serde_json::to_string(&val).unwrap();
        let decoded: SpikeEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.switch_prev_pid, 0);
        assert_eq!(decoded.switch_prev_state, 0);
        assert_eq!(decoded.switch_prev_state_label, "");
    }

    #[test]
    fn spike_point_preserves_switch_prev_context() {
        let stats = crate::metrics::TaskStats::new(42, "t".to_owned(), 0);
        let spike = crate::metrics::SpikeRecord {
            latency_ns: 100,
            cpu: 1,
            wakeup_target_cpu: 0,
            prio: 0,
            wakeup_ns: 10,
            switch_ns: 110,
            switch_prev_pid: 99,
            switch_prev_state: 1,
            switch_prev_state_label: "voluntary_sleep_interruptible".to_owned(),
            ..crate::metrics::SpikeRecord::default()
        };

        let rec = recorded_spike(&stats, &spike);
        assert_eq!(rec.switch_prev_pid, 99);
        assert_eq!(rec.switch_prev_state, 1);
    }

    #[test]
    fn interval_csv_writer_streams_header_and_rows() {
        let dir = temp_dir("interval-csv-writer");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("interval.csv");

        {
            let mut writer = IntervalCsvWriter::create_file(path.clone()).unwrap();
            writer.push(&test_interval_record()).unwrap();
            writer.finish().unwrap();
        }

        let csv = fs::read_to_string(&path).unwrap();
        assert!(csv.starts_with("elapsed_ms,task,active"));
        assert!(csv.contains("worker"));
        fs::remove_dir_all(dir).ok();
    }

    fn test_interval_record() -> IntervalRecord {
        IntervalRecord {
            elapsed_ms: 1,
            task: 2,
            active: true,
            class: TaskClass::Game,
            comm: "worker".to_owned(),
            process_pid: Some(2),
            process_comm: "game".into(),
            samples: 1,
            stored_samples: 1,
            truncated_samples: 0,
            min_ns: 1,
            avg_ns: 1,
            p95_ns: 1,
            p99_ns: 1,
            major_faults: 0,
            minor_faults: 0,
            max_ns: 1,
            over_1ms: 0,
            over_2ms: 0,
            over_5ms: 0,
            busiest_cpu: None,
            busiest_cpu_samples: 0,
            worst_cpu: None,
            worst_cpu_max_ns: 0,
            spikiest_cpu: None,
            spikiest_cpu_spikes: 0,
            cpu_psi_some: 0.0,
            mem_psi_some: 0.0,
            mem_psi_full: 0.0,
            io_psi_some: 0.0,
            io_psi_full: 0.0,
            percentile_scope: "all".to_owned(),
            histogram: Vec::new(),
            drop_counters: crate::ebpf_loader::DropCountersSnapshot::default(),
            ..Default::default()
        }
    }

    #[test]
    fn test_spike_event_buffer_truncation() {
        let mut buf = SpikeEventBuffer::with_max_events(1);
        let event = SpikeEvent {
            elapsed_ms: Some(0),
            task: 1,
            active: true,
            class: TaskClass::Unknown,
            process_pid: Some(1),
            process_comm: "test".into(),
            comm: "test".to_owned(),
            latency_ns: 1000,
            ..Default::default()
        };

        assert_eq!(buf.push(event.clone()), SpikePushResult::Stored);
        assert_eq!(buf.push(event), SpikePushResult::Dropped);
        assert!(buf.truncated());
    }

    #[test]
    fn test_spike_event_serialization() {
        let event = SpikeEvent {
            elapsed_ms: Some(100),
            task: 123,
            active: true,
            class: TaskClass::Game,
            process_pid: Some(123),
            process_comm: "game".into(),
            comm: "game".to_owned(),
            cpu: 1,
            wakeup_target_cpu: 1,
            prio: 120,
            latency_ns: 1_000_000,
            wakeup_ns: 2000,
            switch_ns: 3000,
            major_faults: 1,
            minor_faults: 2,
            scx_ops: Some("scx_lavd".to_owned()),
            scx_state: Some("enabled".to_owned()),
            scx_enable_seq: Some("1".to_owned()),
            cause_tags: vec!["cpu_pressure".to_string()],
            primary_cause: Some("cpu_pressure".to_string()),
            ..Default::default()
        };

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("\"scx_ops\":\"scx_lavd\""));
        assert!(json.contains("\"scx_state\":\"enabled\""));

        let decoded: SpikeEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.scx_ops.as_deref(), Some("scx_lavd"));
        assert_eq!(decoded.scx_state.as_deref(), Some("enabled"));
    }

    #[test]
    fn test_spike_event_deserialization_compatibility() {
        let json = r#"{"elapsed_ms":100,"task":123,"active":true,"class":"Game","process_pid":123,"process_comm":"game","comm":"game","cpu":1,"wakeup_target_cpu":1,"prio":120,"latency_ns":1000000,"wakeup_ns":2000,"switch_ns":3000,"target_pending_wakeups":0,"major_faults":1,"minor_faults":2}"#;
        let decoded: SpikeEvent = serde_json::from_str(json).unwrap();
        assert!(decoded.scx_ops.is_none());
        assert!(decoded.scx_state.is_none());
    }

    #[test]
    fn test_write_ndjson_value() {
        let event = SpikeEvent {
            elapsed_ms: Some(100),
            task: 123,
            active: true,
            class: TaskClass::Game,
            process_pid: Some(123),
            process_comm: "game".into(),
            comm: "game".to_owned(),
            cpu: 1,
            wakeup_target_cpu: 1,
            prio: 120,
            latency_ns: 1_000_000,
            wakeup_ns: 2000,
            switch_ns: 3000,
            target_pending_wakeups: 0,
            major_faults: 1,
            minor_faults: 2,
            ..Default::default()
        };

        let mut buf = Vec::new();
        write_ndjson_value(&mut buf, &event).unwrap();
        write_ndjson_value(&mut buf, &event).unwrap();

        let output = String::from_utf8(buf).unwrap();
        let lines: Vec<_> = output.lines().collect();
        assert_eq!(lines.len(), 2);

        for line in lines {
            let decoded: serde_json::Value = serde_json::from_str(line).unwrap();
            assert!(decoded.is_object());
            assert_eq!(decoded["task"], 123);
        }
    }

    #[test]
    fn recording_warnings_include_intervals_dropped() {
        let recorder = LiveRecorder {
            counters: RecordingCounters {
                intervals_dropped: 3,
                ..Default::default()
            },
            ..Default::default()
        };

        let warnings = recording_warnings(&recorder);

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("3 interval record(s) were dropped"));
        assert!(warnings[0].contains("--retain-intervals"));
    }

    #[test]
    fn recording_warnings_include_spike_events_dropped() {
        let recorder = LiveRecorder {
            counters: RecordingCounters {
                spike_events_dropped_count: 2,
                ..Default::default()
            },
            ..Default::default()
        };

        let warnings = recording_warnings(&recorder);

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("2 spike event record(s)"));
    }

    #[test]
    fn recording_warnings_include_event_stream_write_errors() {
        let recorder = LiveRecorder {
            counters: RecordingCounters {
                event_stream_write_errors: 4,
                ..Default::default()
            },
            ..Default::default()
        };

        let warnings = recording_warnings(&recorder);

        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("4 event stream write error(s)"));
        assert!(warnings[0].contains("incomplete"));
    }

    #[test]
    fn recording_warnings_include_all_recording_problems() {
        let recorder = LiveRecorder {
            counters: RecordingCounters {
                intervals_dropped: 1,
                spike_events_dropped_count: 2,
                event_stream_write_errors: 3,
                ..Default::default()
            },
            ..Default::default()
        };

        let warnings = recording_warnings(&recorder);

        assert_eq!(warnings.len(), 3);
    }

    #[test]
    fn recording_warnings_empty_for_clean_recorder() {
        let recorder = LiveRecorder::default();

        let warnings = recording_warnings(&recorder);

        assert!(warnings.is_empty());
    }

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        dir
    }

    #[test]
    fn sync_tracker_tracks_parent_once_for_same_directory() {
        let mut tracker = SyncTracker::default();

        tracker.mark_parent_for_test(Path::new("run-a/session.json"));
        tracker.mark_parent_for_test(Path::new("run-a/metadata.json"));

        assert_eq!(tracker.synced_dir_count_for_test(), 1);
    }

    #[test]
    fn sync_tracker_tracks_distinct_parent_directories() {
        let mut tracker = SyncTracker::default();

        tracker.mark_parent_for_test(Path::new("run-a/session.json"));
        tracker.mark_parent_for_test(Path::new("run-b/session.json"));

        assert_eq!(tracker.synced_dir_count_for_test(), 2);
    }

    #[test]
    fn sync_parent_once_does_not_error_for_existing_parent() {
        let dir = temp_dir("sync-tracker");
        fs::create_dir_all(&dir).unwrap();

        let path = dir.join("session.json");
        fs::write(&path, "{}\n").unwrap();

        let mut tracker = SyncTracker::default();
        tracker.sync_parent_once(&path).unwrap();
        tracker
            .sync_parent_once(&dir.join("metadata.json"))
            .unwrap();

        assert_eq!(tracker.synced_dir_count_for_test(), 1);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn block_io_record_correlation_basis_serializes_as_string() {
        let record = crate::recorder::BlockIoRecord {
            elapsed_ms: 1,
            tid: 42,
            correlation_basis: std::borrow::Cow::Borrowed("dev+sector"),
            dev: 1,
            nr_sector: 8,
            sector: 123,
            duration_ns: 456,
            timestamp_ns: 789,
            rwbs: "R".to_string(),
        };

        let value = serde_json::to_value(&record).unwrap();

        assert_eq!(value["correlation_basis"], "dev+sector");
        assert!(value.get("correlation_basis").unwrap().is_string());
    }

    #[test]
    fn block_io_record_correlation_basis_deserializes_from_string() {
        let json = serde_json::json!({
            "elapsed_ms": 1,
            "tid": 42,
            "correlation_basis": "request-pointer",
            "dev": 1,
            "nr_sector": 8,
            "sector": 123,
            "duration_ns": 456,
            "timestamp_ns": 789,
            "rwbs": "R"
        });

        let record: crate::recorder::BlockIoRecord = serde_json::from_value(json).unwrap();

        assert_eq!(record.correlation_basis.as_ref(), "request-pointer");
    }
}
