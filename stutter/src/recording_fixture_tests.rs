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
        csv_path: None,
        irq_latency: false,
        irqs: vec![],
        hwmon: false,
        hwmon_root: None,
        hwmon_drm_card: None,
        hwmon_render_node: None,
        mangohud_log: None,
        tui: false,
        summary_period_ms: 1000,
        epoch_period_ms: None,
        retain_intervals: None,
        max_tasks: 1024,
        spike_threshold_ns: 1_000_000,
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
        stat_wait_count: None,
        cpu_perf: None,
    };

    let session = SessionFile {
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
        stop_reason: "manual".to_owned(),
        config: mk_config(),
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
        tasks: vec![task],
        top_spikes: vec![],
    };

    let metadata_file = MetadataFile {
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
        stat_wait_count: None,
        cpu_perf: None,
    }
}

fn base_session(run_name: &str) -> SessionFile {
    SessionFile {
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
        stop_reason: "test".to_owned(),
        config: mk_dummy_config(),
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
        tasks: vec![task_for_fixture(100, TaskClass::Unknown, "worker")],
        top_spikes: vec![],
    }
}

fn base_metadata_from_session(session: &SessionFile) -> MetadataFile {
    MetadataFile {
        schema_version: session.schema_version,
        run_name: session.run_name.clone(),
        started_at: session.started_at.clone(),
        ended_at: session.ended_at.clone(),
        monotonic_start_ns: session.monotonic_start_ns,
        monotonic_end_ns: session.monotonic_end_ns,
        duration_ms: session.duration_ms,
        mangohud_start_offset: session.mangohud_start_offset,
        mangohud_first_frame_monotonic_ns: session.mangohud_first_frame_monotonic_ns,
        mangohud_first_frame_raw_elapsed_ms: session.mangohud_first_frame_raw_elapsed_ms,
        metadata: session.metadata.clone(),
        target_pids_max: session.target_pids_max,
        active_target_pids_count: session.active_target_pids_count,
        active_expanded_tasks: session.active_expanded_tasks.clone(),
        interval_record_count: session.interval_record_count,
        intervals_dropped: session.intervals_dropped,
        spike_events_retained_count: session.spike_events_retained_count,
        spike_events_dropped_count: session.spike_events_dropped_count,
        spike_events_truncated: session.spike_events_truncated,
        scx_event_count: session.scx_event_count,
        irq_event_count: session.irq_event_count,
        migration_event_count: session.migration_event_count,
        cpu_freq_sample_count: session.cpu_freq_sample_count,
        gpu_sample_count: session.gpu_sample_count,
        frame_event_count: session.frame_event_count,
        block_io_event_count: session.block_io_event_count,
        event_stream_write_errors: session.event_stream_write_errors,
        alert_events_dropped_count: session.alert_events_dropped_count,
        alert_channel_closed_count: session.alert_channel_closed_count,
        first_event_stream_write_error: session.first_event_stream_write_error.clone(),
        block_io_correlation_basis: session.block_io_correlation_basis.clone(),
        drop_counters: session.drop_counters.clone(),
        cpu_perf_sample_count: session.cpu_perf_sample_count,
        cpu_perf_open_errors: session.cpu_perf_open_errors,
        cpu_perf_read_errors: session.cpu_perf_read_errors,
        cpu_perf_skipped_tasks: session.cpu_perf_skipped_tasks,
        cpu_perf_last_error: session.cpu_perf_last_error.clone(),
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
        task,
        active: true,
        class,
        process_pid: Some(task),
        process_comm: comm.into(),
        comm: comm.to_owned(),
        cpu: 0,
        wakeup_target_cpu: 0,
        prio: 120,
        latency_ns,
        wakeup_ns: switch_ns.saturating_sub(latency_ns),
        switch_ns,
        target_pending_wakeups: 0,
        major_faults: 0,
        minor_faults: 0,
        scx_ops: None,
        scx_state: None,
        scx_enable_seq: None,
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
    session.spike_events_retained_count = spikes.len() as u64;
    session.active_target_pids_count = spikes.len() as u64;
    session.active_expanded_tasks = spikes.iter().map(|spike| spike.task).collect();
    session.tasks = spikes
        .iter()
        .map(|spike| task_for_fixture(spike.task, spike.class, &spike.comm))
        .collect();
}

fn candidate_contains(diagnosis: &Diagnosis, cause: StutterCause) -> bool {
    diagnosis
        .candidates
        .iter()
        .any(|candidate| candidate.cause == cause)
}

#[test]
fn minimal_recording_fixture_files_exist() {
    let mut temp = env::temp_dir();
    temp.push(format!(
        "stutter-test-minimal-fixture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    write_minimal_recording_fixture(&temp);

    assert!(temp.join("session.json").exists());
    assert!(temp.join("metadata.json").exists());

    let _ = fs::remove_dir_all(temp);
}

#[test]
fn report_rejects_missing_required_artifacts() {
    let mut temp = env::temp_dir();
    temp.push(format!(
        "stutter-test-missing-artifacts-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&temp).unwrap();

    // Call the report loading helper. It should fail because the directory is empty.
    let result = report::print_report(&temp, false, false, false, 10, 500, None);

    assert!(result.is_err());
    let err_msg = format!("{:?}", result.err().unwrap()).to_lowercase();

    let contains_marker = err_msg.contains("metadata")
        || err_msg.contains("session")
        || err_msg.contains("recording")
        || err_msg.contains("missing");

    assert!(
        contains_marker,
        "Error message '{}' did not contain any of the required markers (metadata, session, recording, missing)",
        err_msg
    );

    // Cleanup
    let _ = fs::remove_dir_all(temp);
}

#[test]
fn minimal_recording_report_text_does_not_panic() {
    let mut temp = env::temp_dir();
    temp.push(format!(
        "stutter-test-report-render-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    write_minimal_recording_fixture(&temp);

    // Load the session to pass to the renderer.
    let session_path = temp.join("session.json");
    let session_data = fs::read_to_string(&session_path).unwrap();
    let session: SessionFile = serde_json::from_str(&session_data).unwrap();

    // Call the report rendering helper.
    // We use default/empty values for clusters and artifacts to keep it minimal.
    let output = report::render_report(
        &temp,
        &session,
        &SpikeClusterAnalysis {
            source: SpikeClusterSource::TopSpikesFallback,
            source_count: 0,
            clusters: vec![],
        },
        &[],
        &RunArtifacts::default(),
        10,
        500,
        None,
    );

    // Assert rendered text contains stable words from report.rs
    assert!(output.contains("stutter report"));
    assert!(output.contains("duration_ms"));

    let _ = fs::remove_dir_all(temp);
}

#[test]
fn recording_schema_round_trip_keeps_core_fields() {
    let mut temp = env::temp_dir();
    temp.push(format!(
        "stutter-test-schema-trip-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    write_minimal_recording_fixture(&temp);

    // This test performs a schema serde round-trip check only.
    // There is currently no standalone public loading helper in report.rs to use for full coverage.
    let session_path = temp.join("session.json");
    let file = fs::File::open(&session_path).unwrap();
    let reader = std::io::BufReader::new(file);
    let session: SessionFile = serde_json::from_reader(reader).unwrap();

    // Assert core fields survive (matching values from write_minimal_recording_fixture)
    assert_eq!(session.schema_version, SESSION_SCHEMA_VERSION);
    assert_eq!(session.tasks.len(), 1);
    assert_eq!(session.tasks[0].comm, "game");
    assert_eq!(session.spike_events_retained_count, 0);
    assert!(!session.spike_events_truncated);
    assert_eq!(session.block_io_correlation_basis, "dev+sector");

    // Assert drop counters (should be default/zero in the minimal fixture)
    assert_eq!(session.drop_counters.total(), 0);

    let _ = fs::remove_dir_all(temp);
}

// JSON report output is currently only exposed through the CLI printing path,
// so unit tests avoid brittle stdout capture.

#[test]
fn report_replay_fixture_game_thread_scheduler_delay() {
    let dir = temp_run_dir("replay-game-scheduler");
    let spikes = clustered_spikes(TaskClass::Game, "RenderThread", 8_000_000);
    write_base_run(&dir, "game_thread_scheduler_delay", |session| {
        apply_spike_session_fields(session, &spikes);
    });
    write_ndjson(dir.join("spike_events.json"), &spikes);

    let analysis = report::build_report_analysis(&dir, 10, 5, None).unwrap();
    let diagnosis = analysis.cluster_analysis.clusters[0]
        .diagnosis
        .as_ref()
        .unwrap();

    assert_eq!(diagnosis.cause, StutterCause::GameThreadSchedulerDelay);
    assert!(matches!(
        diagnosis.confidence,
        Confidence::High | Confidence::Medium
    ));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn report_replay_fixture_compositor_scheduler_delay() {
    let dir = temp_run_dir("replay-compositor-scheduler");
    let spikes = clustered_spikes(TaskClass::Compositor, "sway", 6_000_000);
    write_base_run(&dir, "compositor_scheduler_delay", |session| {
        apply_spike_session_fields(session, &spikes);
    });
    write_ndjson(dir.join("spike_events.json"), &spikes);

    let analysis = report::build_report_analysis(&dir, 10, 5, None).unwrap();
    let diagnosis = analysis.cluster_analysis.clusters[0]
        .diagnosis
        .as_ref()
        .unwrap();

    assert_eq!(diagnosis.cause, StutterCause::CompositorSchedulerDelay);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn report_replay_fixture_irq_overlap() {
    let dir = temp_run_dir("replay-irq-overlap");
    let spikes = vec![
        spike_event(100, TaskClass::Unknown, "worker-a", 3_000_000, 0),
        spike_event(101, TaskClass::Unknown, "worker-b", 2_500_000, 250_000),
        spike_event(102, TaskClass::Unknown, "worker-c", 2_000_000, 500_000),
    ];
    let irq_events = vec![IrqEventRecord {
        elapsed_ms: Some(100),
        irq: 137,
        cpu: 0,
        enter_ns: 99_000_000,
        exit_ns: 103_000_000,
        duration_ns: 4_000_000,
    }];
    write_base_run(&dir, "irq_overlap", |session| {
        apply_spike_session_fields(session, &spikes);
        session.irq_event_count = irq_events.len() as u64;
    });
    write_ndjson(dir.join("spike_events.json"), &spikes);
    write_ndjson(dir.join("irq_events.json"), &irq_events);

    let analysis = report::build_report_analysis(&dir, 10, 5, None).unwrap();
    let diagnosis = analysis.cluster_analysis.clusters[0]
        .diagnosis
        .as_ref()
        .unwrap();

    assert_eq!(diagnosis.cause, StutterCause::IrqDelayCandidate);

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn report_replay_fixture_gpu_bound_candidate() {
    let dir = temp_run_dir("replay-gpu-bound");
    let spikes = vec![
        spike_event(100, TaskClass::Unknown, "worker-a", 3_000_000, 0),
        spike_event(101, TaskClass::Unknown, "worker-b", 2_500_000, 250_000),
        spike_event(102, TaskClass::Unknown, "worker-c", 2_000_000, 500_000),
    ];
    let gpu_samples = vec![GpuSample {
        elapsed_ms: 100,
        gpu_busy_percent: Some(99),
        ..Default::default()
    }];
    write_base_run(&dir, "gpu_bound", |session| {
        apply_spike_session_fields(session, &spikes);
        session.gpu_sample_count = gpu_samples.len() as u64;
    });
    write_ndjson(dir.join("spike_events.json"), &spikes);
    write_ndjson(dir.join("gpu_samples.json"), &gpu_samples);

    let analysis = report::build_report_analysis(&dir, 10, 5, None).unwrap();
    let diagnosis = analysis.cluster_analysis.clusters[0]
        .diagnosis
        .as_ref()
        .unwrap();

    assert!(candidate_contains(
        diagnosis,
        StutterCause::GpuBoundCandidate
    ));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn report_replay_fixture_block_io_overlap_candidate() {
    let dir = temp_run_dir("replay-block-io");
    let spikes = vec![
        spike_event(100, TaskClass::Unknown, "worker-a", 3_000_000, 0),
        spike_event(101, TaskClass::Unknown, "worker-b", 2_500_000, 250_000),
        spike_event(102, TaskClass::Unknown, "worker-c", 2_000_000, 500_000),
    ];
    let io_events = vec![BlockIoRecord {
        elapsed_ms: 100,
        tid: 100,
        correlation_basis: "request-pointer".to_owned(),
        dev: 1,
        nr_sector: 8,
        sector: 2048,
        duration_ns: 8_000_000,
        timestamp_ns: 102_000_000,
        rwbs: "R".to_owned(),
    }];
    write_base_run(&dir, "block_io_overlap", |session| {
        apply_spike_session_fields(session, &spikes);
        session.block_io_event_count = io_events.len() as u64;
        session.block_io_correlation_basis = "request-pointer".to_owned();
    });
    write_ndjson(dir.join("spike_events.json"), &spikes);
    write_ndjson(dir.join("io_events.json"), &io_events);

    let analysis = report::build_report_analysis(&dir, 10, 5, None).unwrap();
    let diagnosis = analysis.cluster_analysis.clusters[0]
        .diagnosis
        .as_ref()
        .unwrap();

    assert!(candidate_contains(
        diagnosis,
        StutterCause::BlockIoCandidate
    ));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn report_replay_fixture_cpu_psi_pressure_candidate() {
    let dir = temp_run_dir("replay-cpu-psi");
    let spikes = vec![
        spike_event(100, TaskClass::Unknown, "worker-a", 3_000_000, 0),
        spike_event(101, TaskClass::Unknown, "worker-b", 2_500_000, 250_000),
        spike_event(102, TaskClass::Unknown, "worker-c", 2_000_000, 500_000),
    ];
    let intervals = vec![IntervalRecord {
        elapsed_ms: 100,
        task: 100,
        active: true,
        class: TaskClass::Unknown,
        comm: "worker-a".to_owned(),
        process_pid: Some(100),
        process_comm: "worker-a".into(),
        samples: 10,
        stored_samples: 10,
        cpu_psi_some: 80.0,
        percentile_scope: "exact".to_owned(),
        ..Default::default()
    }];
    write_base_run(&dir, "cpu_psi_pressure", |session| {
        apply_spike_session_fields(session, &spikes);
        session.interval_record_count = intervals.len() as u64;
    });
    write_ndjson(dir.join("spike_events.json"), &spikes);
    write_ndjson(dir.join("interval.json"), &intervals);

    let analysis = report::build_report_analysis(&dir, 10, 5, None).unwrap();
    let diagnosis = analysis.cluster_analysis.clusters[0]
        .diagnosis
        .as_ref()
        .unwrap();

    assert!(candidate_contains(
        diagnosis,
        StutterCause::CpuPressureCandidate
    ));

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn report_replay_fixture_low_quality_missing_optional_files() {
    let dir = temp_run_dir("replay-low-quality-missing");
    fs::create_dir_all(&dir).unwrap();
    let session = base_session("low_quality_missing_optional_files");
    write_json_pretty(dir.join("session.json"), &session);

    let analysis = report::build_report_analysis(&dir, 10, 5, None).unwrap();

    assert!(matches!(
        analysis.data_quality.level,
        DataQualityLevel::Medium | DataQualityLevel::Low
    ));
    assert!(
        analysis
            .data_quality
            .missing_optional_files
            .iter()
            .any(|file| file == "metadata.json")
    );

    let _ = fs::remove_dir_all(dir);
}

#[test]
fn report_check_detects_regression_from_new_scored_task() {
    let mut baseline_dir = env::temp_dir();
    baseline_dir.push(format!(
        "stutter-test-check-baseline-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    let mut current_dir = env::temp_dir();
    current_dir.push(format!(
        "stutter-test-check-current-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    // Baseline: Create a recording with no tasks that have samples.
    // We'll just write an empty session file for simplicity.
    fs::create_dir_all(&baseline_dir).unwrap();
    let session_baseline = SessionFile {
        schema_version: SESSION_SCHEMA_VERSION,
        run_name: Some("baseline".to_owned()),
        started_at: mk_dummy_time(),
        ended_at: mk_dummy_time(),
        monotonic_start_ns: Some(0),
        monotonic_end_ns: Some(1000),
        duration_ms: 1,
        mangohud_start_offset: None,
        mangohud_first_frame_monotonic_ns: None,
        mangohud_first_frame_raw_elapsed_ms: None,
        stop_reason: "manual".to_owned(),
        config: mk_dummy_config(),
        metadata: SystemMetadata::default(),
        target_pids_max: 1024,
        active_target_pids_count: 0,
        active_expanded_tasks: vec![],
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
        tasks: vec![],
        top_spikes: vec![],
    };
    let file = fs::File::create(baseline_dir.join("session.json")).unwrap();
    serde_json::to_writer(file, &session_baseline).unwrap();

    // Current: Use the minimal fixture which contains one TaskClass::Game task with p99_ns: 2000.
    write_minimal_recording_fixture(&current_dir);

    // Call check helper with 0.0 threshold. It should fail because 2000ns > 0.0ms.
    let result = report::check_percentile_regression(&baseline_dir, &current_dir, 0.0);
    assert!(result.is_err());
    let err_msg = format!("{:?}", result.err().unwrap());
    assert!(err_msg.contains("regression_check_failed"));

    let _ = fs::remove_dir_all(baseline_dir);
    let _ = fs::remove_dir_all(current_dir);
}

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
        csv_path: None,
        irq_latency: false,
        irqs: vec![],
        hwmon: false,
        hwmon_root: None,
        hwmon_drm_card: None,
        hwmon_render_node: None,
        mangohud_log: None,
        tui: false,
        summary_period_ms: 1000,
        epoch_period_ms: None,
        retain_intervals: None,
        max_tasks: 1024,
        spike_threshold_ns: 1_000_000,
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
    }
}
