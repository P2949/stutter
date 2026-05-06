#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::useless_vec)]
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use stutter_common::{EVENT_RUNNABLE_LATENCY, SchedulerEvent};

use crate::{
    cli::{Config, RecordingConfig},
    ebpf_loader::DropCountersSnapshot,
    events::{self, AlertPayload},
    metadata::SystemMetadata,
    metrics,
    process_tree::{self, TargetDiffAction, TaskClass, TaskInfo},
    recorder::{
        self, FinalizeRecordingInput, FrameEvent, GpuSample, IrqEventRecord, RecordedCpuSnapshot,
        RecordedLatency, RecordingRun, SESSION_SCHEMA_VERSION, SessionFile, SessionTask,
        SpikeEvent, SpikeEventBuffer, recorded_config, recorded_time,
    },
    tasks, tune,
};

#[test]
fn reused_tid_with_different_task_resets_stats_after_removal() {
    let mut stats_by_task = BTreeMap::from([(
        42,
        task_stats_with_info(42, 100, "old-game", "old-thread", TaskClass::Game, 10),
    )]);
    {
        let stats = stats_by_task.get_mut(&42).unwrap();
        stats.removed_ms = Some(50);
        stats.session_latency.record(3_000_000);
    }

    let new_task = task_info(42, 200, "new-game", "new-thread", TaskClass::Helper);
    tasks::reactivate_or_reset_stats_inner(&mut stats_by_task, None, 42, &new_task, 77);

    let stats = stats_by_task.get(&42).unwrap();
    assert_eq!(stats.first_seen_ms, 77);
    assert_eq!(stats.last_seen_ms, 77);
    assert_eq!(stats.session_latency.count, 0);
    assert_eq!(stats.process_pid, Some(200));
    assert_eq!(stats.process_comm, "new-game".into());
    assert_eq!(stats.comm, "new-thread");
    assert_eq!(stats.class, TaskClass::Helper);
    assert!(stats.active);
    assert_eq!(stats.removed_ms, None);
}

#[test]
fn active_same_tid_replacement_resets_stats_even_without_remove_add_diff() {
    let old_task = task_info(42, 100, "old-game", "old-worker", TaskClass::Game);
    let new_task = task_info(42, 200, "new-helper", "new-worker", TaskClass::Helper);

    let active_targets = BTreeMap::from([(42, old_task.clone())]);
    let desired_tasks = BTreeMap::from([(42, new_task.clone())]);
    let known_targets = active_targets.clone();

    let mut stats = task_stats_with_info(42, 100, "old-game", "old-worker", TaskClass::Game, 10);
    stats.session_latency.record(5_000_000);
    let stats_by_task = BTreeMap::from([(42, stats)]);

    let mut tree_events = Vec::new();
    let mut tasks = tasks::TaskTracker {
        active_targets,
        known_targets,
        stats_by_task,
        task_exe_inodes: BTreeMap::new(),
        prev_faults_snapshot: BTreeMap::from([(42, (10, 20))]),
        cache: process_tree::ProcessCache::default(),
    };

    tasks.handle_replacements(
        &desired_tasks,
        &mut tree_events,
        &mut None,
        77,
        Some(Instant::now()),
    );

    let stats = tasks.stats_by_task.get(&42).unwrap();
    assert_eq!(stats.first_seen_ms, 77);
    assert_eq!(stats.last_seen_ms, 77);
    assert_eq!(stats.session_latency.count, 0);
    assert_eq!(stats.process_pid, Some(200));
    assert_eq!(stats.process_comm, "new-helper".into());
    assert_eq!(stats.comm, "new-worker");
    assert_eq!(stats.class, TaskClass::Helper);
    assert!(stats.active);
    assert!(!tasks.prev_faults_snapshot.contains_key(&42));

    assert_eq!(tasks.known_targets.get(&42), Some(&new_task));
    assert_eq!(tree_events.len(), 1);
    assert_eq!(tree_events[0].action, "replaced");
    assert_eq!(tree_events[0].tid, 42);
    assert_eq!(tree_events[0].process_pid, 200);
}

#[test]
fn same_reused_tid_reactivates_without_clearing_stats() {
    let mut stats_by_task = BTreeMap::from([(
        42,
        task_stats_with_info(42, 100, "game", "worker", TaskClass::Game, 10),
    )]);
    {
        let stats = stats_by_task.get_mut(&42).unwrap();
        stats.removed_ms = Some(50);
        stats.session_latency.record(3_000_000);
    }

    let same_task = task_info(42, 100, "game", "worker", TaskClass::Game);
    tasks::reactivate_or_reset_stats_inner(&mut stats_by_task, None, 42, &same_task, 77);

    let stats = stats_by_task.get(&42).unwrap();
    assert_eq!(stats.first_seen_ms, 10);
    assert_eq!(stats.session_latency.count, 1);
    assert_eq!(stats.removed_ms, None);
    assert!(stats.active);
}

#[test]
fn same_tid_same_names_different_starttime_resets_stats() {
    let mut stats_by_task = BTreeMap::from([(
        42,
        task_stats_with_info(42, 100, "game", "worker", TaskClass::Game, 10),
    )]);
    {
        let stats = stats_by_task.get_mut(&42).unwrap();
        stats.removed_ms = Some(50);
        stats.session_latency.record(3_000_000);
    }

    let mut new_task = task_info(42, 100, "game", "worker", TaskClass::Game);
    new_task.task_starttime_ticks = Some(999);
    tasks::reactivate_or_reset_stats_inner(&mut stats_by_task, None, 42, &new_task, 77);

    let stats = stats_by_task.get(&42).unwrap();
    assert_eq!(stats.first_seen_ms, 77);
    assert_eq!(stats.session_latency.count, 0);
    assert_eq!(stats.task_starttime_ticks, Some(999));
}

#[test]
fn event_comm_updates_only_unknown_existing_name() {
    let config = test_config(vec![7], vec![], None);
    let stats_by_task = BTreeMap::from([(7, metrics::TaskStats::new(7, "?".to_owned(), 0))]);

    let first_event = scheduler_event(7, "real-name");
    let mut tasks = tasks::TaskTracker::default();
    tasks.stats_by_task = stats_by_task;
    let mut recorder = recorder::LiveRecorder::default();

    events::handle_event(
        &first_event,
        &config,
        Instant::now(),
        &mut tasks,
        None,
        &mut recorder,
        None,
        None,
        None,
        None,
    );

    assert_eq!(tasks.stats_by_task.get(&7).unwrap().comm, "real-name");

    let second_event = scheduler_event(7, "later-name");
    events::handle_event(
        &second_event,
        &config,
        Instant::now(),
        &mut tasks,
        None,
        &mut recorder,
        None,
        None,
        None,
        None,
    );

    assert_eq!(tasks.stats_by_task.get(&7).unwrap().comm, "real-name");
}

#[test]
fn spike_events_capture_only_threshold_crossing_events() {
    let config = test_config(vec![7], vec![], None);
    let active_targets = BTreeMap::from([(
        7,
        task_info(7, 77, "KingdomCome.exe", "RenderThread", TaskClass::Game),
    )]);
    let spike_events = SpikeEventBuffer::default();

    let below_threshold = scheduler_event_with_latency(7, "RenderThread", 999_999);
    let mut tasks = tasks::TaskTracker::default();
    let stats_by_task = BTreeMap::<u32, crate::metrics::TaskStats>::new();
    tasks.active_targets = active_targets;
    tasks.stats_by_task = stats_by_task;
    let mut recorder = recorder::LiveRecorder::default();
    recorder.spike_events = Some(spike_events);

    events::handle_event(
        &below_threshold,
        &config,
        Instant::now(),
        &mut tasks,
        Some(100),
        &mut recorder,
        None,
        None,
        None,
        None,
    );
    assert!(
        recorder
            .spike_events
            .as_ref()
            .unwrap()
            .as_slice()
            .is_empty()
    );

    let at_threshold = scheduler_event_with_latency(7, "RenderThread", 1_000_000);
    events::handle_event(
        &at_threshold,
        &config,
        Instant::now(),
        &mut tasks,
        Some(100),
        &mut recorder,
        None,
        None,
        None,
        None,
    );

    let spike_events_slice = recorder.spike_events.as_ref().unwrap().as_slice();
    assert_eq!(spike_events_slice.len(), 1);
    let spike = &spike_events_slice[0];
    assert_eq!(spike.task, 7);
    assert!(spike.active);
    assert_eq!(spike.class, TaskClass::Game);
    assert_eq!(spike.process_pid, Some(77));
    assert_eq!(spike.process_comm, "KingdomCome.exe".into());
    assert_eq!(spike.comm, "RenderThread");
    assert_eq!(spike.cpu, 0);
    assert_eq!(spike.prio, 120);
    assert_eq!(spike.latency_ns, 1_000_000);
    assert_eq!(spike.wakeup_ns, 100);
    assert_eq!(spike.switch_ns, 1_000_100);
    assert_eq!(spike.elapsed_ms, Some(1));
}

