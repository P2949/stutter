use std::{
    collections::BTreeMap,
    env, fs,
    io::Write,
    path::PathBuf,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use stutter_common::SchedulerEvent;

use crate::{
    TARGET_PIDS_MAX,
    cli::{Config, RecordingConfig},
    metadata::{SystemMetadata, collect_system_metadata},
    metrics::{
        CpuLine, CpuSnapshot, IntervalRecord as MetricsIntervalRecord, LatencyHistogramBucket,
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
    #[serde(default)]
    pub spike_event_count: usize,
    #[serde(default)]
    pub spike_events_truncated: bool,
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

#[derive(Clone, Serialize, Deserialize)]
pub struct SpikeEvent {
    #[serde(default)]
    pub elapsed_ms: Option<u128>,
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

pub const SESSION_SCHEMA_VERSION: u32 = 7;

pub struct FinalizeRecordingInput<'a> {
    pub recording: &'a RecordingRun,
    pub config: &'a Config,
    pub stop_reason: &'a str,
    pub active_targets: &'a BTreeMap<u32, TaskInfo>,
    pub stats_by_task: &'a BTreeMap<u32, TaskStats>,
    pub interval_records: &'a [IntervalRecord],
    pub tree_events: &'a [TreeEvent],
    pub spike_events: &'a [SpikeEvent],
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
    let spike_events = input.spike_events;
    let ended_at = SystemTime::now();
    let monotonic_end_ns = monotonic_now_ns();
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
        monotonic_end_ns,
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
        active_expanded_tasks: active_expanded_tasks.clone(),
        spike_event_count: spike_events.len(),
        spike_events_truncated: false,
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
        active_expanded_tasks: active_expanded_tasks.clone(),
        spike_event_count: spike_events.len(),
        spike_events_truncated: false,
    };

    write_json(recording.run_dir.join("session.json"), &session)?;
    write_json(recording.run_dir.join("metadata.json"), &metadata_file)?;
    write_json(recording.run_dir.join("interval.json"), interval_records)?;
    write_json(recording.run_dir.join("tree_events.json"), tree_events)?;
    write_json(recording.run_dir.join("spike_events.json"), spike_events)?;

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
            histogram: stats.session_latency.histogram.clone(),
        },
        session_cpu: crate::metrics::CpuStatsSet {
            by_cpu: stats.session_cpu.by_cpu.clone(),
        },
        top_spikes: stats.top_spikes.clone(),
    }
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
    let mut timespec = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };

    // SAFETY: clock_gettime writes to the provided valid timespec pointer and
    // does not retain it after the call.
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

