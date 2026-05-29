use std::{
    env, fs,
    path::{Path, PathBuf},
};

use crate::{
    diagnosis::{Confidence, Diagnosis, StutterCause},
    ebpf_loader::DropCountersSnapshot,
    metadata::SystemMetadata,
    process_tree::TaskClass,
    recorder::{
        BlockIoRecord, GpuSample, IntervalRecord, IrqEventRecord, MetadataFile, RecordedConfig,
        RecordedCpuSnapshot, RecordedLatency, RecordedTime, SESSION_SCHEMA_VERSION, SessionFile,
        SessionTask, SpikeEvent,
    },
    report::{self, DataQualityLevel, SpikeClusterAnalysis, SpikeClusterSource},
    session_io::RunArtifacts,
};

pub fn write_minimal_recording_fixture(dir: &Path) {
    let mk_time = || RecordedTime {
        unix_seconds: 1625097600,
        unix_nanos: 0,
        system_time_debug: "2021-07-01T00:00:00Z".to_owned(),
    };

    let mk_config = || RecordedConfig {
        manual_pids: vec![123],
        tree_roots: vec![],
        cgroupv2: None,
        exclude_tree_pids: vec![],
        include_comm: vec![],
        exclude_comm: vec![],
        watch_process: None,
        persistent: false,
        keep_missing_pid: false,
        watch_poll_ms: 100,
        watch_timeout_ms: None,
        csv_stream: None,
        irq_latency: false,
        irqs: vec![],
        hwmon: false,
        hwmon_root: None,
        hwmon_drm_card: None,
        hwmon_render_node: None,
        mangohud_log: None,
        mangohud_log_live: false,
        tui: false,
        summary_period_ms: 1000,
        epoch_period_ms: None,
        retain_intervals: None,
        max_tasks: 1024,
        spike_threshold_ns: 1_000_000,
        live_diagnosis_cluster_window_ms:
            crate::config::model::DEFAULT_LIVE_DIAGNOSIS_CLUSTER_WINDOW_MS,
        alert_threshold_ns: None,
        alert_webhook_url: None,
        follow_exec: true,
        verbose: false,
        faults: false,
        cpu_perf: false,
        cpu_perf_kernel: false,
        cpu_perf_max_tasks: 128,
        cpu_perf_cache_refs: false,
        block_io: false,
        stat_wait: false,
        otlp_endpoint: None,
        otel_service_name: "stutter".to_owned(),
        ..Default::default()
    };

    let task = SessionTask {
        task: 123,
        active: true,
        first_seen_ms: 0,
        last_seen_ms: 1000,
        removed_ms: None,
        class: TaskClass::Game,
        process_pid: Some(123),
        process_comm: "game".into(),
        process_starttime_ticks: None,
        task_starttime_ticks: None,
        exe_dev: None,
        exe_ino: None,
        comm: "game".to_owned(),
        latency: RecordedLatency {
            samples: 100,
            stored_samples: 100,
            truncated_samples: 0,
            percentile_scope: "exact".to_owned(),
            histogram: vec![],
            min_ns: 100,
            avg_ns: 500,
            p95_ns: 1000,
            p99_ns: 2000,
            max_ns: 5000,
            over_1ms: 0,
            over_2ms: 0,
            over_5ms: 0,
        },
        cpu: RecordedCpuSnapshot {
            busiest_cpu: Some(0),
            busiest_cpu_samples: 100,
            worst_cpu: Some(0),
            worst_cpu_max_ns: 5000,
            spikiest_cpu: Some(0),
            spikiest_cpu_spikes: 0,
            per_cpu: vec![],
        },
        top_spikes: vec![],
        migration_count: 0,
        cross_numa_migrations: 0,
        top_wakers: vec![],
        sched_policy: None,
        stat_wait_sum_ns: None,
        stat_wait_sum_ns_saturated: false,
        stat_wait_count: None,
        cpu_perf: None,
    };

    let session = SessionFile {
        core: crate::recorder::SessionMetadataCore {
            schema_version: SESSION_SCHEMA_VERSION,
            run_name: Some("minimal".to_owned()),
            started_at: mk_time(),
            ended_at: mk_time(),
            monotonic_start_ns: Some(0),
            monotonic_end_ns: Some(1_000_000_000),
            duration_ms: 1000,
            mangohud_start_offset: None,
            mangohud_first_frame_monotonic_ns: None,
            mangohud_first_frame_raw_elapsed_ms: None,
            metadata: SystemMetadata::default(),
            target_pids_max: 1024,
            active_target_pids_count: 1,
            active_expanded_tasks: vec![123],
            interval_record_count: 0,
            intervals_dropped: 0,
            spike_events_retained_count: 0,
            spike_events_dropped_count: 0,
            spike_events_truncated: false,
            scx_event_count: 0,
            irq_event_count: 0,
            migration_event_count: Some(0),
            cpu_freq_sample_count: Some(0),
            gpu_sample_count: 0,
            frame_event_count: 0,
            block_io_event_count: 0,
            event_stream_write_errors: 0,
            alert_events_dropped_count: 0,
            alert_channel_closed_count: 0,
            first_event_stream_write_error: None,
            block_io_correlation_basis: "dev+sector".to_owned(),
            drop_counters: DropCountersSnapshot::default(),
            cpu_perf_sample_count: 0,
            cpu_perf_open_errors: 0,
            cpu_perf_read_errors: 0,
            cpu_perf_skipped_tasks: 0,
            cpu_perf_last_error: None,
            ..Default::default()
        },
        stop_reason: "manual".to_owned(),
        config: mk_config(),
        tasks: vec![task],
        top_spikes: vec![],
    };

    let metadata_file = MetadataFile {
        core: crate::recorder::SessionMetadataCore {
            schema_version: SESSION_SCHEMA_VERSION,
            run_name: Some("minimal".to_owned()),
            started_at: mk_time(),
            ended_at: mk_time(),
            monotonic_start_ns: Some(0),
            monotonic_end_ns: Some(1_000_000_000),
            duration_ms: 1000,
            mangohud_start_offset: None,
            mangohud_first_frame_monotonic_ns: None,
            mangohud_first_frame_raw_elapsed_ms: None,
            metadata: SystemMetadata::default(),
            target_pids_max: 1024,
            active_target_pids_count: 1,
            active_expanded_tasks: vec![123],
            interval_record_count: 0,
            intervals_dropped: 0,
            spike_events_retained_count: 0,
            spike_events_dropped_count: 0,
            spike_events_truncated: false,
            scx_event_count: 0,
            irq_event_count: 0,
            migration_event_count: Some(0),
            cpu_freq_sample_count: Some(0),
            gpu_sample_count: 0,
            frame_event_count: 0,
            block_io_event_count: 0,
            event_stream_write_errors: 0,
            alert_events_dropped_count: 0,
            alert_channel_closed_count: 0,
            first_event_stream_write_error: None,
            block_io_correlation_basis: "dev+sector".to_owned(),
            drop_counters: DropCountersSnapshot::default(),
            cpu_perf_sample_count: 0,
            cpu_perf_open_errors: 0,
            cpu_perf_read_errors: 0,
            cpu_perf_skipped_tasks: 0,
            cpu_perf_last_error: None,
            ..Default::default()
        },
    };

    fs::create_dir_all(dir).unwrap();
    let session_file = fs::File::create(dir.join("session.json")).unwrap();
    serde_json::to_writer_pretty(session_file, &session).unwrap();

    let metadata_file_ptr = fs::File::create(dir.join("metadata.json")).unwrap();
    serde_json::to_writer_pretty(metadata_file_ptr, &metadata_file).unwrap();
}