#[test]
fn spike_event_fault_deltas_are_captured_correctly() {
    let config = test_config(vec![7], vec![], None);
    let active_targets = BTreeMap::from([(
        7,
        task_info(7, 77, "KingdomCome.exe", "RenderThread", TaskClass::Game),
    )]);
    let spike_events = SpikeEventBuffer::default();

    // First event establishes baseline faults
    let mut first_event = scheduler_event_with_latency(7, "RenderThread", 10);
    first_event.maj_flt = 10;
    first_event.min_flt = 20;

    let stats_by_task = BTreeMap::<u32, crate::metrics::TaskStats>::new();
    let mut tasks = tasks::TaskTracker::default();
    tasks.active_targets = active_targets;
    tasks.stats_by_task = stats_by_task;
    let mut recorder = recorder::LiveRecorder::default();
    recorder.spike_events = Some(spike_events);

    events::handle_event(
        &first_event,
        &config,
        Instant::now(),
        &mut tasks,
        Some(100),
        &mut recorder,
        None,
        None,
        None,
        None,
    );

    // Second event is a spike with additional faults
    let mut spike_event = scheduler_event_with_latency(7, "RenderThread", 1_000_000);
    spike_event.maj_flt = 15; // +5 delta
    spike_event.min_flt = 30; // +10 delta

    events::handle_event(
        &spike_event,
        &config,
        Instant::now(),
        &mut tasks,
        Some(100),
        &mut recorder,
        None,
        None,
        None,
        None,
    );

    let spike_events_slice = recorder.spike_events.as_ref().unwrap().as_slice();
    assert_eq!(spike_events_slice.len(), 1);
    let spike = &spike_events_slice[0];
    assert_eq!(spike.major_faults, 5);
    assert_eq!(spike.minor_faults, 10);

    // Also verify TaskStats internal top_spikes has the same deltas
    let stats = tasks.stats_by_task.get(&7).unwrap();
    assert_eq!(stats.top_spikes.len(), 1);
    assert_eq!(stats.top_spikes[0].major_faults, 5);
    assert_eq!(stats.top_spikes[0].minor_faults, 10);
}

#[test]
fn alert_payload_captures_spike_task_identity() {
    let event = scheduler_event_with_latency(7, "RenderThread", 250_000_000);
    let mut stats = metrics::TaskStats::new(7, "RenderThread".to_owned(), 10);
    stats.apply_task_info(&task_info(
        7,
        77,
        "KingdomCome.exe",
        "RenderThread",
        TaskClass::Game,
    ));

    let payload = AlertPayload::from_task_stats(&stats, &event, 1234, None, None, None);

    assert_eq!(payload.title, "stutter latency alert");
    assert_eq!(payload.task, 7);
    assert_eq!(payload.class, TaskClass::Game);
    assert_eq!(payload.comm, "RenderThread");
    assert_eq!(payload.process_pid, Some(77));
    assert_eq!(payload.process_comm, "KingdomCome.exe");
    assert_eq!(payload.latency_ns, 250_000_000);
    assert_eq!(payload.latency_ms, 250);
    assert_eq!(payload.elapsed_ms, 1234);
    assert!(payload.message.contains("latency=250.000ms"));
}

#[test]
fn spike_event_buffer_caps_and_marks_truncated() {
    let mut buffer = SpikeEventBuffer::with_max_events(2);

    buffer.push(spike_event(1, 1_000));
    buffer.push(spike_event(2, 2_000));
    buffer.push(spike_event(3, 3_000));

    assert_eq!(buffer.as_slice().len(), 2);
    assert!(buffer.truncated());
    assert_eq!(buffer.as_slice()[0].task, 1);
    assert_eq!(buffer.as_slice()[1].task, 2);
}

#[test]
fn histogram_records_boundaries_and_overflow() {
    let mut histogram = metrics::LatencyHistogram::new();

    histogram.record(1_000);
    histogram.record(1_001);
    histogram.record(60_000_000);

    let buckets = histogram.snapshot();

    assert_eq!(buckets[0].upper_bound_ns, Some(1_000));
    assert_eq!(buckets[0].count, 1);
    assert_eq!(buckets[1].upper_bound_ns, Some(2_000));
    assert_eq!(buckets[1].count, 1);
    assert_eq!(buckets.last().unwrap().upper_bound_ns, None);
    assert_eq!(buckets.last().unwrap().count, 1);
}

#[test]
fn histogram_percentile_uses_conservative_bucket_upper_bound() {
    let mut histogram = metrics::LatencyHistogram::new();

    for _ in 0..95 {
        histogram.record(1_000);
    }
    for _ in 0..5 {
        histogram.record(1_500_000);
    }

    assert_eq!(histogram.percentile_upper_bound(100, 0.95), Some(1_000));
    assert_eq!(histogram.percentile_upper_bound(100, 0.99), Some(2_000_000));
}

#[test]
fn untruncated_snapshot_uses_exact_percentiles() {
    let mut stats = metrics::LatencyStats::new();

    stats.record(1_234);
    stats.record(9_876);

    let snapshot = stats.snapshot().unwrap();

    assert_eq!(snapshot.percentile_scope, "exact");
    assert_eq!(snapshot.stored_samples, 2);
    assert_eq!(snapshot.samples_truncated, 0);
    assert_eq!(snapshot.p95_ns, 9_876);
    assert_eq!(snapshot.p99_ns, 9_876);
}

#[test]
fn truncated_snapshot_uses_histogram_percentiles() {
    let mut stats = metrics::LatencyStats::new();

    for _ in 0..metrics::MAX_EXACT_SAMPLES {
        stats.record(1_000);
    }
    for _ in 0..4_000 {
        stats.record(2_000_000);
    }

    let snapshot = stats.snapshot().unwrap();

    assert_eq!(snapshot.percentile_scope, "histogram");
    assert_eq!(snapshot.samples_truncated, 4_000);
    assert_eq!(snapshot.p95_ns, 2_000_000);
    assert_eq!(snapshot.p99_ns, 2_000_000);
}

#[test]
fn snapshot_and_reset_clears_histogram_state() {
    let mut stats = metrics::LatencyStats::new();

    stats.record(1_000);
    assert!(stats.snapshot_and_reset().is_some());
    stats.record(60_000_000);

    let snapshot = stats.snapshot().unwrap();
    assert_eq!(snapshot.count, 1);
    assert_eq!(snapshot.histogram[0].count, 0);
    assert_eq!(snapshot.histogram.last().unwrap().count, 1);
}

#[test]
fn diff_tasks_orders_removed_before_added_by_tid() {
    let old_tasks = BTreeMap::from([
        (2, task_info(2, 20, "old", "old-2", TaskClass::Helper)),
        (4, task_info(4, 40, "old", "old-4", TaskClass::Helper)),
    ]);
    let new_tasks = BTreeMap::from([
        (1, task_info(1, 10, "new", "new-1", TaskClass::Game)),
        (3, task_info(3, 30, "new", "new-3", TaskClass::Game)),
    ]);

    let diffs = process_tree::diff_tasks_ref(&old_tasks, &new_tasks);

    let actions_and_tids = diffs
        .iter()
        .map(|diff| (&diff.action, diff.task.tid))
        .collect::<Vec<_>>();
    assert_eq!(
        actions_and_tids,
        vec![
            (&TargetDiffAction::Removed, 2),
            (&TargetDiffAction::Removed, 4),
            (&TargetDiffAction::Added, 1),
            (&TargetDiffAction::Added, 3),
        ]
    );
}

#[test]
fn classify_task_known_classes() {
    assert_eq!(
        process_tree::classify_task("gamescope", "gamescope", ""),
        TaskClass::GameScope
    );
    assert_eq!(
        process_tree::classify_task("sway", "sway", ""),
        TaskClass::Compositor
    );
    assert_eq!(
        process_tree::classify_task("wineserver", "wineserver", ""),
        TaskClass::WineServer
    );
    assert_eq!(
        process_tree::classify_task("steamwebhelper", "steamwebhelper", ""),
        TaskClass::Helper
    );
}

