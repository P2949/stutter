//! Regression coverage for recording finalization, interval CSV output, and run directory handling.

use super::{support::*, *};

#[test]
fn recording_serializes_sorted_tasks_schema_histogram_spikes_and_drop_counters() {
    let dir = temp_test_dir("recording-schema");
    fs::create_dir_all(&dir).unwrap();

    let recording = RecordingRun {
        run_name: Some("schema-test".to_owned()),
        run_dir: dir.clone(),
        started_at: UNIX_EPOCH,
        started_instant: Instant::now(),
        monotonic_start_ns: Some(1_000),
        mangohud_start_offset: None,
        mangohud_first_frame_monotonic_ns: None,
        mangohud_first_frame_raw_elapsed_ms: None,
    };
    let mut config = test_config(vec![9, 1, 4], vec![], Some(Duration::from_secs(1)));
    config.target.cgroupv2 = Some(PathBuf::from("/sys/fs/cgroup/game"));
    config.target.exclude_tree_pids = vec![77];
    config.safety.follow_exec = false;
    config.target.max_tasks = 2048;
    config.recording.retain_intervals = Some(8);
    config.hwmon.root = Some(PathBuf::from("/tmp/hwmon"));
    let active_targets = BTreeMap::from([
        (9, task_info(9, 9, "task-9", "task-9", TaskClass::Helper)),
        (1, task_info(1, 1, "task-1", "task-1", TaskClass::Helper)),
        (4, task_info(4, 4, "task-4", "task-4", TaskClass::Helper)),
    ]);

    let mut stats = metrics::TaskStats::new(7, "worker".to_owned(), 0);
    stats.apply_task_info(&task_info(7, 7, "task-7", "worker", TaskClass::Helper));
    stats.session_latency.record(1_000);
    stats.session_latency.record(2_000_000);
    let stats_by_task = BTreeMap::from([(7, stats)]);
    let spike_events = [SpikeEvent {
        task: 7,
        class: TaskClass::Helper,
        process_pid: Some(7),
        process_comm: "task-7".into(),
        comm: "worker".into(),
        cpu: 1,
        wakeup_target_cpu: 1,
        prio: 120,
        latency_ns: 2_000_000,
        wakeup_ns: 10,
        switch_ns: 2_000_010,
        switch_prev_pid: 12345,
        switch_prev_state: 2, // TASK_UNINTERRUPTIBLE
        switch_prev_state_label: "voluntary_sleep_uninterruptible".to_owned(),
        active: true,
        elapsed_ms: Some(1),
        ..Default::default()
    }];
    let drop_counters = DropCountersSnapshot {
        wakeup_data_insert_failed: 2,
        wakeup_data_stale_entries: 0,
        ringbuf_reserve_failed: 3,
        irq_start_times_insert_failed: 0,
        block_start_insert_failed: 0,
        block_fallback_key_collisions: 0,
    };

    let task_tracker = tasks::TaskTracker {
        active_targets,
        stats_by_task,
        ..Default::default()
    };

    let mut recorder = recorder::LiveRecorder {
        run: Some(recording),
        buffers: recorder::LiveBuffers {
            spike_events: Some(SpikeEventBuffer::default()),
            ..Default::default()
        },
        ..Default::default()
    };
    recorder
        .buffers
        .spike_events
        .as_mut()
        .unwrap()
        .push(spike_events[0].clone());
    recorder.buffers.spike_events.as_mut().unwrap().truncate(); // Force truncated state for testing

    let monitor_config = config.clone();
    recorder::finalize_recording(FinalizeRecordingInput {
        recorder: &recorder,
        config: &monitor_config,
        tree_pids: &config.target.tree_pids,
        stop_reason: "test",
        tasks: &task_tracker,
        frame_events: &[],
        block_io_correlation_basis: "dev+sector".to_owned(),
        block_io_correlation_confidence: "medium".to_owned(),
        drop_counters,
        cpu_perf_status: None,
        focus_mode: None,
        final_focus_kind: None,
        focus_switch_count: 0,
        current_focus: None,
        final_foreground_event: None,
    })
    .unwrap();

    let artifacts =
        crate::session_io::load_run_artifacts(&dir, ArtifactSelection::report()).unwrap();
    let session = artifacts.session;
    let metadata = artifacts.metadata.unwrap();
    let recordedspike_events = artifacts.spikes;

    assert_eq!(session.core.schema_version, SESSION_SCHEMA_VERSION);
    assert_eq!(session.core.active_expanded_tasks, vec![1, 4, 9]);
    assert_eq!(metadata.core.active_expanded_tasks, vec![1, 4, 9]);
    assert_eq!(session.core.spike_events_retained_count, 1);
    assert_eq!(metadata.core.spike_events_retained_count, 1);
    assert_eq!(session.core.scx_event_count, 0);
    assert_eq!(metadata.core.scx_event_count, 0);
    assert_eq!(session.core.spike_events_dropped_count, 0);
    assert_eq!(metadata.core.spike_events_dropped_count, 0);
    assert!(session.core.spike_events_truncated);
    assert!(metadata.core.spike_events_truncated);
    assert_eq!(session.core.drop_counters.total(), 5);
    assert_eq!(metadata.core.drop_counters.total(), 5);
    assert_eq!(session.core.drop_counters.wakeup_data_insert_failed, 2);
    assert_eq!(session.core.drop_counters.ringbuf_reserve_failed, 3);
    assert_eq!(
        session.config.cgroupv2,
        Some(PathBuf::from("/sys/fs/cgroup/game"))
    );
    assert_eq!(session.config.exclude_tree_pids, vec![77]);
    assert!(!session.config.follow_exec);
    assert_eq!(session.config.max_tasks, 2048);
    assert_eq!(session.config.retain_intervals, Some(8));
    assert_eq!(session.config.hwmon_root, Some(PathBuf::from("/tmp/hwmon")));
    assert_eq!(recordedspike_events.len(), 1);
    assert_eq!(recordedspike_events[0].task, 7);
    assert_eq!(recordedspike_events[0].switch_prev_pid, 12345);
    assert_eq!(recordedspike_events[0].switch_prev_state, 2);
    assert_eq!(
        recordedspike_events[0].switch_prev_state_label,
        "voluntary_sleep_uninterruptible"
    );
    assert_eq!(
        session.tasks[0]
            .latency
            .histogram
            .iter()
            .map(|bucket| bucket.count)
            .sum::<u64>(),
        2
    );

    fs::remove_dir_all(dir).ok();
}
#[test]
fn interval_csv_writer_outputs_header_and_rows() {
    let dir = temp_test_dir("interval-csv");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("interval.csv");
    let mut stats = metrics::TaskStats::new(7, "worker,quoted".to_owned(), 0);
    stats.apply_task_info(&task_info(7, 7, "proc", "worker,quoted", TaskClass::Helper));
    let mut latency = metrics::LatencyStats::new();
    latency.record(1_000_000);
    let latency = latency.snapshot().unwrap();
    let cpu = metrics::CpuStatsSet::new().snapshot();
    let record = metrics::interval_record_from_snapshot(metrics::IntervalRecordFromSnapshotInput {
        task: 7,
        stats: &mut stats,
        latency: &latency,
        cpu: &cpu,
        elapsed_ms: 123,
        drop_counters: &Default::default(),
        psi: None,
        faults_delta: (0, 0),
    });

    recorder::write_interval_csv(&path, &[record]).unwrap();
    let csv = fs::read_to_string(&path).unwrap();

    assert!(csv.starts_with("elapsed_ms,task,active"));
    assert!(csv.contains("\"worker,quoted\""));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn prepare_recording_refuses_to_overwrite_existing_output_dir() {
    let dir = temp_test_dir("recording-existing-dir");
    fs::create_dir_all(&dir).unwrap();

    let mut config = test_config(vec![7], vec![], None);
    config.recording.run_name = Some("collision-test".to_owned());
    config.recording.output_dir = Some(dir.clone());
    let monitor_config = config;

    let err = recorder::prepare_recording(&monitor_config).unwrap_err();

    assert!(format!("{err:#}").contains("output directory already exists"));

    fs::remove_dir_all(dir).ok();
}