const OPTIONAL_ARTIFACT_FILES: &[&str] = &[
    "spike_events.json",
    "interval.json",
    "tree_events.json",
    "irq_events.json",
    "gpu_samples.json",
    "frame_correlation.json",
    "migration_events.json",
    "cpu_freq_samples.json",
    "io_events.json",
    "scx_events.json",
];

fn temp_run_dir(name: &str) -> PathBuf {
    let mut temp = env::temp_dir();
    temp.push(format!(
        "stutter-test-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    temp
}

fn write_json_pretty<T: serde::Serialize>(path: impl AsRef<Path>, value: &T) {
    let file = fs::File::create(path).unwrap();
    serde_json::to_writer_pretty(file, value).unwrap();
}

fn write_ndjson<T: serde::Serialize>(path: impl AsRef<Path>, values: &[T]) {
    use std::io::Write;

    let mut file = fs::File::create(path).unwrap();
    for value in values {
        serde_json::to_writer(&mut file, value).unwrap();
        file.write_all(b"\n").unwrap();
    }
}

fn task_for_fixture(task: u32, class: TaskClass, comm: &str) -> SessionTask {
    SessionTask {
        task,
        active: true,
        first_seen_ms: 0,
        last_seen_ms: 1000,
        removed_ms: None,
        class,
        process_pid: Some(task),
        process_comm: comm.into(),
        process_starttime_ticks: None,
        task_starttime_ticks: None,
        exe_dev: None,
        exe_ino: None,
        comm: comm.to_owned(),
        latency: RecordedLatency {
            samples: 10,
            stored_samples: 10,
            truncated_samples: 0,
            percentile_scope: "exact".to_owned(),
            histogram: vec![],
            min_ns: 100,
            avg_ns: 500,
            p95_ns: 1_000,
            p99_ns: 2_000,
            max_ns: 5_000,
            over_1ms: 0,
            over_2ms: 0,
            over_5ms: 0,
        },
        cpu: RecordedCpuSnapshot::default(),
        top_spikes: vec![],
        migration_count: 0,
        cross_numa_migrations: 0,
        top_wakers: vec![],
        sched_policy: None,
        stat_wait_sum_ns: None,
        stat_wait_sum_ns_saturated: false,
        stat_wait_count: None,
        cpu_perf: None,
    }
}

fn base_session(run_name: &str) -> SessionFile {
    SessionFile {
        core: crate::recorder::SessionMetadataCore {
            schema_version: SESSION_SCHEMA_VERSION,
            run_name: Some(run_name.to_owned()),
            started_at: mk_dummy_time(),
            ended_at: mk_dummy_time(),
            monotonic_start_ns: Some(0),
            monotonic_end_ns: Some(1_000_000_000),
            duration_ms: 1000,
            mangohud_start_offset: None,
            mangohud_first_frame_monotonic_ns: None,
            mangohud_first_frame_raw_elapsed_ms: None,
            metadata: SystemMetadata::default(),
            target_pids_max: 1024,
            active_target_pids_count: 1,
            active_expanded_tasks: vec![100],
            interval_record_count: 1,
            intervals_dropped: 0,
            spike_events_retained_count: 0,
            spike_events_dropped_count: 0,
            spike_events_truncated: false,
            scx_event_count: 0,
            irq_event_count: 0,
            migration_event_count: Some(0),
            cpu_freq_sample_count: Some(0),
            gpu_sample_count: 0,
            frame_event_count: 0,
            block_io_event_count: 0,
            event_stream_write_errors: 0,
            alert_events_dropped_count: 0,
            alert_channel_closed_count: 0,
            first_event_stream_write_error: None,
            block_io_correlation_basis: "dev+sector".to_owned(),
            drop_counters: DropCountersSnapshot::default(),
            cpu_perf_sample_count: 0,
            cpu_perf_open_errors: 0,
            cpu_perf_read_errors: 0,
            cpu_perf_skipped_tasks: 0,
            cpu_perf_last_error: None,
            ..Default::default()
        },
        stop_reason: "test".to_owned(),
        config: mk_dummy_config(),
        tasks: vec![task_for_fixture(100, TaskClass::Unknown, "worker")],
        top_spikes: vec![],
    }
}

fn base_metadata_from_session(session: &SessionFile) -> MetadataFile {
    MetadataFile {
        core: session.core.clone(),
    }
}

fn write_base_run(
    dir: &Path,
    run_name: &str,
    session_mutator: impl FnOnce(&mut SessionFile),
) -> SessionFile {
    fs::create_dir_all(dir).unwrap();
    let mut session = base_session(run_name);
    session_mutator(&mut session);
    let metadata = base_metadata_from_session(&session);

    write_json_pretty(dir.join("session.json"), &session);
    write_json_pretty(dir.join("metadata.json"), &metadata);
    for file in OPTIONAL_ARTIFACT_FILES {
        write_ndjson::<serde_json::Value>(dir.join(file), &[]);
    }

    session
}

fn spike_event(
    task: u32,
    class: TaskClass,
    comm: &str,
    latency_ns: u64,
    offset_ns: u64,
) -> SpikeEvent {
    let switch_ns = 100_000_000 + offset_ns;
    SpikeEvent {
        elapsed_ms: Some(100),
        task: task.into(),
        active: true,
        class,
        process_pid: Some(task.into()),
        process_comm: comm.into(),
        comm: comm.to_owned(),
        cpu: 0,
        wakeup_target_cpu: 0,
        prio: 120,
        latency_ns,
        wakeup_ns: switch_ns.saturating_sub(latency_ns),
        switch_ns,
        ..Default::default()
    }
}

fn clustered_spikes(
    anchor_class: TaskClass,
    anchor_comm: &str,
    anchor_latency_ns: u64,
) -> Vec<SpikeEvent> {
    vec![
        spike_event(100, anchor_class, anchor_comm, anchor_latency_ns, 0),
        spike_event(101, TaskClass::Unknown, "helper-a", 1_500_000, 250_000),
        spike_event(102, TaskClass::Unknown, "helper-b", 1_250_000, 500_000),
    ]
}

fn apply_spike_session_fields(session: &mut SessionFile, spikes: &[SpikeEvent]) {
    session.core.spike_events_retained_count = spikes.len() as u64;
    session.core.active_target_pids_count = spikes.len() as u64;
    session.core.active_expanded_tasks = spikes.iter().map(|spike| spike.task.as_u32()).collect();
    session.tasks = spikes
        .iter()
        .map(|spike| task_for_fixture(spike.task.as_u32(), spike.class, &spike.comm))
        .collect();
}

fn candidate_contains(diagnosis: &Diagnosis, cause: StutterCause) -> bool {
    diagnosis
        .candidates
        .iter()
        .any(|candidate| candidate.cause == cause)
}
// JSON report output is currently only exposed through the CLI printing path,
// so unit tests avoid brittle stdout capture.
fn mk_dummy_time() -> RecordedTime {
    RecordedTime {
        unix_seconds: 1625097600,
        unix_nanos: 0,
        system_time_debug: "2021-07-01T00:00:00Z".to_owned(),
    }
}

fn mk_dummy_config() -> RecordedConfig {
    RecordedConfig {
        manual_pids: vec![],
        tree_roots: vec![],
        cgroupv2: None,
        exclude_tree_pids: vec![],
        include_comm: vec![],
        exclude_comm: vec![],
        watch_process: None,
        persistent: false,
        keep_missing_pid: false,
        watch_poll_ms: 100,
        watch_timeout_ms: None,
        csv_stream: None,
        irq_latency: false,
        irqs: vec![],
        hwmon: false,
        hwmon_root: None,
        hwmon_drm_card: None,
        hwmon_render_node: None,
        mangohud_log: None,
        mangohud_log_live: false,
        tui: false,
        summary_period_ms: 1000,
        epoch_period_ms: None,
        retain_intervals: None,
        max_tasks: 1024,
        spike_threshold_ns: 1_000_000,
        live_diagnosis_cluster_window_ms:
            crate::config::model::DEFAULT_LIVE_DIAGNOSIS_CLUSTER_WINDOW_MS,
        alert_threshold_ns: None,
        alert_webhook_url: None,
        follow_exec: true,
        verbose: false,
        faults: false,
        cpu_perf: false,
        cpu_perf_kernel: false,
        cpu_perf_max_tasks: 128,
        cpu_perf_cache_refs: false,
        block_io: false,
        stat_wait: false,
        otlp_endpoint: None,
        otel_service_name: "stutter".to_owned(),
        ..Default::default()
    }
}

mod existence;

mod report_text;

mod schema_roundtrip;

mod replay_fixtures;

mod regression_detection;
