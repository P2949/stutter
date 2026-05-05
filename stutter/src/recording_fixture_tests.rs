use std::{env, fs, path::Path};

use crate::{
    ebpf_loader::DropCountersSnapshot,
    metadata::SystemMetadata,
    process_tree::TaskClass,
    recorder::{
        MetadataFile, RecordedConfig, RecordedCpuSnapshot, RecordedLatency, RecordedTime,
        SESSION_SCHEMA_VERSION, SessionFile, SessionTask,
    },
    report::{self, RunArtifacts, SpikeClusterAnalysis, SpikeClusterSource},
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
    };

    fs::create_dir_all(dir).unwrap();
    let session_file = fs::File::create(dir.join("session.json")).unwrap();
    serde_json::to_writer_pretty(session_file, &session).unwrap();

    let metadata_file_ptr = fs::File::create(dir.join("metadata.json")).unwrap();
    serde_json::to_writer_pretty(metadata_file_ptr, &metadata_file).unwrap();
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
    let result = report::print_report(&temp, false, false, 10, 500, None);

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

    // Load the session using the same logic as report.rs
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
    assert!(err_msg.contains("percentile_regression_check_failed"));

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
        block_io: false,
        stat_wait: false,
    }
}