#[test]
fn target_snapshot_adds_fallback_without_o_n_squared_duplicate_scan_behavior() {
    let dir = temp_test_dir("proc-snapshot-fallback");
    create_fake_proc(&dir, 10, 1, "root", "root", &[10]);
    create_fake_proc(&dir, 11, 10, "child", "child", &[]);

    let snapshot = process_tree::target_snapshot(
        process_tree::TargetSnapshotInput::default()
            .proc_root(&dir)
            .manual_pids(&[])
            .tree_pids(&[10])
            .cgroup_path(None),
    );

    assert!(snapshot.process_roots.contains(&10));
    assert!(snapshot.process_roots.contains(&11));
    assert!(snapshot.tasks.contains_key(&10));
    assert!(snapshot.tasks.contains_key(&11));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn target_snapshot_does_not_add_unknown_fallback_for_missing_tree_root() {
    let dir = temp_test_dir("proc-missing-tree-root");
    fs::create_dir_all(&dir).unwrap();

    let snapshot = process_tree::target_snapshot(
        process_tree::TargetSnapshotInput::default()
            .proc_root(&dir)
            .manual_pids(&[])
            .tree_pids(&[42])
            .cgroup_path(None),
    );

    assert!(snapshot.tasks.is_empty());
    assert!(snapshot.process_roots.is_empty());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn target_snapshot_drops_manual_missing_pid_by_default() {
    let dir = temp_test_dir("proc-missing-manual-pid");
    fs::create_dir_all(&dir).unwrap();

    let snapshot = process_tree::target_snapshot(
        process_tree::TargetSnapshotInput::default()
            .proc_root(&dir)
            .manual_pids(&[42])
            .tree_pids(&[])
            .cgroup_path(None),
    );

    assert!(!snapshot.tasks.contains_key(&42));
    fs::remove_dir_all(dir).ok();
}

#[test]
fn target_snapshot_keeps_manual_missing_pid_when_requested() {
    let dir = temp_test_dir("proc-keep-missing-manual-pid");
    fs::create_dir_all(&dir).unwrap();

    let mut cache = process_tree::ProcessCache::default();
    let snapshot = process_tree::target_snapshot(
        process_tree::TargetSnapshotInput::default()
            .proc_root(&dir)
            .manual_pids(&[42])
            .keep_missing_pid(true)
            .cache(&mut cache),
    );

    let task = snapshot.tasks.get(&42).unwrap();
    assert_eq!(task.comm, "?");
    assert_eq!(task.class, TaskClass::Unknown);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn target_snapshot_accepts_manual_thread_ids() {
    let dir = temp_test_dir("proc-manual-thread-id");
    create_fake_proc(&dir, 10, 1, "game", "game", &[10, 11, 12]);

    let snapshot = process_tree::target_snapshot(
        process_tree::TargetSnapshotInput::default()
            .proc_root(&dir)
            .manual_pids(&[11])
            .tree_pids(&[])
            .cgroup_path(None),
    );

    assert_eq!(snapshot.tasks.keys().copied().collect::<Vec<_>>(), vec![11]);
    let task = snapshot.tasks.get(&11).unwrap();
    assert_eq!(task.process_pid, 10);
    assert_eq!(task.comm, "game-11");
    fs::remove_dir_all(dir).ok();
}

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
    config.cgroupv2 = Some(PathBuf::from("/sys/fs/cgroup/game"));
    config.exclude_tree_pids = vec![77];
    config.follow_exec = false;
    config.max_tasks = 2048;
    config.retain_intervals = Some(8);
    config.hwmon_root = Some(PathBuf::from("/tmp/hwmon"));
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
    let spike_events = vec![SpikeEvent {
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
        target_pending_wakeups: 0,
        observed_runnable_depth: 0,
        major_faults: 0,
        minor_faults: 0,
        active: true,
        elapsed_ms: Some(1),
        scx_ops: None,
        scx_state: None,
        scx_enable_seq: None,
        cause_tags: Vec::new(),
        primary_cause: None,
    }];
    let drop_counters = DropCountersSnapshot {
        wakeup_data_insert_failed: 2,
        ringbuf_reserve_failed: 3,
        irq_start_times_insert_failed: 0,
        block_start_insert_failed: 0,
    };

    let mut task_tracker = tasks::TaskTracker::default();
    task_tracker.active_targets = active_targets;
    task_tracker.stats_by_task = stats_by_task;

    let mut recorder = recorder::LiveRecorder::default();
    recorder.run = Some(recording);
    recorder.spike_events = Some(SpikeEventBuffer::default());
    recorder
        .spike_events
        .as_mut()
        .unwrap()
        .push(spike_events[0].clone());
    recorder.spike_events.as_mut().unwrap().truncate(); // Force truncated state for testing

    recorder::finalize_recording(FinalizeRecordingInput {
        recorder: &recorder,
        config: &config,
        tree_pids: &config.tree_pids,
        stop_reason: "test",
        tasks: &task_tracker,
        frame_events: &[],
        block_io_correlation_basis: "dev+sector",
        drop_counters,
        cpu_perf_status: None,
    })
    .unwrap();

    let artifacts =
        crate::session_io::load_run_artifacts(&dir, crate::session_io::ArtifactLoadOptions::REPORT)
            .unwrap();
    let session = artifacts.session;
    let metadata = artifacts.metadata.unwrap();
    let recordedspike_events = artifacts.spikes;

    assert_eq!(session.schema_version, SESSION_SCHEMA_VERSION);
    assert_eq!(session.active_expanded_tasks, vec![1, 4, 9]);
    assert_eq!(metadata.active_expanded_tasks, vec![1, 4, 9]);
    assert_eq!(session.spike_events_retained_count, 1);
    assert_eq!(metadata.spike_events_retained_count, 1);
    assert_eq!(session.scx_event_count, 0);
    assert_eq!(metadata.scx_event_count, 0);
    assert_eq!(session.spike_events_dropped_count, 0);
    assert_eq!(metadata.spike_events_dropped_count, 0);
    assert!(session.spike_events_truncated);
    assert!(metadata.spike_events_truncated);
    assert_eq!(session.drop_counters.total(), 5);
    assert_eq!(metadata.drop_counters.total(), 5);
    assert_eq!(session.drop_counters.wakeup_data_insert_failed, 2);
    assert_eq!(session.drop_counters.ringbuf_reserve_failed, 3);
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
fn target_snapshot_filters_include_and_exclude_comm_patterns() {
    let dir = temp_test_dir("proc-snapshot-filters");
    create_fake_proc(&dir, 10, 1, "game", "KingdomCome.exe", &[10, 11, 12]);
    fs::write(dir.join("10/task/10/comm"), "RenderThread\n").unwrap();
    fs::write(dir.join("10/task/11/comm"), "AudioThread\n").unwrap();
    fs::write(dir.join("10/task/12/comm"), "steamwebhelper\n").unwrap();

    let filters = process_tree::TaskFilters {
        include_comm: vec![process_tree::CompiledPattern::new("thread".to_owned()).unwrap()],
        exclude_comm: vec![
            process_tree::CompiledPattern::new("STEAMWEBHELPER".to_owned()).unwrap(),
        ],
    };
    let snapshot = process_tree::target_snapshot(
        process_tree::TargetSnapshotInput::default()
            .proc_root(&dir)
            .manual_pids(&[])
            .tree_pids(&[10])
            .cgroup_path(None)
            .filters(&filters),
    );

    assert!(
        snapshot
            .tasks
            .values()
            .any(|task| task.comm == "RenderThread")
    );
    assert!(
        snapshot
            .tasks
            .values()
            .any(|task| task.comm == "AudioThread")
    );
    assert!(
        !snapshot
            .tasks
            .values()
            .any(|task| task.comm == "steamwebhelper")
    );
    fs::remove_dir_all(dir).ok();
}

#[test]
fn target_snapshot_prefetches_exe_inode_metadata() {
    let dir = temp_test_dir("proc-exe-inode");
    create_fake_proc(&dir, 10, 1, "game", "KingdomCome.exe", &[10]);

    let snapshot = process_tree::target_snapshot(
        process_tree::TargetSnapshotInput::default()
            .proc_root(&dir)
            .manual_pids(&[])
            .tree_pids(&[10])
            .cgroup_path(None),
    );
    let task = snapshot.tasks.get(&10).unwrap();

    assert!(task.exe_dev.is_some());
    assert!(task.exe_ino.is_some());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn process_cache_invalidates_when_pid_starttime_changes() {
    let dir = temp_test_dir("proc-cache-starttime");
    create_fake_proc(&dir, 10, 1, "old-name", "old-name", &[10]);

    let mut cache = process_tree::ProcessCache::default();
    let first = process_tree::scan_processes_at(&dir, &mut cache);
    assert_eq!(first.get(&10).unwrap().comm, "old-name");

    fs::remove_dir_all(dir.join("10")).unwrap();
    // Recreate the process to simulate PID reuse.
    create_fake_proc(&dir, 10, 1, "new-name", "new-name", &[10]);
    // Manually overwrite stat to match the test's expected starttime.
    fs::write(dir.join("10/stat"), fake_stat("new-name", 999)).unwrap();

    let second = process_tree::scan_processes_at(&dir, &mut cache);
    assert_eq!(second.get(&10).unwrap().comm, "new-name");
    assert_eq!(second.get(&10).unwrap().starttime_ticks, Some(999));

    fs::remove_dir_all(dir).ok();
}

#[test]
fn process_cache_can_be_invalidated_for_exec_without_starttime_change() {
    let dir = temp_test_dir("proc-cache-exec");
    create_fake_proc(&dir, 10, 1, "launcher", "launcher", &[10]);

    let mut cache = process_tree::ProcessCache::default();
    let first = process_tree::scan_processes_at(&dir, &mut cache);
    assert_eq!(first.get(&10).unwrap().comm, "launcher");

    fs::write(dir.join("10/status"), "Name:\tgame\nPPid:\t1\n").unwrap();
    fs::write(dir.join("10/cmdline"), b"game.exe").unwrap();
    cache.invalidate(10);

    let second = process_tree::scan_processes_at(&dir, &mut cache);
    assert_eq!(second.get(&10).unwrap().comm, "game");
    assert_eq!(second.get(&10).unwrap().starttime_ticks, Some(100));

    fs::remove_dir_all(dir).ok();
}

#[test]
fn target_snapshot_reads_fresh_task_comm_with_previous_tasks() {
    let dir = temp_test_dir("proc-fresh-task-comm");
    create_fake_proc(&dir, 10, 1, "game", "game", &[10, 11]);

    let mut cache = process_tree::ProcessCache::default();
    let first = process_tree::target_snapshot(
        process_tree::TargetSnapshotInput::default()
            .proc_root(&dir)
            .tree_pids(&[10])
            .cache(&mut cache),
    );
    assert_eq!(first.tasks.get(&11).unwrap().comm, "game-11");

    fs::write(dir.join("10/task/11/comm"), "RenderThread\n").unwrap();

    let second = process_tree::target_snapshot(
        process_tree::TargetSnapshotInput::default()
            .proc_root(&dir)
            .tree_pids(&[10])
            .cache(&mut cache)
            .previous_tasks(Some(&first.tasks)),
    );
    assert_eq!(second.tasks.get(&11).unwrap().comm, "RenderThread");

    fs::remove_dir_all(dir).ok();
}

#[test]
fn cgroup_target_pid_collection_parses_sorts_and_dedups() {
    let dir = temp_test_dir("cgroup-target-pids");
    fs::create_dir_all(dir.join("child")).unwrap();
    fs::write(dir.join("cgroup.procs"), "30\nnot-a-pid\n10\n20\n").unwrap();
    fs::write(dir.join("cgroup.threads"), "40\n20\n").unwrap();
    fs::write(dir.join("child/cgroup.procs"), "50\n").unwrap();
    fs::write(dir.join("child/cgroup.threads"), "60\n").unwrap();

    let mut pids_set = std::collections::BTreeSet::from([1, 20]);
    process_tree::collect_cgroup_pids_at(&dir, &mut pids_set);
    let target_pids: Vec<_> = pids_set.into_iter().collect();

    assert_eq!(target_pids, vec![1, 10, 20, 30, 40, 50, 60]);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn proc_stat_starttime_handles_comm_with_parentheses() {
    let stat = "123 (name with ) paren) S 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 98765 0";

    assert_eq!(process_tree::parse_proc_stat_starttime(stat), Some(98_765));
}

#[test]
fn watch_process_selection_prefers_exact_then_executable_then_highest_pid() {
    let dir = temp_test_dir("watch-select");
    create_fake_proc(&dir, 10, 1, "helper", "/games/KingdomCome", &[10]);
    create_fake_proc(&dir, 20, 1, "KingdomCome", "/runtime/helper", &[20]);
    create_fake_proc(&dir, 30, 1, "other", "/bin/other KingdomCome", &[30]);

    assert_eq!(
        crate::watch::find_process_by_pattern_at(&dir, "KingdomCome"),
        Some(20)
    );

    fs::remove_dir_all(&dir).ok();

    let dir = temp_test_dir("watch-highest");
    create_fake_proc(&dir, 10, 1, "helper", "/bin/foo target", &[10]);
    create_fake_proc(&dir, 30, 1, "helper", "/bin/bar target", &[30]);

    assert_eq!(
        crate::watch::find_process_by_pattern_at(&dir, "target"),
        Some(30)
    );

    fs::remove_dir_all(dir).ok();
}

#[test]
fn target_snapshot_respects_exclude_tree_pids() {
    let dir = temp_test_dir("exclude-tree");
    // Root 100 -> children 101, 102. 102 -> child 103.
    create_fake_proc(&dir, 100, 1, "root", "root", &[100]);
    create_fake_proc(&dir, 101, 100, "child1", "child1", &[101]);
    create_fake_proc(&dir, 102, 100, "child2", "child2", &[102]);
    create_fake_proc(&dir, 103, 102, "child3", "child3", &[103]);

    let snapshot = process_tree::target_snapshot(
        process_tree::TargetSnapshotInput::default()
            .proc_root(&dir)
            .tree_pids(&[100])
            .exclude_tree_pids(&[102]),
    );

    assert!(snapshot.tasks.contains_key(&100));
    assert!(snapshot.tasks.contains_key(&101));
    assert!(!snapshot.tasks.contains_key(&102));
    assert!(!snapshot.tasks.contains_key(&103));
    assert_eq!(snapshot.process_roots, [100, 101].into_iter().collect());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn watch_process_selection_treats_wine_backslashes_as_path_separators() {
    let dir = temp_test_dir("watch-wine-path");
    create_fake_proc(
        &dir,
        10,
        1,
        "helper",
        r#"Z:\home\p2949\Games\KingdomCome.exe"#,
        &[10],
    );
    create_fake_proc(&dir, 20, 1, "other", "/bin/other KingdomCome.exe", &[20]);

    assert_eq!(
        crate::watch::find_process_by_pattern_at(&dir, "KingdomCome.exe"),
        Some(10)
    );

    fs::remove_dir_all(dir).ok();
}

#[test]
fn tree_root_starttime_change_is_stale() {
    let mut roots = BTreeMap::new();
    roots.insert(42, Some(100));

    let dir = temp_test_dir("root-starttime-stale");
    create_fake_proc(&dir, 42, 1, "game", "game", &[42]);

    let current = process_tree::process_starttime_at(&dir, 42);
    assert_eq!(current, Some(420));
    assert_ne!(roots.get(&42).copied().flatten(), current);

    fs::remove_dir_all(dir).ok();
}

#[test]
fn same_task_info_falls_back_to_conservative_metadata_without_starttimes() {
    let mut left = task_info(42, 100, "game", "worker", TaskClass::Game);
    let mut right = left.clone();
    left.process_starttime_ticks = None;
    left.task_starttime_ticks = None;
    right.process_starttime_ticks = None;
    right.task_starttime_ticks = None;

    // Provide exe info so it can fall back when starttimes are missing
    left.exe_dev = Some(1);
    left.exe_ino = Some(2);
    right.exe_dev = Some(1);
    right.exe_ino = Some(2);

    assert!(crate::tasks::same_task_info(&left, &right));

    right.process_comm = "other-game".into();
    assert!(!crate::tasks::same_task_info(&left, &right));
}

#[test]
fn old_session_task_without_starttime_fields_deserializes() {
    let task: recorder::SessionTask = serde_json::from_value(serde_json::json!({
        "task": 7,
        "active": true,
        "first_seen_ms": 0,
        "last_seen_ms": 1,
        "removed_ms": null,
        "class": "Game",
        "process_pid": 7,
        "process_comm": "game",
        "comm": "worker",
        "latency": {
            "samples": 1,
            "stored_samples": 1,
            "truncated_samples": 0,
            "percentile_scope": "exact",
            "histogram": [],
            "min_ns": 1,
            "avg_ns": 1,
            "p95_ns": 1,
            "p99_ns": 1,
            "max_ns": 1,
            "over_1ms": 0,
            "over_2ms": 0,
            "over_5ms": 0
        },
        "cpu": {
            "busiest_cpu": null,
            "busiest_cpu_samples": 0,
            "worst_cpu": null,
            "worst_cpu_max_ns": 0,
            "spikiest_cpu": null,
            "spikiest_cpu_spikes": 0,
            "per_cpu": []
        },
        "top_spikes": []
    }))
    .unwrap();

    assert_eq!(task.process_starttime_ticks, None);
    assert_eq!(task.task_starttime_ticks, None);
}

#[test]
fn recorded_time_accepts_legacy_local_field() {
    let recorded: recorder::RecordedTime = serde_json::from_str(
        r#"{"unix_seconds":0,"unix_nanos":0,"local":"SystemTime { tv_sec: 0, tv_nsec: 0 }"}"#,
    )
    .unwrap();

    assert_eq!(
        recorded.system_time_debug,
        "SystemTime { tv_sec: 0, tv_nsec: 0 }"
    );
}

#[test]
fn report_reads_recorded_session_and_spike_events() {
    let dir = temp_test_dir("report-smoke");
    fs::create_dir_all(&dir).unwrap();

    let recording = RecordingRun {
        run_name: Some("report-test".to_owned()),
        run_dir: dir.clone(),
        started_at: UNIX_EPOCH,
        started_instant: Instant::now(),
        monotonic_start_ns: Some(1_000_000_000),
        mangohud_start_offset: None,
        mangohud_first_frame_monotonic_ns: None,
        mangohud_first_frame_raw_elapsed_ms: None,
    };
    let config = test_config(vec![7], vec![], Some(Duration::from_secs(1)));
    let active_targets = BTreeMap::from([(
        7,
        task_info(7, 7, "KingdomCome.exe", "RenderThread", TaskClass::Game),
    )]);
    let mut stats = metrics::TaskStats::new(7, "RenderThread".to_owned(), 0);
    stats.apply_task_info(active_targets.get(&7).unwrap());
    stats.session_latency.record(6_000_000);
    stats.top_spikes.push(metrics::SpikeRecord {
        latency_ns: 6_000_000,
        cpu: 0,
        wakeup_target_cpu: 0,
        prio: 120,
        wakeup_ns: 1_010_000_000,
        switch_ns: 1_016_000_000,
        target_pending_wakeups: 0,
        observed_runnable_depth: 0,
        major_faults: 0,
        minor_faults: 0,
        scx_ops: None,
        scx_state: None,
        scx_enable_seq: None,
        cause_tags: Vec::new(),
        primary_cause: None,
    });
    let stats_by_task = BTreeMap::from([(7, stats)]);
    let spike_events = vec![SpikeEvent {
        elapsed_ms: Some(16),
        task: 7,
        active: true,
        class: TaskClass::Game,
        process_pid: Some(7),
        process_comm: "KingdomCome.exe".into(),
        comm: "RenderThread".into(),
        cpu: 0,
        wakeup_target_cpu: 0,
        prio: 120,
        latency_ns: 6_000_000,
        wakeup_ns: 1_010_000_000,
        switch_ns: 1_016_000_000,
        target_pending_wakeups: 0,
        observed_runnable_depth: 0,
        major_faults: 0,
        minor_faults: 0,
        scx_ops: None,
        scx_state: None,
        scx_enable_seq: None,
        cause_tags: Vec::new(),
        primary_cause: None,
    }];

    let mut task_tracker = tasks::TaskTracker::default();
    task_tracker.active_targets = active_targets;
    task_tracker.stats_by_task = stats_by_task;

    let mut recorder = recorder::LiveRecorder::default();
    recorder.run = Some(recording);
    let mut buffer = SpikeEventBuffer::default();
    for spike in spike_events {
        buffer.push(spike);
    }
    recorder.spike_events = Some(buffer);

    recorder::finalize_recording(FinalizeRecordingInput {
        recorder: &recorder,
        config: &config,
        tree_pids: &config.tree_pids,
        stop_reason: "test",
        tasks: &task_tracker,
        frame_events: &[],
        block_io_correlation_basis: "dev+sector",
        drop_counters: DropCountersSnapshot::default(),
        cpu_perf_status: None,
    })
    .unwrap();

    crate::report::print_report(&dir, false, false, false, 10, 5, None).unwrap();
    crate::report::print_report(&dir, true, false, false, 10, 5, None).unwrap();

    fs::remove_dir_all(dir).ok();
}

#[test]
fn report_cluster_output_caps_inline_points() {
    let dir = temp_test_dir("report-cluster-cap");
    fs::create_dir_all(&dir).unwrap();

    let recording = RecordingRun {
        run_name: Some("cluster-cap-test".to_owned()),
        run_dir: dir.clone(),
        started_at: UNIX_EPOCH,
        started_instant: Instant::now(),
        monotonic_start_ns: Some(1_000_000_000),
        mangohud_start_offset: None,
        mangohud_first_frame_monotonic_ns: None,
        mangohud_first_frame_raw_elapsed_ms: None,
    };

    let config = test_config(vec![7], vec![], Some(Duration::from_secs(1)));
    let active_targets: BTreeMap<u32, TaskInfo> = BTreeMap::new();
    let stats_by_task: BTreeMap<u32, metrics::TaskStats> = BTreeMap::new();

    let spike_events = (0..10)
        .map(|idx| SpikeEvent {
            elapsed_ms: Some(idx as u128),
            task: 100 + idx as u32,
            active: true,
            class: TaskClass::Helper,
            process_pid: Some(100 + idx as u32),
            process_comm: format!("proc-{}", idx).into(),
            comm: format!("worker-{}", idx),
            cpu: idx as u32 % 4,
            wakeup_target_cpu: idx as u32 % 4,
            prio: 120,
            latency_ns: 1_000_000 + idx as u64,
            wakeup_ns: 1_000_000_000 + idx as u64 * 100_000,
            switch_ns: 1_001_000_000 + idx as u64 * 100_000,
            target_pending_wakeups: 0,
            observed_runnable_depth: 0,
            major_faults: 0,
            minor_faults: 0,
            scx_ops: None,
            scx_state: None,
            scx_enable_seq: None,
            cause_tags: Vec::new(),
            primary_cause: None,
        })
        .collect::<Vec<_>>();

    let mut task_tracker = tasks::TaskTracker::default();
    task_tracker.active_targets = active_targets;
    task_tracker.stats_by_task = stats_by_task;

    let mut recorder = recorder::LiveRecorder::default();
    recorder.run = Some(recording);
    let mut buffer = SpikeEventBuffer::default();
    for spike in spike_events.iter().cloned() {
        buffer.push(spike);
    }
    recorder.spike_events = Some(buffer);

    recorder::finalize_recording(FinalizeRecordingInput {
        recorder: &recorder,
        config: &config,
        tree_pids: &config.tree_pids,
        stop_reason: "test",
        tasks: &task_tracker,
        frame_events: &[],
        block_io_correlation_basis: "dev+sector",
        drop_counters: DropCountersSnapshot::default(),
        cpu_perf_status: None,
    })
    .unwrap();

    let session = crate::session_io::load_session(&dir).unwrap();

    let cluster_analysis =
        crate::report::spike_cluster_analysis(&session, Some(&spike_events), 5_000_000, 10, None);
    let output = crate::report::render_report(
        &dir,
        &session,
        &cluster_analysis,
        &[],
        &crate::session_io::RunArtifacts::default(),
        10,
        5,
        None,
    );

    assert!(output.contains("total_spikes=10"));
    assert!(output.contains("shown_points=8"));
    assert!(output.contains("omitted_points=2"));
    assert!(output.contains("100("));
    assert!(output.contains("107("));
    assert!(!output.contains("108("));
    assert!(!output.contains("109("));

    fs::remove_dir_all(dir).ok();
}

#[test]
fn report_correlates_artifacts_with_spike_clusters() {
    let dir = temp_test_dir("report-correlation");
    fs::create_dir_all(&dir).unwrap();
    let session_path = dir.join("session.json");
    let session = minimal_session_for_report();
    let spike_events = (0..3)
        .map(|idx| SpikeEvent {
            elapsed_ms: Some(10 + idx as u128),
            task: 10 + idx as u32,
            active: true,
            class: TaskClass::Game,
            process_pid: Some(10 + idx as u32),
            process_comm: "game".to_owned().into(),
            comm: if idx == 0 {
                "RenderThread".to_owned()
            } else {
                format!("worker-{}", idx)
            },
            cpu: idx as u32,
            wakeup_target_cpu: idx as u32,
            prio: 120,
            latency_ns: 1_000_000,
            wakeup_ns: 1_000_000 + idx as u64 * 100,
            switch_ns: 10_000_000 + idx as u64 * 100,
            target_pending_wakeups: 0,
            observed_runnable_depth: 0,
            major_faults: 0,
            minor_faults: 0,
            scx_ops: None,
            scx_state: None,
            scx_enable_seq: None,
            cause_tags: Vec::new(),
            primary_cause: None,
        })
        .collect::<Vec<_>>();
    let artifacts = crate::session_io::RunArtifacts {
        scx_events: Vec::new(),
        irq_events: vec![IrqEventRecord {
            elapsed_ms: Some(10),
            irq: 137,
            cpu: 0,
            enter_ns: 9_999_900,
            exit_ns: 10_000_200,
            duration_ns: 300,
        }],
        gpu_samples: vec![GpuSample {
            elapsed_ms: 11,
            gpu_busy_percent: Some(91),
            vram_used_bytes: None,
            vram_total_bytes: None,
            vram_used_percent: None,
            gpu_clock_mhz: Some(2200),
            mem_clock_mhz: Some(1000),
            temp_millidegrees: Some(61000),
            power_microwatts: Some(120_000_000),
        }],
        frame_events: vec![FrameEvent {
            elapsed_ms: 11,
            frametime_ms: 22.5,
        }],
        migration_events: Vec::new(),
        cpu_freq_events: Vec::new(),
        block_io_events: Vec::new(),
        intervals: Vec::new(),
        ..Default::default()
    };

    let cluster_analysis =
        crate::report::spike_cluster_analysis(&session, Some(&spike_events), 5_000_000, 10, None);
    let output = crate::report::render_report(
        &session_path,
        &session,
        &cluster_analysis,
        &[],
        &artifacts,
        10,
        5,
        None,
    );

    assert!(output.contains("irq overlap"));
    assert!(output.contains("irqs=137"));
    assert!(output.contains("gpu near clusters"));
    assert!(output.contains("gpu_busy=91"));
    assert!(output.contains("frame overlap"));
    assert!(output.contains("max_frametime_ms=22.500"));

    fs::remove_dir_all(dir).ok();
}

#[test]
fn report_uses_run_level_block_io_correlation_basis() {
    let dir = temp_test_dir("report-block-io-basis");
    fs::create_dir_all(&dir).unwrap();
    let session_path = dir.join("session.json");
    let mut session = minimal_session_for_report();
    session.block_io_event_count = 1;
    session.block_io_correlation_basis = "request-pointer".to_owned();

    let cluster_analysis =
        crate::report::spike_cluster_analysis(&session, None, 5_000_000, 10, None);
    let output = crate::report::render_report(
        &session_path,
        &session,
        &cluster_analysis,
        &[],
        &crate::session_io::RunArtifacts::default(),
        10,
        5,
        None,
    );

    assert!(output.contains("io_events: 1 (request-pointer correlated)"));
    assert!(!output.contains("block i/o correlation warning"));

    session.block_io_correlation_basis = "dev+sector".to_owned();
    let output = crate::report::render_report(
        &session_path,
        &session,
        &cluster_analysis,
        &[],
        &crate::session_io::RunArtifacts::default(),
        10,
        5,
        None,
    );
    assert!(output.contains("io_events: 1 (dev+sector correlated (advisory, approximate))"));
    assert!(output.contains("block i/o correlation warning"));

    fs::remove_dir_all(dir).ok();
}

#[test]
fn tune_counts_only_scored_post_warmup_records() {
    let records = vec![
        interval_record(10, TaskClass::Helper, 200, "helper"),
        interval_record(11, TaskClass::Compositor, 100, "compositor"),
    ];

    assert_eq!(crate::tune::tune_scored_record_counts(&records), (0, 0));

    let mut records = records;
    records.push(interval_record(12, TaskClass::Game, 55, "game"));

    assert_eq!(crate::tune::tune_scored_record_counts(&records), (1, 55));
}

#[test]
fn tune_coverage_counts_duplicate_scored_thread_identities() {
    let mut session = minimal_session_for_report();
    session.tasks = vec![
        session_task(10, 100, TaskClass::Game, "worker", Some(1000), Some(10)),
        session_task(11, 100, TaskClass::Game, "worker", Some(1000), Some(11)),
        session_task(12, 100, TaskClass::Game, "worker", Some(1000), Some(12)),
    ];
    let intervals = vec![
        interval_record(10, TaskClass::Game, 10, "worker"),
        interval_record(11, TaskClass::Game, 10, "worker"),
        interval_record(12, TaskClass::Game, 10, "worker"),
    ];

    let coverage = tune::comparability::tune_coverage_metrics(&session, &intervals);

    assert_eq!(coverage.unique_scored_tasks, 3);
    assert_eq!(
        coverage
            .scored_identity_counts
            .iter()
            .map(|c| c.count)
            .sum::<usize>(),
        3
    );
    assert_eq!(coverage.scored_identity_counts.len(), 3);
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
    config.recording = Some(RecordingConfig {
        run_name: Some("collision-test".to_owned()),
        out_dir: Some(dir.clone()),
    });

    let err = recorder::prepare_recording(&config).unwrap_err();

    assert!(format!("{err:#}").contains("output directory already exists"));

    fs::remove_dir_all(dir).ok();
}

fn test_config(
    target_pids: Vec<u32>,
    tree_pids: Vec<u32>,
    max_duration: Option<Duration>,
) -> Config {
    Config {
        target_pids,
        tree_pids,
        summary_period_ms: 1_000,
        epoch_period_ms: None,
        spike_threshold_ns: 1_000_000,
        alert_threshold_ns: None,
        alert_webhook_url: None,
        verbose: false,
        max_tasks: 1024,
        task_filters: process_tree::TaskFilters::default(),
        keep_missing_pid: false,
        watch_process: None,
        persistent: false,
        watch_poll_ms: 2_000,
        watch_timeout: None,
        csv_path: None,
        irq_latency: false,
        irqs: Vec::new(),
        hwmon: false,
        hwmon_root: None,
        hwmon_drm_card: None,
        hwmon_render_node: None,
        mangohud_log: None,
        tui: false,
        retain_intervals: None,
        recording: None,
        max_duration,
        cgroupv2: None,
        native_cgroup_filter: false,
        follow_exec: true,
        exclude_tree_pids: Vec::new(),
        cpu_freq: false,
        faults: false,
        cpu_perf: false,
        cpu_perf_kernel: false,
        cpu_perf_max_tasks: 128,
        cpu_perf_cache_refs: false,
        block_io: false,
        stat_wait: false,
        json_stream: false,
        mangohud_log_live: false,
        metrics_port: None,
    }
}

fn minimal_session_for_report() -> SessionFile {
    let mut config = test_config(vec![], vec![], None);
    config.irq_latency = true;
    config.irqs = vec![137];
    config.hwmon = true;

    SessionFile {
        schema_version: SESSION_SCHEMA_VERSION,
        run_name: Some("report-correlation".to_owned()),
        started_at: recorded_time(UNIX_EPOCH),
        ended_at: recorded_time(UNIX_EPOCH),
        monotonic_start_ns: Some(0),
        mangohud_start_offset: None,
        mangohud_first_frame_monotonic_ns: None,
        mangohud_first_frame_raw_elapsed_ms: None,
        monotonic_end_ns: Some(20_000_000),
        duration_ms: 20,
        stop_reason: "test".to_owned(),
        config: recorded_config(&config, &config.tree_pids),
        metadata: SystemMetadata::default(),
        target_pids_max: 1024,
        active_target_pids_count: 0,
        active_expanded_tasks: Vec::new(),
        spike_events_retained_count: 3,
        spike_events_dropped_count: 0,
        spike_events_truncated: false,
        scx_event_count: 0,
        irq_event_count: 1,
        migration_event_count: Some(0),
        cpu_freq_sample_count: Some(0),
        gpu_sample_count: 1,
        frame_event_count: 1,
        block_io_event_count: 0,
        event_stream_write_errors: 0,
        alert_events_dropped_count: 0,
        alert_channel_closed_count: 0,
        first_event_stream_write_error: None,
        block_io_correlation_basis: "dev+sector".to_owned(),
        drop_counters: DropCountersSnapshot::default(),
        interval_record_count: 0,
        intervals_dropped: 0,
        cpu_perf_sample_count: 0,
        cpu_perf_open_errors: 0,
        cpu_perf_read_errors: 0,
        cpu_perf_skipped_tasks: 0,
        cpu_perf_last_error: None,
        tasks: Vec::new(),
        top_spikes: Vec::new(),
    }
}

fn session_task(
    tid: u32,
    process_pid: u32,
    class: TaskClass,
    comm: &str,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
) -> SessionTask {
    SessionTask {
        task: tid,
        active: true,
        first_seen_ms: 0,
        last_seen_ms: 100,
        removed_ms: None,
        class,
        process_pid: Some(process_pid),
        process_comm: "game".into(),
        process_starttime_ticks,
        task_starttime_ticks,
        exe_dev: Some(1),
        exe_ino: Some(2),
        comm: comm.to_owned(),
        latency: RecordedLatency {
            samples: 1,
            stored_samples: 1,
            truncated_samples: 0,
            percentile_scope: "histogram".to_owned(),
            histogram: Vec::new(),
            min_ns: 1,
            avg_ns: 1,
            p95_ns: 1,
            p99_ns: 1,
            max_ns: 1,
            over_1ms: 0,
            over_2ms: 0,
            over_5ms: 0,
        },
        cpu: RecordedCpuSnapshot {
            busiest_cpu: None,
            busiest_cpu_samples: 0,
            worst_cpu: None,
            worst_cpu_max_ns: 0,
            spikiest_cpu: None,
            spikiest_cpu_spikes: 0,
            per_cpu: Vec::new(),
        },
        top_spikes: Vec::new(),
        migration_count: 0,
        cross_numa_migrations: 0,
        top_wakers: Vec::new(),
        sched_policy: None,
        stat_wait_sum_ns: None,
        stat_wait_count: None,
        cpu_perf: None,
    }
}

fn interval_record(
    task: u32,
    class: TaskClass,
    samples: u64,
    comm: &str,
) -> metrics::IntervalRecord {
    metrics::IntervalRecord {
        elapsed_ms: 1_000,
        task,
        active: true,
        class,
        comm: comm.to_owned(),
        process_pid: Some(100),
        process_comm: "game".into(),
        samples,
        stored_samples: samples,
        truncated_samples: 0,
        min_ns: 0,
        avg_ns: 0,
        p95_ns: 0,
        p99_ns: 0,
        max_ns: 0,
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
        major_faults: 0,
        minor_faults: 0,
        percentile_scope: "histogram".to_owned(),
        histogram: Vec::new(),
        drop_counters: DropCountersSnapshot::default(),
        cpu_perf: None,
    }
}

fn task_stats_with_info(
    tid: u32,
    process_pid: u32,
    process_comm: &str,
    comm: &str,
    class: TaskClass,
    first_seen_ms: u128,
) -> metrics::TaskStats {
    let mut stats = metrics::TaskStats::new(tid, comm.to_owned(), first_seen_ms);
    stats.apply_task_info(&task_info(tid, process_pid, process_comm, comm, class));
    stats
}

fn task_info(
    tid: u32,
    process_pid: u32,
    process_comm: &str,
    comm: &str,
    class: TaskClass,
) -> TaskInfo {
    TaskInfo {
        tid,
        process_pid,
        process_ppid: 1,
        comm: comm.into(),
        process_comm: process_comm.into(),
        process_starttime_ticks: Some(u64::from(process_pid) * 10),
        task_starttime_ticks: Some(u64::from(tid) * 10),
        exe_dev: None,
        exe_ino: None,
        class,
        sched_policy: None,
        from_cgroup: false,
    }
}

fn scheduler_event(pid: u32, comm: &str) -> SchedulerEvent {
    scheduler_event_with_latency(pid, comm, 10)
}

fn scheduler_event_with_latency(pid: u32, comm: &str, latency_ns: u64) -> SchedulerEvent {
    let mut comm_bytes = [0; 16];
    for (idx, byte) in comm.as_bytes().iter().take(15).enumerate() {
        comm_bytes[idx] = *byte;
    }

    SchedulerEvent {
        kind: EVENT_RUNNABLE_LATENCY,
        pid,
        cpu: 0,
        wakeup_target_cpu: 0,
        prio: 120,
        wakeup_ns: 100,
        switch_ns: 100 + latency_ns,
        latency_ns,
        comm: comm_bytes,
        waker_tid: 0,
        target_pending_wakeups: 0,
        observed_runnable_depth: 0,
        maj_flt: 0,
        min_flt: 0,
    }
}

fn spike_event(task: u32, switch_ns: u64) -> SpikeEvent {
    SpikeEvent {
        elapsed_ms: Some(u128::from(switch_ns / 1_000_000)),
        task,
        active: true,
        class: TaskClass::Game,
        process_pid: Some(task),
        process_comm: "game".into(),
        comm: "game".to_owned(),
        cpu: 0,
        wakeup_target_cpu: 0,
        prio: 120,
        latency_ns: 1_000_000,
        wakeup_ns: switch_ns.saturating_sub(1_000_000),
        switch_ns,
        target_pending_wakeups: 0,
        observed_runnable_depth: 0,
        major_faults: 0,
        minor_faults: 0,
        scx_ops: None,
        scx_state: None,
        scx_enable_seq: None,
        cause_tags: Vec::new(),
        primary_cause: None,
    }
}

#[test]
fn report_diff_shows_regressions_and_improvements() {
    let dir_a = temp_test_dir("diff-a");
    let dir_b = temp_test_dir("diff-b");
    fs::create_dir_all(&dir_a).unwrap();
    fs::create_dir_all(&dir_b).unwrap();

    let session_a_json = r#"{
        "schema_version": 2,
        "run_name": "run-a",
        "duration_ms": 10000,
        "metadata": {
            "kernel_osrelease": null,
            "kernel_version": null,
            "cpu_online": null,
            "cpu_possible": null,
            "cpu_topology": [],
            "scx_state": null,
            "scx_ops": null,
            "scx_enable_seq": null
        },
        "monotonic_start_ns": 0,
        "started_at": {
            "unix_seconds": 0,
            "unix_nanos": 0,
            "system_time_debug": "test"
        },
        "ended_at": {
            "unix_seconds": 0,
            "unix_nanos": 0,
            "system_time_debug": "test"
        },
        "target_pids_max": 2048,
        "stop_reason": "test",
        "active_target_pids_count": 2,
        "active_expanded_tasks": [],
        "total_targets_tracked": 0,
        "total_events_processed": 0,
        "total_tasks_seen": 0,
        "interval_record_count": 0,
        "intervals_dropped": 0,
        "config": {
            "tree_roots": [],
            "manual_pids": [],
            "include_comm": [],
            "exclude_comm": [],
            "hwmon": false,
            "hwmon_device_prefix": null,
            "hwmon_drm_card": null,
            "hwmon_render_node": null,
            "watch_process": null,
            "watch_process_args": null,
            "persistent": false,
            "csv_path": null,
            "tui": false,
            "summary_period_ms": 1000,
            "spike_threshold_ns": 5000000,
            "verbose": false
        },
        "tasks": [
            {
                "task": 1,
                "active": true,
                "first_seen_ms": 0,
                "last_seen_ms": 0,
                "removed_ms": null,
                "class": "Game",
                "process_pid": 1,
                "process_comm": "game",
                "comm": "game-thread",
                "latency": {
                    "samples": 100,
                    "stored_samples": 100,
                    "truncated_samples": 0,
                    "percentile_scope": "session",
                    "histogram": [],
                    "min_ns": 100000,
                    "avg_ns": 500000,
                    "p95_ns": 1000000,
                    "p99_ns": 2000000,
                    "max_ns": 5000000,
                    "over_1ms": 10,
                    "over_2ms": 5,
                    "over_5ms": 0
                },
                "cpu": {
                    "busiest_cpu": null,
                    "busiest_cpu_samples": 0,
                    "worst_cpu": null,
                    "worst_cpu_max_ns": 0,
                    "spikiest_cpu": null,
                    "spikiest_cpu_spikes": 0,
                    "per_cpu": []
                },
                "top_spikes": []
            },
            {
                "task": 2,
                "active": true,
                "first_seen_ms": 0,
                "last_seen_ms": 0,
                "removed_ms": null,
                "class": "Helper",
                "process_pid": 1,
                "process_comm": "game",
                "comm": "helper-thread",
                "latency": {
                    "samples": 100,
                    "stored_samples": 100,
                    "truncated_samples": 0,
                    "percentile_scope": "session",
                    "histogram": [],
                    "min_ns": 100000,
                    "avg_ns": 500000,
                    "p95_ns": 1000000,
                    "p99_ns": 2000000,
                    "max_ns": 6000000,
                    "over_1ms": 15,
                    "over_2ms": 5,
                    "over_5ms": 2
                },
                "cpu": {
                    "busiest_cpu": null,
                    "busiest_cpu_samples": 0,
                    "worst_cpu": null,
                    "worst_cpu_max_ns": 0,
                    "spikiest_cpu": null,
                    "spikiest_cpu_spikes": 0,
                    "per_cpu": []
                },
                "top_spikes": []
            }
        ],
        "top_spikes": [],
        "spike_events_retained_count": 0,
        "spike_events_dropped_count": 0,
        "spike_events_truncated": false,
        "drop_counters": {
            "sched_switch": 0,
            "sched_wakeup": 0,
            "scx_runnable": 0,
            "scx_consume": 0,
            "timer_expire_entry": 0,
            "irq_handler_entry": 0,
            "gpu_drm_sched_job": 0,
            "gpu_dma_fence_signaled": 0,
            "sys_enter_read": 0,
            "sys_enter_write": 0,
            "wakeup_data_insert_failed": 0,
            "ringbuf_reserve_failed": 0,
            "irq_start_times_insert_failed": 0,
            "block_start_insert_failed": 0
        },
        "scx_event_count": 0,
        "irq_event_count": 0,
        "migration_event_count": 0,
        "cpu_freq_sample_count": 0,
        "gpu_sample_count": 0,
        "frame_event_count": 0,
        "block_io_event_count": 0,
        "block_io_correlation_basis": "dev+sector"
    }"#;

    let session_b_json = session_a_json
        .replace("\"run-a\"", "\"run-b\"")
        // Game thread max 5ms -> 8ms
        .replace("\"max_ns\": 5000000", "\"max_ns\": 8000000")
        // Game thread over_1ms 10 -> 8
        .replace("\"over_1ms\": 10", "\"over_1ms\": 8")
        // Game thread p99 2ms -> 2.5ms
        .replace("\"p99_ns\": 2000000", "\"p99_ns\": 2500000")
        // Helper thread max 6ms -> 4ms
        .replace("\"max_ns\": 6000000", "\"max_ns\": 4000000");

    fs::write(dir_a.join("session.json"), session_a_json).unwrap();
    fs::write(dir_b.join("session.json"), session_b_json).unwrap();

    let output = crate::report::render_diff_report(&dir_a, &dir_b, 10, None).unwrap();
    println!("DEBUG OUTPUT:\n{}", output);

    assert!(output.contains("regressions"));
    assert!(output.contains("improvements"));
    // Game thread regressed max latency
    assert!(output.contains("max: 5.000ms -> 8.000ms (delta=+3.000ms)"));
    assert!(output.contains("p99_delta=+500.000us"));
    assert!(output.contains("over_1ms_delta=-2"));

    // Helper thread improved max latency
    assert!(output.contains("max: 6.000ms -> 4.000ms (delta=-2.000ms)"));

    // Now test with filter-class
    let output_filtered =
        crate::report::render_diff_report(&dir_a, &dir_b, 10, Some(TaskClass::Game)).unwrap();
    assert!(output_filtered.contains("game-thread"));
    assert!(!output_filtered.contains("helper-thread"));

    fs::remove_dir_all(dir_a).ok();
    fs::remove_dir_all(dir_b).ok();
}

fn temp_test_dir(name: &str) -> std::path::PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    dir
}

fn create_fake_proc(
    proc_root: &Path,
    pid: u32,
    ppid: u32,
    name: &str,
    cmdline: &str,
    tids: &[u32],
) {
    let proc_dir = proc_root.join(pid.to_string());
    fs::create_dir_all(proc_dir.join("task")).unwrap();
    fs::write(
        proc_dir.join("status"),
        format!("Name:\t{name}\nPPid:\t{ppid}\n"),
    )
    .unwrap();
    fs::write(proc_dir.join("cmdline"), cmdline.as_bytes()).unwrap();
    fs::write(proc_dir.join("stat"), fake_stat(name, u64::from(pid) * 10)).unwrap();
    fs::write(proc_dir.join("exe"), format!("{name}\n")).unwrap();

    for tid in tids {
        let task_dir = proc_dir.join("task").join(tid.to_string());
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(task_dir.join("comm"), format!("{name}-{tid}\n")).unwrap();
        fs::write(task_dir.join("stat"), fake_stat(name, u64::from(*tid) * 10)).unwrap();
    }
}

fn fake_stat(comm: &str, starttime: u64) -> String {
    let mut fields = vec!["0".to_owned(); 18];
    fields.push(starttime.to_string());
    format!("1 ({comm}) S {}\n", fields.join(" "))
}

#[test]
fn scx_correlation_spike_event_serialization() {
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
        observed_runnable_depth: 0,
        major_faults: 1,
        minor_faults: 2,
        scx_ops: Some("scx_lavd".to_owned()),
        scx_state: Some("enabled".to_owned()),
        scx_enable_seq: Some("1".to_owned()),
        cause_tags: Vec::new(),
        primary_cause: None,
    };

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("\"scx_ops\":\"scx_lavd\""));
    assert!(json.contains("\"scx_state\":\"enabled\""));

    let deserialized: SpikeEvent = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.scx_ops.as_deref(), Some("scx_lavd"));
    assert_eq!(deserialized.scx_state.as_deref(), Some("enabled"));
    assert_eq!(deserialized.scx_enable_seq.as_deref(), Some("1"));
}

#[test]
fn scx_correlation_backward_compatibility() {
    let json = r#"{"elapsed_ms":100,"task":123,"active":true,"class":"Game","process_pid":123,"process_comm":"game","comm":"game-main","cpu":1,"wakeup_target_cpu":1,"prio":120,"latency_ns":5000000,"wakeup_ns":100,"switch_ns":5000100,"target_pending_wakeups":0,"major_faults":0,"minor_faults":0}"#;
    let deserialized: SpikeEvent = serde_json::from_str(json).unwrap();
    assert_eq!(deserialized.scx_ops, None);
    assert_eq!(deserialized.scx_state, None);
    assert_eq!(deserialized.scx_enable_seq, None);
}

#[test]
fn spike_event_stream_writes_ndjson() {
    let dir = temp_test_dir("spike-stream");
    fs::create_dir_all(&dir).unwrap();
    let spike_path = dir.join("spike_events.json");

    let mut recorder = recorder::LiveRecorder::default();
    recorder.spike_event_writer =
        Some(recorder::JsonArrayWriter::create(spike_path.clone()).unwrap());

    let spike1 = spike_event(1, 1000);
    let spike2 = spike_event(2, 2000);

    events::push_ndjson_event(
        recorder.spike_event_writer.as_mut().unwrap(),
        &spike1,
        &mut recorder.spike_event_count,
        &mut recorder.event_stream_write_errors,
        &mut recorder.first_event_stream_write_error,
        "spike_events",
    );

    events::push_ndjson_event(
        recorder.spike_event_writer.as_mut().unwrap(),
        &spike2,
        &mut recorder.spike_event_count,
        &mut recorder.event_stream_write_errors,
        &mut recorder.first_event_stream_write_error,
        "spike_events",
    );

    // Drop the recorder to finish the writer
    drop(recorder);

    let contents = fs::read_to_string(&spike_path).unwrap();
    let lines: Vec<_> = contents.lines().collect();
    assert_eq!(lines.len(), 2);

    for line in lines {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(value.is_object());
    }

    assert!(!contents.trim_start().starts_with('['));

    fs::remove_dir_all(dir).ok();
}

#[test]
fn load_run_artifacts_loads_streamed_spikes() {
    let dir = temp_test_dir("load-spikes");
    fs::create_dir_all(&dir).unwrap();

    let spike1 = spike_event(1, 1000);
    let spike2 = spike_event(2, 2000);

    let mut session = SessionFile::default();
    session.schema_version = SESSION_SCHEMA_VERSION;
    session.spike_events_retained_count = 2;
    fs::write(
        dir.join("session.json"),
        serde_json::to_string(&session).unwrap(),
    )
    .unwrap();

    let mut file = fs::File::create(dir.join("spike_events.json")).unwrap();
    use std::io::Write;
    writeln!(file, "{}", serde_json::to_string(&spike1).unwrap()).unwrap();
    writeln!(file, "{}", serde_json::to_string(&spike2).unwrap()).unwrap();
    drop(file);

    let artifacts =
        crate::session_io::load_run_artifacts(&dir, crate::session_io::ArtifactLoadOptions::REPORT)
            .unwrap();

    assert_eq!(artifacts.spikes.len(), 2);
    assert_eq!(artifacts.spikes[0].task, spike1.task);
    assert_eq!(artifacts.spikes[1].task, spike2.task);

    fs::remove_dir_all(dir).ok();
}

#[test]
fn load_run_artifacts_falls_back_to_top_spikes() {
    let dir = temp_test_dir("fallback-spikes");
    fs::create_dir_all(&dir).unwrap();

    let spike1 = recorder::SessionSpike {
        task: 1,
        latency_ns: 1000,
        ..Default::default()
    };

    let mut session = SessionFile::default();
    session.schema_version = SESSION_SCHEMA_VERSION;
    session.top_spikes = vec![spike1.clone()];
    session.spike_events_retained_count = 1;
    fs::write(
        dir.join("session.json"),
        serde_json::to_string(&session).unwrap(),
    )
    .unwrap();

    // No spike_events.json

    let artifacts =
        crate::session_io::load_run_artifacts(&dir, crate::session_io::ArtifactLoadOptions::REPORT)
            .unwrap();

    assert_eq!(artifacts.spikes.len(), 1);
    assert_eq!(artifacts.spikes[0].task, spike1.task);
    assert_eq!(artifacts.spikes[0].latency_ns, spike1.latency_ns);

    fs::remove_dir_all(dir).ok();
}