fn write_json<T: ?Sized + Serialize>(path: PathBuf, value: &T) -> anyhow::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        fs,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::process_tree::TaskClass;

    #[test]
    fn metadata_uses_sorted_active_expanded_tasks() {
        let dir = temp_test_dir("metadata-sorted");
        fs::create_dir_all(&dir).unwrap();

        let recording = RecordingRun {
            run_name: Some("test".to_owned()),
            run_dir: dir.clone(),
            started_at: UNIX_EPOCH,
            started_instant: Instant::now(),
            monotonic_start_ns: Some(1_000),
        };
        let config = Config {
            target_pids: vec![9, 1, 4],
            tree_pids: vec![],
            summary_period_ms: 1_000,
            spike_threshold_ns: 1_000_000,
            verbose: false,
            recording: None,
            max_duration: Some(Duration::from_secs(1)),
        };
        let active_targets =
            BTreeMap::from([(9, task_info(9)), (1, task_info(1)), (4, task_info(4))]);
        let stats_by_task = BTreeMap::new();
        let interval_records = Vec::new();
        let tree_events = Vec::new();
        let spike_events = Vec::new();

        finalize_recording(FinalizeRecordingInput {
            recording: &recording,
            config: &config,
            stop_reason: "test",
            active_targets: &active_targets,
            stats_by_task: &stats_by_task,
            interval_records: &interval_records,
            tree_events: &tree_events,
            spike_events: &spike_events,
        })
        .unwrap();

        let session: SessionFile =
            serde_json::from_str(&fs::read_to_string(dir.join("session.json")).unwrap()).unwrap();
        let metadata: MetadataFile =
            serde_json::from_str(&fs::read_to_string(dir.join("metadata.json")).unwrap()).unwrap();

        assert_eq!(session.active_expanded_tasks, vec![1, 4, 9]);
        assert_eq!(metadata.active_expanded_tasks, vec![1, 4, 9]);
        assert_eq!(session.spike_event_count, 0);
        assert_eq!(metadata.spike_event_count, 0);
        assert!(!session.spike_events_truncated);
        assert!(!metadata.spike_events_truncated);

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn session_serializes_schema_seven_latency_histogram_and_spike_events() {
        let dir = temp_test_dir("latency-histogram");
        fs::create_dir_all(&dir).unwrap();

        let recording = RecordingRun {
            run_name: Some("histogram-test".to_owned()),
            run_dir: dir.clone(),
            started_at: UNIX_EPOCH,
            started_instant: Instant::now(),
            monotonic_start_ns: Some(1_000),
        };
        let config = Config {
            target_pids: vec![7],
            tree_pids: vec![],
            summary_period_ms: 1_000,
            spike_threshold_ns: 1_000_000,
            verbose: false,
            recording: None,
            max_duration: Some(Duration::from_secs(1)),
        };
        let active_targets = BTreeMap::from([(7, task_info(7))]);
        let mut stats = crate::metrics::TaskStats::new(7, "worker".to_owned(), 0);
        stats.session_latency.record(1_000);
        stats.session_latency.record(2_000_000);
        let stats_by_task = BTreeMap::from([(7, stats)]);
        let interval_records = Vec::new();
        let tree_events = Vec::new();
        let spike_events = vec![SpikeEvent {
            elapsed_ms: Some(12),
            task: 7,
            active: true,
            class: TaskClass::Helper,
            process_pid: Some(7),
            process_comm: "task-7".to_owned(),
            comm: "worker".to_owned(),
            cpu: 1,
            prio: 120,
            latency_ns: 2_000_000,
            wakeup_ns: 10,
            switch_ns: 2_000_010,
        }];

        finalize_recording(FinalizeRecordingInput {
            recording: &recording,
            config: &config,
            stop_reason: "test",
            active_targets: &active_targets,
            stats_by_task: &stats_by_task,
            interval_records: &interval_records,
            tree_events: &tree_events,
            spike_events: &spike_events,
        })
        .unwrap();

        let session: SessionFile =
            serde_json::from_str(&fs::read_to_string(dir.join("session.json")).unwrap()).unwrap();
        let metadata: MetadataFile =
            serde_json::from_str(&fs::read_to_string(dir.join("metadata.json")).unwrap()).unwrap();
        let recorded_spike_events: Vec<SpikeEvent> =
            serde_json::from_str(&fs::read_to_string(dir.join("spike_events.json")).unwrap())
                .unwrap();

        assert_eq!(session.schema_version, SESSION_SCHEMA_VERSION);
        assert_eq!(session.schema_version, 7);
        assert_eq!(session.monotonic_end_ns, metadata.monotonic_end_ns);
        assert_eq!(session.spike_event_count, 1);
        assert_eq!(metadata.spike_event_count, 1);
        assert!(!session.spike_events_truncated);
        assert!(!metadata.spike_events_truncated);
        assert_eq!(recorded_spike_events.len(), 1);
        assert_eq!(recorded_spike_events[0].task, 7);
        assert_eq!(recorded_spike_events[0].elapsed_ms, Some(12));
        assert_eq!(recorded_spike_events[0].latency_ns, 2_000_000);
        assert_eq!(session.tasks.len(), 1);

        let latency = &session.tasks[0].latency;
        assert_eq!(latency.percentile_scope, "exact");
        assert_eq!(latency.stored_samples, 2);
        assert_eq!(
            latency.histogram.len(),
            crate::metrics::LATENCY_HISTOGRAM_BUCKET_COUNT
        );
        assert_eq!(latency.histogram[0].count, 1);
        assert_eq!(
            latency
                .histogram
                .iter()
                .find(|bucket| bucket.upper_bound_ns == Some(2_000_000))
                .unwrap()
                .count,
            1
        );

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn timespec_conversion_rejects_invalid_values() {
        assert_eq!(
            timespec_to_ns(libc::timespec {
                tv_sec: 1,
                tv_nsec: 2_000,
            }),
            Some(1_000_002_000)
        );
        assert_eq!(
            timespec_to_ns(libc::timespec {
                tv_sec: -1,
                tv_nsec: 0,
            }),
            None
        );
        assert_eq!(
            timespec_to_ns(libc::timespec {
                tv_sec: 0,
                tv_nsec: -1,
            }),
            None
        );
        assert_eq!(
            timespec_to_ns(libc::timespec {
                tv_sec: 0,
                tv_nsec: 1_000_000_000,
            }),
            None
        );
        assert_eq!(
            timespec_to_ns(libc::timespec {
                tv_sec: (u64::MAX / 1_000_000_000 + 1) as libc::time_t,
                tv_nsec: 0,
            }),
            None
        );
    }

    #[test]
    fn spike_event_elapsed_uses_monotonic_switch_timestamp() {
        let mut stats = crate::metrics::TaskStats::new(7, "RenderThread".to_owned(), 0);
        stats.apply_task_info(&task_info(7));
        let event = scheduler_event(7, 1_500_000_000, 1_000_000);

        let spike = SpikeEvent::from_task_stats(Some(1_000_000_000), &stats, &event);
        assert_eq!(spike.elapsed_ms, Some(500));

        let spike = SpikeEvent::from_task_stats(None, &stats, &event);
        assert_eq!(spike.elapsed_ms, None);

        let spike = SpikeEvent::from_task_stats(Some(2_000_000_000), &stats, &event);
        assert_eq!(spike.elapsed_ms, None);
    }

    fn task_info(tid: u32) -> TaskInfo {
        TaskInfo {
            tid,
            process_pid: tid,
            process_ppid: 1,
            comm: format!("task-{tid}"),
            process_comm: format!("task-{tid}"),
            class: TaskClass::Helper,
        }
    }

    fn temp_test_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("stutter-{name}-{unique}"));
        path
    }

    fn scheduler_event(pid: u32, switch_ns: u64, latency_ns: u64) -> SchedulerEvent {
        SchedulerEvent {
            kind: stutter_common::EVENT_RUNNABLE_LATENCY,
            pid,
            cpu: 1,
            prio: 120,
            wakeup_ns: switch_ns.saturating_sub(latency_ns),
            switch_ns,
            latency_ns,
            comm: [0; 16],
        }
    }
}
