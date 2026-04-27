use std::{
    collections::BTreeMap,
    env, fs,
    io::Write,
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{
    TARGET_PIDS_MAX,
    cli::{Config, RecordingConfig},
    metadata::{SystemMetadata, collect_system_metadata},
    metrics::{
        CpuLine, CpuSnapshot, IntervalRecord as MetricsIntervalRecord, MAX_EXACT_SAMPLES,
        SpikeRecord, TaskStats,
    },
    process_tree::{TaskClass, TaskInfo},
};

pub type IntervalRecord = MetricsIntervalRecord;

pub struct RecordingRun {
    pub run_name: Option<String>,
    pub run_dir: PathBuf,
    pub started_at: SystemTime,
    pub started_instant: Instant,
    pub monotonic_start_ns: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TreeEvent {
    pub elapsed_ms: u128,
    pub action: String,
    pub tid: u32,
    pub process_pid: u32,
    pub process_ppid: u32,
    pub comm: String,
    pub process_comm: String,
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
}

#[derive(Serialize, Deserialize)]
pub struct RecordedConfig {
    pub manual_pids: Vec<u32>,
    pub tree_roots: Vec<u32>,
    pub summary_period_ms: u64,
    pub spike_threshold_ns: u64,
    pub verbose: bool,
}

#[derive(Serialize, Deserialize)]
pub struct RecordedTime {
    pub unix_seconds: u64,
    pub unix_nanos: u32,
    pub local: String,
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
    pub process_comm: String,
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
    pub process_comm: String,
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
    pub process_comm: String,
    pub comm: String,
    pub cpu: u32,
    pub prio: i32,
    pub latency_ns: u64,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
}

pub const SESSION_SCHEMA_VERSION: u32 = 4;

pub struct FinalizeRecordingInput<'a> {
    pub recording: &'a RecordingRun,
    pub config: &'a Config,
    pub stop_reason: &'a str,
    pub active_targets: &'a BTreeMap<u32, TaskInfo>,
    pub stats_by_task: &'a BTreeMap<u32, TaskStats>,
    pub interval_records: &'a [IntervalRecord],
    pub tree_events: &'a [TreeEvent],
}

pub fn prepare_recording(config: &Config) -> anyhow::Result<Option<RecordingRun>> {
    let Some(recording) = &config.recording else {
        return Ok(None);
    };

    let started_at = SystemTime::now();
    let run_dir = resolve_run_dir(recording, started_at, env::var_os("HOME"));
    ensure_empty_dir(&run_dir)?;

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
    let ended_at = SystemTime::now();
    let duration_ms = recording.started_instant.elapsed().as_millis();
    let metadata = collect_system_metadata();

    let mut active_expanded_tasks = active_targets.keys().copied().collect::<Vec<_>>();
    active_expanded_tasks.sort_unstable();

    let mut tasks = Vec::new();
    let mut top_spikes = Vec::new();

    for (task, stats) in stats_by_task {
        let mut cloned_latency = clone_latency_stats(stats);
        let Some(latency) = cloned_latency.session_latency.snapshot() else {
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
        monotonic_end_ns: monotonic_now_ns(),
        duration_ms,
        stop_reason: stop_reason.to_owned(),
        config: RecordedConfig {
            manual_pids: config.target_pids.clone(),
            tree_roots: config.tree_pids.clone(),
            summary_period_ms: config.summary_period_ms,
            spike_threshold_ns: config.spike_threshold_ns,
            verbose: config.verbose,
        },
        metadata: metadata.clone(),
        target_pids_max: TARGET_PIDS_MAX,
        active_target_pids_count: active_targets.len(),
        active_expanded_tasks,
        tasks,
        top_spikes,
    };

    let metadata_file = MetadataFile {
        schema_version: SESSION_SCHEMA_VERSION,
        run_name: recording.run_name.clone(),
        started_at: recorded_time(recording.started_at),
        ended_at: recorded_time(ended_at),
        monotonic_start_ns: recording.monotonic_start_ns,
        monotonic_end_ns: monotonic_now_ns(),
        duration_ms,
        metadata,
        target_pids_max: TARGET_PIDS_MAX,
        active_target_pids_count: active_targets.len(),
        active_expanded_tasks: active_targets.keys().copied().collect(),
    };

    write_json(recording.run_dir.join("session.json"), &session)?;
    write_json(recording.run_dir.join("metadata.json"), &metadata_file)?;
    write_json(recording.run_dir.join("interval.json"), interval_records)?;
    write_json(recording.run_dir.join("tree_events.json"), tree_events)?;

    println!("recording written to {}", recording.run_dir.display());
    Ok(())
}

fn clone_latency_stats(stats: &TaskStats) -> TaskStats {
    TaskStats {
        task: stats.task,
        comm: stats.comm.clone(),
        class: stats.class,
        process_pid: stats.process_pid,
        process_comm: stats.process_comm.clone(),
        active: stats.active,
        first_seen_ms: stats.first_seen_ms,
        last_seen_ms: stats.last_seen_ms,
        removed_ms: stats.removed_ms,
        interval_latency: crate::metrics::LatencyStats::new(),
        interval_cpu: crate::metrics::CpuStatsSet::new(),
        session_latency: crate::metrics::LatencyStats {
            count: stats.session_latency.count,
            min_ns: stats.session_latency.min_ns,
            max_ns: stats.session_latency.max_ns,
            sum_ns: stats.session_latency.sum_ns,
            over_1ms: stats.session_latency.over_1ms,
            over_2ms: stats.session_latency.over_2ms,
            over_5ms: stats.session_latency.over_5ms,
            samples_ns: stats.session_latency.samples_ns.clone(),
            samples_truncated: stats.session_latency.samples_truncated,
        },
        session_cpu: crate::metrics::CpuStatsSet {
            by_cpu: stats.session_cpu.by_cpu.clone(),
        },
        top_spikes: stats.top_spikes.clone(),
    }
}

fn recorded_latency(latency: crate::metrics::LatencySnapshot) -> RecordedLatency {
    let stored_samples = latency.count.min(MAX_EXACT_SAMPLES as u64);
    let percentile_scope = if latency.samples_truncated > 0 {
        "capped_prefix"
    } else {
        "complete"
    };

    RecordedLatency {
        samples: latency.count,
        stored_samples,
        truncated_samples: latency.samples_truncated,
        percentile_scope: percentile_scope.to_owned(),
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
        local: format!("{time:?}"),
    }
}

fn timestamp_for_path(time: SystemTime) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    duration.as_secs().to_string()
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
    fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|value| value.split_whitespace().next().map(str::to_owned))
        .and_then(|seconds| seconds.parse::<f64>().ok())
        .map(|seconds| (seconds * 1_000_000_000.0) as u64)
}

fn write_json<T: ?Sized + Serialize>(path: PathBuf, value: &T) -> anyhow::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.write_all(b"\n")?;
    Ok(())
}
