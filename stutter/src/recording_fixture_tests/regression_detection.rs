use super::*;

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
        core: crate::recorder::SessionMetadataCore {
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
            ..Default::default()
        },
        stop_reason: "manual".to_owned(),
        config: mk_dummy_config(),
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
