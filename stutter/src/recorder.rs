use std::{
    collections::BTreeMap,
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use crate::error::StutterError;
use std::io;
use serde::{Deserialize, Serialize};
use stutter_common::SchedulerEvent;

use crate::{
    TARGET_PIDS_MAX,
    cli::{Config, RecordingConfig},
    ebpf_loader::DropCountersSnapshot,
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

pub struct SpikeEventBuffer {
    events: Vec<SpikeEvent>,
    truncated: bool,
    max_events: usize,
}

impl Default for SpikeEventBuffer {
    fn default() -> Self {
        Self {
            events: Vec::new(),
            truncated: false,
            max_events: MAX_SPIKE_EVENTS,
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
        }
    }

    pub fn push(&mut self, event: SpikeEvent) {
        if self.events.len() < self.max_events {
            self.events.push(event);
        } else {
            self.truncated = true;
        }
    }

    pub fn as_slice(&self) -> &[SpikeEvent] {
        &self.events
    }

    pub fn truncated(&self) -> bool {
        self.truncated
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
    pub spike_event_count: usize,
    #[serde(default)]
    pub spike_events_truncated: bool,
    #[serde(default)]
    pub scx_event_count: usize,
    #[serde(default)]
    pub irq_event_count: usize,
    #[serde(default)]
    pub gpu_sample_count: usize,
    #[serde(default)]
    pub frame_event_count: usize,
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
    pub spike_event_count: usize,
    #[serde(default)]
    pub spike_events_truncated: bool,
    #[serde(default)]
    pub scx_event_count: usize,
    #[serde(default)]
    pub irq_event_count: usize,
    #[serde(default)]
    pub gpu_sample_count: usize,
    #[serde(default)]
    pub frame_event_count: usize,
    #[serde(default)]
    pub drop_counters: DropCountersSnapshot,
}

#[derive(Serialize, Deserialize)]
pub struct RecordedConfig {
    pub manual_pids: Vec<u32>,
    pub tree_roots: Vec<u32>,
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
    pub mangohud_log: Option<PathBuf>,
    #[serde(default)]
    pub tui: bool,
    pub summary_period_ms: u64,
    pub spike_threshold_ns: u64,
    pub verbose: bool,
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
    pub comm: String,
    pub latency: RecordedLatency,
    pub cpu: RecordedCpuSnapshot,
    pub top_spikes: Vec<RecordedSpike>,
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
    pub prio: i32,
    pub latency_ns: u64,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
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
    pub prio: i32,
    pub latency_ns: u64,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
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
    pub prio: i32,
    pub latency_ns: u64,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
}

impl SpikeEvent {
    pub fn from_task_stats(
        monotonic_start_ns: Option<u64>,
        stats: &TaskStats,
        event: &SchedulerEvent,
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
            prio: event.prio,
            latency_ns: event.latency_ns,
            wakeup_ns: event.wakeup_ns,
            switch_ns: event.switch_ns,
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

pub const SESSION_SCHEMA_VERSION: u32 = 13;

pub struct FinalizeRecordingInput<'a> {
    pub recording: &'a RecordingRun,
    pub config: &'a Config,
    pub stop_reason: &'a str,
    pub active_targets: &'a BTreeMap<u32, TaskInfo>,
    pub stats_by_task: &'a BTreeMap<u32, TaskStats>,
    pub interval_records: &'a [IntervalRecord],
    pub tree_events: &'a [TreeEvent],
    pub spike_events: &'a [SpikeEvent],
    pub spike_events_truncated: bool,
    pub scx_events: &'a [ScxEvent],
    pub irq_events: &'a [IrqEventRecord],
    pub gpu_samples: &'a [GpuSample],
    pub frame_events: &'a [FrameEvent],
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
    let tree_events = input.tree_events;
    let spike_events = input.spike_events;
    let spike_events_truncated = input.spike_events_truncated;
    let scx_events = input.scx_events;
    let irq_events = input.irq_events;
    let gpu_samples = input.gpu_samples;
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
            comm: stats.comm.clone(),
            latency: recorded_latency(latency),
            cpu: recorded_cpu(cpu),
            top_spikes: stats
                .top_spikes
                .iter()
                .map(|spike| recorded_spike(stats, spike))
                .collect(),
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
                prio: spike.prio,
                latency_ns: spike.latency_ns,
                wakeup_ns: spike.wakeup_ns,
                switch_ns: spike.switch_ns,
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
            include_comm: config.task_filters.include_comm.clone(),
            exclude_comm: config.task_filters.exclude_comm.clone(),
            watch_process: config.watch_process.clone(),
            persistent: config.persistent,
            keep_missing_pid: config.keep_missing_pid,
            watch_poll_ms: config.watch_poll_ms,
            watch_timeout_ms: config.watch_timeout.map(|timeout| timeout.as_millis()),
            csv_path: config.csv_path.clone(),
            irq_latency: config.irq_latency,
            irqs: config.irqs.clone(),
            hwmon: config.hwmon,
            mangohud_log: config.mangohud_log.clone(),
            tui: config.tui,
            summary_period_ms: config.summary_period_ms,
            spike_threshold_ns: config.spike_threshold_ns,
            verbose: config.verbose,
        },
        metadata: metadata.clone(),
        target_pids_max: TARGET_PIDS_MAX,
        active_target_pids_count: active_targets.len(),
        active_expanded_tasks: active_expanded_tasks.clone(),
        spike_event_count: spike_events.len(),
        spike_events_truncated,
        scx_event_count: scx_events.len(),
        irq_event_count: irq_events.len(),
        gpu_sample_count: gpu_samples.len(),
        frame_event_count: frame_events.len(),
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
        spike_event_count: spike_events.len(),
        spike_events_truncated,
        scx_event_count: scx_events.len(),
        irq_event_count: irq_events.len(),
        gpu_sample_count: gpu_samples.len(),
        frame_event_count: frame_events.len(),
        drop_counters,
    };

    // Map any write errors to `StutterError::RecordWrite` so callers can decide
    // whether a failed recording should be treated as fatal.
    let map_write_err = |e: anyhow::Error| -> anyhow::Error {
        let io_err = io::Error::other(e.to_string());
        anyhow::Error::new(StutterError::RecordWrite(io_err))
    };

    write_json(recording.run_dir.join("session.json"), &session)
        .map_err(map_write_err)?;
    write_json(recording.run_dir.join("metadata.json"), &metadata_file)
        .map_err(map_write_err)?;
    write_json(recording.run_dir.join("interval.json"), interval_records)
        .map_err(map_write_err)?;
    write_json(recording.run_dir.join("tree_events.json"), tree_events)
        .map_err(map_write_err)?;
    write_json(recording.run_dir.join("spike_events.json"), spike_events)
        .map_err(map_write_err)?;
    write_json(recording.run_dir.join("scx_events.json"), scx_events)
        .map_err(map_write_err)?;
    write_json(recording.run_dir.join("irq_events.json"), irq_events)
        .map_err(map_write_err)?;
    write_json(recording.run_dir.join("gpu_samples.json"), gpu_samples)
        .map_err(map_write_err)?;
    write_json(
        recording.run_dir.join("frame_correlation.json"),
        frame_events,
    )
    .map_err(map_write_err)?;

    println!("recording written to {}", recording.run_dir.display());
    Ok(())
}

pub fn write_interval_csv(path: &Path, interval_records: &[IntervalRecord]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = fs::File::create(path)?;
    writeln!(
        file,
        "elapsed_ms,task,active,class,comm,process_pid,process_comm,samples,stored_samples,truncated_samples,min_ns,avg_ns,p95_ns,p99_ns,max_ns,over_1ms,over_2ms,over_5ms,busiest_cpu,busiest_cpu_samples,worst_cpu,worst_cpu_max_ns,spikiest_cpu,spikiest_cpu_spikes,percentile_scope"
    )?;

    for record in interval_records {
        writeln!(
            file,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
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
        )?;
    }

    Ok(())
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
        prio: spike.prio,
        latency_ns: spike.latency_ns,
        wakeup_ns: spike.wakeup_ns,
        switch_ns: spike.switch_ns,
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

fn ensure_empty_dir(path: &PathBuf) -> anyhow::Result<()> {
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
    let mut timespec = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };

    // SAFETY: clock_gettime writes to the provided valid timespec pointer and
    // does not retain it after the call. CLOCK_MONOTONIC intentionally matches
    // bpf_ktime_get_ns(), so recorded elapsed times line up with eBPF timestamps.
    let result = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut timespec) };
    if result != 0 {
        return None;
    }

    timespec_to_ns(timespec)
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
    let tmp_path = path.with_file_name(format!("{}.tmp", path.file_name().unwrap_or_default().to_string_lossy()));
    let mut file = fs::File::create(&tmp_path)
        .with_context(|| format!("failed to create temp JSON {}", tmp_path.display()))?;
    file.write_all(&serde_json::to_vec_pretty(value)?)
        .with_context(|| format!("failed to write temp JSON {}", tmp_path.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("failed to finish temp JSON {}", tmp_path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to sync temp JSON {}", tmp_path.display()))?;
    fs::rename(&tmp_path, &path).with_context(|| {
        format!(
            "failed to atomically rename {} to {}",
            tmp_path.display(),
            path.display()
        )
    })?;
    Ok(())
}
