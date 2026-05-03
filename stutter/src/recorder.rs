use std::{
    collections::BTreeMap,
    env, fs, io,
    io::Write,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use stutter_common::SchedulerEvent;

use crate::{
    TARGET_PIDS_MAX,
    cli::{Config, RecordingConfig},
    ebpf_loader::DropCountersSnapshot,
    error::StutterError,
    metadata::{SystemMetadata, collect_system_metadata},
    metrics::{
        CpuLine, CpuSnapshot, IntervalRecord as MetricsIntervalRecord, LatencyHistogramBucket,
        SpikeRecord, TaskStats,
    },
    process_tree::{TaskClass, TaskInfo},
    scx::ScxEvent,
};

pub type IntervalRecord = MetricsIntervalRecord;
pub const MAX_SPIKE_EVENTS: usize = 500_000;

#[derive(Debug)]
pub struct RecordingRun {
    pub run_name: Option<String>,
    pub run_dir: PathBuf,
    pub started_at: SystemTime,
    pub started_instant: Instant,
    pub monotonic_start_ns: Option<u64>,
}

#[derive(Debug)]
pub struct JsonArrayWriter {
    file: fs::File,
    wrote_any: bool,
    finished: bool,
    path: PathBuf,
}

pub struct IntervalCsvWriter {
    file: fs::File,
    path: PathBuf,
    finished: bool,
}

impl IntervalCsvWriter {
    pub fn create(path: PathBuf) -> anyhow::Result<Self> {
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
            file,
            path,
            finished: false,
        })
    }

    pub fn push(&mut self, record: &IntervalRecord) -> anyhow::Result<()> {
        write_interval_csv_row(&mut self.file, record)
            .with_context(|| format!("failed to write interval CSV {}", self.path.display()))
    }

    pub fn finish(&mut self) -> anyhow::Result<()> {
        if self.finished {
            return Ok(());
        }
        self.file
            .sync_all()
            .with_context(|| format!("failed to sync interval CSV {}", self.path.display()))?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for IntervalCsvWriter {
    fn drop(&mut self) {
        if let Err(err) = self.finish() {
            log::warn!(
                "interval_csv_finish_failed path={} err={err:#}",
                self.path.display()
            );
        }
    }
}

impl JsonArrayWriter {
    pub fn create(path: PathBuf) -> anyhow::Result<Self> {
        if path.file_name().is_none() {
            anyhow::bail!(
                "JSON stream destination has no file name: {}",
                path.display()
            );
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let file = fs::File::create(&path)
            .with_context(|| format!("failed to create JSON stream {}", path.display()))?;

        Ok(Self {
            file,
            wrote_any: false,
            finished: false,
            path,
        })
    }

    pub fn push<T: Serialize>(&mut self, value: &T) -> anyhow::Result<()> {
        if self.finished {
            anyhow::bail!("JSON stream {} is already finalized", self.path.display());
        }

        serde_json::to_writer(&mut self.file, value)
            .with_context(|| format!("failed to write JSON stream {}", self.path.display()))?;
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
            .with_context(|| format!("failed to sync JSON stream {}", self.path.display()))?;
        self.finished = true;
        Ok(())
    }
}

impl Drop for JsonArrayWriter {
    fn drop(&mut self) {
        if let Err(err) = self.finish() {
            log::warn!(
                "json_array_finish_failed path={} err={err:#}",
                self.path.display()
            );
        }
    }
}

pub struct SpikeEventBuffer {
    events: Vec<SpikeEvent>,
    truncated: bool,
    max_events: usize,
    dropped_count: usize,
}

impl Default for SpikeEventBuffer {
    fn default() -> Self {
        Self {
            // Avoid eagerly allocating a very large buffer for spike events.
            // Start empty and grow on demand.
            events: Vec::new(),
            truncated: false,
            max_events: MAX_SPIKE_EVENTS,
            dropped_count: 0,
        }
    }
}

impl SpikeEventBuffer {
    #[cfg(test)]
    pub fn with_max_events(max_events: usize) -> Self {
        Self {
            events: Vec::new(),
            truncated: false,
            max_events,
            dropped_count: 0,
        }
    }

    pub fn push(&mut self, event: SpikeEvent) {
        if self.events.len() < self.max_events {
            self.events.push(event);
        } else {
            self.truncated = true;
            self.dropped_count = self.dropped_count.saturating_add(1);
        }
    }

    pub fn as_slice(&self) -> &[SpikeEvent] {
        &self.events
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }
    pub fn dropped_count(&self) -> usize {
        self.dropped_count
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TreeEvent {
    pub elapsed_ms: u128,
    pub action: String,
    pub tid: u32,
    pub process_pid: u32,
    pub process_ppid: u32,
    pub comm: String,
    pub process_comm: std::sync::Arc<str>,
    pub class: TaskClass,
    pub from_cgroup: bool,
}

impl TreeEvent {
    pub fn from_task(started: Instant, action: &str, task: &TaskInfo) -> Self {
        Self {
            elapsed_ms: started.elapsed().as_millis(),
            action: action.to_owned(),
            tid: task.tid,
            process_pid: task.process_pid,
            process_ppid: task.process_ppid,
            comm: task.comm.clone(),
            process_comm: task.process_comm.clone(),
            class: task.class,
            from_cgroup: task.from_cgroup,
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct SessionFile {
    pub schema_version: u32,
    pub run_name: Option<String>,
    pub started_at: RecordedTime,
    pub ended_at: RecordedTime,
    pub monotonic_start_ns: Option<u64>,
    pub monotonic_end_ns: Option<u64>,
    pub duration_ms: u128,
    pub stop_reason: String,
    pub config: RecordedConfig,
    pub metadata: SystemMetadata,
    pub target_pids_max: usize,
    pub active_target_pids_count: usize,
    pub active_expanded_tasks: Vec<u32>,
    #[serde(default)]
    pub interval_record_count: usize,
    #[serde(default)]
    pub intervals_dropped: usize,
    #[serde(default)]
    pub spike_events_retained_count: usize,
    #[serde(default)]
    pub spike_events_dropped_count: usize,
    #[serde(default)]
    pub spike_events_truncated: bool,
    #[serde(default)]
    pub scx_event_count: usize,
    #[serde(default)]
    pub irq_event_count: usize,
    #[serde(default)]
    pub migration_event_count: Option<usize>,
    #[serde(default)]
    pub cpu_freq_sample_count: Option<usize>,
    #[serde(default)]
    pub gpu_sample_count: usize,
    #[serde(default)]
    pub frame_event_count: usize,
    #[serde(default)]
    pub block_io_event_count: usize,
    #[serde(default = "default_block_io_correlation_basis")]
    pub block_io_correlation_basis: String,
    #[serde(default)]
    pub drop_counters: DropCountersSnapshot,
    pub tasks: Vec<SessionTask>,
    pub top_spikes: Vec<SessionSpike>,
}

#[derive(Serialize, Deserialize)]
pub struct MetadataFile {
    pub schema_version: u32,
    pub run_name: Option<String>,
    pub started_at: RecordedTime,
    pub ended_at: RecordedTime,
    pub monotonic_start_ns: Option<u64>,
    pub monotonic_end_ns: Option<u64>,
    pub duration_ms: u128,
    pub metadata: SystemMetadata,
    pub target_pids_max: usize,
    pub active_target_pids_count: usize,
    pub active_expanded_tasks: Vec<u32>,
    #[serde(default)]
    pub interval_record_count: usize,
    #[serde(default)]
    pub intervals_dropped: usize,
    #[serde(default)]
    pub spike_events_retained_count: usize,
    #[serde(default)]
    pub spike_events_dropped_count: usize,
    #[serde(default)]
    pub spike_events_truncated: bool,
    #[serde(default)]
    pub scx_event_count: usize,
    #[serde(default)]
    pub irq_event_count: usize,
    #[serde(default)]
    pub migration_event_count: Option<usize>,
    #[serde(default)]
    pub cpu_freq_sample_count: Option<usize>,
    #[serde(default)]
    pub gpu_sample_count: usize,
    #[serde(default)]
    pub frame_event_count: usize,
    #[serde(default)]
    pub block_io_event_count: usize,
    #[serde(default = "default_block_io_correlation_basis")]
    pub block_io_correlation_basis: String,
    #[serde(default)]
    pub drop_counters: DropCountersSnapshot,
}

#[derive(Serialize, Deserialize)]
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
    pub watch_timeout_ms: Option<u128>,
    #[serde(default)]
    pub csv_path: Option<PathBuf>,
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
    pub block_io: bool,
    #[serde(default)]
    pub stat_wait: bool,
}

fn default_recorded_max_tasks() -> usize {
    TARGET_PIDS_MAX
}

fn default_recorded_follow_exec() -> bool {
    true
}

#[derive(Serialize, Deserialize)]
pub struct RecordedTime {
    pub unix_seconds: u64,
    pub unix_nanos: u32,
    #[serde(alias = "local")]
    pub system_time_debug: String,
}

#[derive(Serialize, Deserialize)]
pub struct SessionTask {
    pub task: u32,
    pub active: bool,
    pub first_seen_ms: u128,
    pub last_seen_ms: u128,
    pub removed_ms: Option<u128>,
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
    pub stat_wait_count: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct WakerEntry {
    pub waker_tid: u32,
    pub waker_comm: String,
    pub count: u64,
}

#[derive(Serialize, Deserialize)]
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

#[derive(Serialize, Deserialize)]
pub struct RecordedCpuSnapshot {
    pub busiest_cpu: Option<u32>,
    pub busiest_cpu_samples: u64,
    pub worst_cpu: Option<u32>,
    pub worst_cpu_max_ns: u64,
    pub spikiest_cpu: Option<u32>,
    pub spikiest_cpu_spikes: u64,
    pub per_cpu: Vec<CpuLine>,
}

#[derive(Clone, Serialize, Deserialize)]
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
    // Diagnostic-only: see docs in `metrics::SpikeRecord`.
    // Do not use this field in scoring or tuning decisions.
    #[serde(alias = "target_runnable_depth")]
    pub target_pending_wakeups: u32,
    #[serde(default)]
    pub major_faults: u64,
    #[serde(default)]
    pub minor_faults: u64,
}

#[derive(Clone, Serialize, Deserialize)]
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
    // Diagnostic-only: see docs in `metrics::SpikeRecord`.
    // Do not use this field in scoring or tuning decisions.
    #[serde(alias = "target_runnable_depth")]
    pub target_pending_wakeups: u32,
    #[serde(default)]
    pub major_faults: u64,
    #[serde(default)]
    pub minor_faults: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SpikeEvent {
    #[serde(default)]
    pub elapsed_ms: Option<u128>,
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
    // Diagnostic-only: see docs in `metrics::SpikeRecord`.
    // Do not use this field in scoring or tuning decisions.
    #[serde(alias = "target_runnable_depth")]
    pub target_pending_wakeups: u32,
    #[serde(default)]
    pub major_faults: u64,
    #[serde(default)]
    pub minor_faults: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct MigrationEventRecord {
    pub elapsed_ms: u128,
    pub tid: u32,
    pub from_cpu: u32,
    pub to_cpu: u32,
    pub timestamp_ns: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CpuFreqRecord {
    pub elapsed_ms: u128,
    pub cpu: u32,
    pub freq_khz: u32,
    pub timestamp_ns: u64,
}

impl SpikeEvent {
    pub fn from_task_stats(
        monotonic_start_ns: Option<u64>,
        stats: &TaskStats,
        event: &SchedulerEvent,
        fault_deltas: (u64, u64),
    ) -> Self {
        Self {
            elapsed_ms: elapsed_ms_from_monotonic(monotonic_start_ns, event.switch_ns),
            task: event.pid,
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
            target_pending_wakeups: event.target_pending_wakeups,
            major_faults: fault_deltas.0,
            minor_faults: fault_deltas.1,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct IrqEventRecord {
    #[serde(default)]
    pub elapsed_ms: Option<u128>,
    pub irq: u32,
    pub cpu: u32,
    pub enter_ns: u64,
    pub exit_ns: u64,
    pub duration_ns: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct GpuSample {
    pub elapsed_ms: u128,
    pub gpu_busy_percent: Option<u32>,
    pub vram_used_bytes: Option<u64>,
    pub vram_total_bytes: Option<u64>,
    pub vram_used_percent: Option<u32>,
    pub gpu_clock_mhz: Option<u32>,
    pub mem_clock_mhz: Option<u32>,
    pub temp_millidegrees: Option<u32>,
    pub power_microwatts: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct FrameEvent {
    pub elapsed_ms: u128,
    pub frametime_ms: f64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BlockIoRecord {
    pub elapsed_ms: u128,
    pub tid: u32,
    #[serde(default = "default_block_io_correlation_basis")]
    pub correlation_basis: String,
    pub dev: u32,
    pub nr_sector: u32,
    pub sector: u64,
    pub duration_ns: u64,
    pub timestamp_ns: u64,
    pub rwbs: String,
}

fn default_block_io_correlation_basis() -> String {
    "dev+sector".to_owned()
}

pub const SESSION_SCHEMA_VERSION: u32 = 16;

pub struct FinalizeRecordingInput<'a> {
    pub recording: &'a RecordingRun,
    pub config: &'a Config,
    pub stop_reason: &'a str,
    pub active_targets: &'a BTreeMap<u32, TaskInfo>,
    pub stats_by_task: &'a BTreeMap<u32, TaskStats>,
    pub interval_records: &'a [IntervalRecord],
    pub streamed_interval_record_count: Option<usize>,
    pub intervals_dropped: usize,
    pub tree_events: &'a [TreeEvent],
    pub spike_events: &'a [SpikeEvent],
    pub spike_events_truncated: bool,
    pub spike_events_dropped_count: usize,
    pub scx_events: &'a [ScxEvent],
    pub irq_events: &'a [IrqEventRecord],
    pub streamed_irq_event_count: Option<usize>,
    pub migration_event_count: Option<usize>,
    pub cpu_freq_sample_count: Option<usize>,
    pub gpu_samples: &'a [GpuSample],
    pub streamed_gpu_sample_count: Option<usize>,
    pub frame_events: &'a [FrameEvent],
    pub block_io_event_count: usize,
    pub block_io_correlation_basis: &'a str,
    pub drop_counters: DropCountersSnapshot,
}

pub fn prepare_recording(config: &Config) -> anyhow::Result<Option<RecordingRun>> {
    let Some(recording) = &config.recording else {
        return Ok(None);
    };

    let started_at = SystemTime::now();
    let run_dir = resolve_run_dir(recording, started_at, env::var_os("HOME"));
    if let Err(err) = ensure_empty_dir(&run_dir) {
        let io_err = io::Error::other(err);
        return Err(anyhow::Error::new(StutterError::RecordWrite(io_err)));
    }

    Ok(Some(RecordingRun {
        run_name: recording.run_name.clone(),
        run_dir,
        started_at,
        started_instant: Instant::now(),
        monotonic_start_ns: monotonic_now_ns(),
    }))
}

pub fn finalize_recording(input: FinalizeRecordingInput<'_>) -> anyhow::Result<()> {
    let recording = input.recording;
    let config = input.config;
    let stop_reason = input.stop_reason;
    let active_targets = input.active_targets;
    let stats_by_task = input.stats_by_task;
    let interval_records = input.interval_records;
    let interval_record_count = input
        .streamed_interval_record_count
        .unwrap_or(interval_records.len());
    let tree_events = input.tree_events;
    let spike_events = input.spike_events;
    let scx_events = input.scx_events;
    let irq_events = input.irq_events;
    let irq_event_count = input.streamed_irq_event_count.unwrap_or(irq_events.len());
    let gpu_samples = input.gpu_samples;
    let gpu_sample_count = input.streamed_gpu_sample_count.unwrap_or(gpu_samples.len());
    let frame_events = input.frame_events;
    let drop_counters = input.drop_counters;
    let ended_at = SystemTime::now();
    let monotonic_end_ns = monotonic_now_ns();
    let duration_ms = recording.started_instant.elapsed().as_millis();
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
            sched_policy: stats
                .sched_policy
                .map(crate::process_tree::sched_policy_name)
                .map(|s| s.to_owned()),
            stat_wait_sum_ns: if stats.stat_wait_count > 0 {
                Some(stats.stat_wait_sum_ns as u64)
            } else {
                None
            },
            stat_wait_count: if stats.stat_wait_count > 0 {
                Some(stats.stat_wait_count)
            } else {
                None
            },
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
                target_pending_wakeups: spike.target_pending_wakeups,
                major_faults: spike.major_faults,
                minor_faults: spike.minor_faults,
            });
        }
    }

    tasks.sort_by_key(|task| std::cmp::Reverse(task.latency.max_ns));
    top_spikes.sort_by_key(|spike| std::cmp::Reverse(spike.latency_ns));
    top_spikes.truncate(64);

    let session = SessionFile {
        schema_version: SESSION_SCHEMA_VERSION,
        run_name: recording.run_name.clone(),
        started_at: recorded_time(recording.started_at),
        ended_at: recorded_time(ended_at),
        monotonic_start_ns: recording.monotonic_start_ns,
        monotonic_end_ns,
        duration_ms,
        stop_reason: stop_reason.to_owned(),
        config: RecordedConfig {
            manual_pids: config.target_pids.clone(),
            tree_roots: config.tree_pids.clone(),
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
            watch_timeout_ms: config.watch_timeout.map(|timeout| timeout.as_millis()),
            csv_path: config.csv_path.clone(),
            irq_latency: config.irq_latency,
            irqs: config.irqs.clone(),
            hwmon: config.hwmon,
            hwmon_root: config.hwmon_root.clone(),
            hwmon_drm_card: config.hwmon_drm_card.clone(),
            hwmon_render_node: config.hwmon_render_node.clone(),
            mangohud_log: config.mangohud_log.clone(),
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
            block_io: config.block_io,
            stat_wait: config.stat_wait,
        },
        metadata: metadata.clone(),
        target_pids_max: TARGET_PIDS_MAX,
        active_target_pids_count: active_targets.len(),
        active_expanded_tasks: active_expanded_tasks.clone(),
        interval_record_count,
        intervals_dropped: input.intervals_dropped,
        spike_events_retained_count: spike_events.len(),
        spike_events_dropped_count: input.spike_events_dropped_count,
        spike_events_truncated: input.spike_events_truncated,
        scx_event_count: scx_events.len(),
        irq_event_count,
        migration_event_count: input.migration_event_count,
        cpu_freq_sample_count: input.cpu_freq_sample_count,
        gpu_sample_count,
        frame_event_count: frame_events.len(),
        block_io_event_count: input.block_io_event_count,
        block_io_correlation_basis: input.block_io_correlation_basis.to_owned(),
        drop_counters: drop_counters.clone(),
        tasks,
        top_spikes,
    };

    let metadata_file = MetadataFile {
        schema_version: SESSION_SCHEMA_VERSION,
        run_name: recording.run_name.clone(),
        started_at: recorded_time(recording.started_at),
        ended_at: recorded_time(ended_at),
        monotonic_start_ns: recording.monotonic_start_ns,
        monotonic_end_ns,
        duration_ms,
        metadata,
        target_pids_max: TARGET_PIDS_MAX,
        active_target_pids_count: active_targets.len(),
        active_expanded_tasks,
        interval_record_count,
        intervals_dropped: input.intervals_dropped,
        spike_events_retained_count: spike_events.len(),
        spike_events_dropped_count: input.spike_events_dropped_count,
        spike_events_truncated: input.spike_events_truncated,
        scx_event_count: scx_events.len(),
        irq_event_count,
        migration_event_count: input.migration_event_count,
        cpu_freq_sample_count: input.cpu_freq_sample_count,
        gpu_sample_count,
        frame_event_count: frame_events.len(),
        block_io_event_count: input.block_io_event_count,
        block_io_correlation_basis: input.block_io_correlation_basis.to_owned(),
        drop_counters,
    };

    // Map any write errors to `StutterError::RecordWrite` so callers can decide
    // whether a failed recording should be treated as fatal.
    let map_write_err = |e: anyhow::Error| -> anyhow::Error {
        let io_err = io::Error::other(e.to_string());
        anyhow::Error::new(StutterError::RecordWrite(io_err))
    };

    write_json(recording.run_dir.join("session.json"), &session).map_err(map_write_err)?;
    write_json(recording.run_dir.join("metadata.json"), &metadata_file).map_err(map_write_err)?;
    if input.streamed_interval_record_count.is_none() {
        write_json_stream(recording.run_dir.join("interval.json"), interval_records)
            .map_err(map_write_err)?;
    }
    write_json_stream(recording.run_dir.join("tree_events.json"), tree_events)
        .map_err(map_write_err)?;
    write_json_stream(recording.run_dir.join("spike_events.json"), spike_events)
        .map_err(map_write_err)?;
    write_json_stream(recording.run_dir.join("scx_events.json"), scx_events)
        .map_err(map_write_err)?;
    if input.streamed_irq_event_count.is_none() {
        write_json_stream(recording.run_dir.join("irq_events.json"), irq_events)
            .map_err(map_write_err)?;
    }
    if input.streamed_gpu_sample_count.is_none() {
        write_json_stream(recording.run_dir.join("gpu_samples.json"), gpu_samples)
            .map_err(map_write_err)?;
    }
    write_json_stream(
        recording.run_dir.join("frame_correlation.json"),
        frame_events,
    )
    .map_err(map_write_err)?;

    println!("recording written to {}", recording.run_dir.display());
    Ok(())
}

#[cfg(test)]
pub fn write_interval_csv(
    path: &std::path::Path,
    interval_records: &[IntervalRecord],
) -> anyhow::Result<()> {
    let mut writer = IntervalCsvWriter::create(path.to_path_buf())?;

    for record in interval_records {
        writer.push(record)?;
    }

    writer.finish()
}

fn write_interval_csv_header(file: &mut fs::File) -> io::Result<()> {
    writeln!(
        file,
        "elapsed_ms,task,active,class,comm,process_pid,process_comm,samples,stored_samples,truncated_samples,min_ns,avg_ns,p95_ns,p99_ns,max_ns,over_1ms,over_2ms,over_5ms,busiest_cpu,busiest_cpu_samples,worst_cpu,worst_cpu_max_ns,spikiest_cpu,spikiest_cpu_spikes,percentile_scope,major_faults,minor_faults,cpu_psi_some,mem_psi_some,mem_psi_full,io_psi_some,io_psi_full,cumulative_drop_counters_total"
    )
}

fn write_interval_csv_row(file: &mut fs::File, record: &IntervalRecord) -> io::Result<()> {
    writeln!(
        file,
        "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
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
        prio: spike.prio,
        latency_ns: spike.latency_ns,
        wakeup_ns: spike.wakeup_ns,
        switch_ns: spike.switch_ns,
        target_pending_wakeups: spike.target_pending_wakeups,
        major_faults: spike.major_faults,
        minor_faults: spike.minor_faults,
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

fn recorded_time(time: SystemTime) -> RecordedTime {
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

fn monotonic_now_ns() -> Option<u64> {
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

fn elapsed_ms_from_monotonic(monotonic_start_ns: Option<u64>, switch_ns: u64) -> Option<u128> {
    let start_ns = monotonic_start_ns?;
    switch_ns
        .checked_sub(start_ns)
        .map(|elapsed_ns| u128::from(elapsed_ns / 1_000_000))
}

fn option_u32(value: Option<u32>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') || value.contains('\r') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

fn write_json<T: ?Sized + Serialize>(path: PathBuf, value: &T) -> anyhow::Result<()> {
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

    if let Some(parent) = path.parent()
        && let Ok(dir) = fs::File::open(parent)
    {
        let _ = dir.sync_all();
    }

    Ok(())
}

fn write_json_stream<T: Serialize>(path: PathBuf, values: &[T]) -> anyhow::Result<()> {
    let mut writer = JsonArrayWriter::create(path.clone())?;
    for value in values {
        writer.push(value)?;
    }
    writer.finish()?;

    if let Some(parent) = path.parent()
        && let Ok(dir) = fs::File::open(parent)
    {
        let _ = dir.sync_all();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_array_writer_outputs_valid_concatenated_json() {
        let dir = temp_dir("json-array-writer");
        fs::create_dir_all(&dir).unwrap();
        let empty_path = dir.join("empty.json");
        {
            let mut writer = JsonArrayWriter::create(empty_path.clone()).unwrap();
            writer.finish().unwrap();
        }
        assert!(fs::read_to_string(&empty_path).unwrap().is_empty());

        let single_path = dir.join("single.json");
        {
            let mut writer = JsonArrayWriter::create(single_path.clone()).unwrap();
            writer.push(&serde_json::json!({"one": true})).unwrap();
            writer.finish().unwrap();
        }
        let single: Vec<serde_json::Value> =
            serde_json::Deserializer::from_reader(fs::File::open(&single_path).unwrap())
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        assert_eq!(single.len(), 1);

        let path = dir.join("items.json");

        {
            let mut writer = JsonArrayWriter::create(path.clone()).unwrap();
            writer.push(&serde_json::json!({"a": 1})).unwrap();
            writer.push(&serde_json::json!({"b": 2})).unwrap();
            writer.finish().unwrap();
        }

        let values: Vec<serde_json::Value> =
            serde_json::Deserializer::from_reader(fs::File::open(&path).unwrap())
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
        assert_eq!(values.len(), 2);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn json_array_writer_rejects_path_without_file_name() {
        let err = JsonArrayWriter::create(PathBuf::from("/")).unwrap_err();
        assert!(err.to_string().contains("no file name"));
    }

    #[test]
    fn write_json_rejects_path_without_file_name() {
        let err = write_json(PathBuf::from("/"), &serde_json::json!({})).unwrap_err();
        assert!(err.to_string().contains("no file name"));
    }

    #[test]
    fn interval_csv_writer_streams_header_and_rows() {
        let dir = temp_dir("interval-csv-writer");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("interval.csv");

        {
            let mut writer = IntervalCsvWriter::create(path.clone()).unwrap();
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
        }
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
}
