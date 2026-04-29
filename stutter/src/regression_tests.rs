use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use stutter_common::{EVENT_RUNNABLE_LATENCY, SchedulerEvent};

use crate::{
    cli::{Config, RecordingConfig},
    ebpf_loader::DropCountersSnapshot,
    metrics,
    process_tree::{self, TargetDiffAction, TaskClass, TaskInfo},
    recorder::{
        self, FinalizeRecordingInput, RecordingRun, SESSION_SCHEMA_VERSION, SessionFile,
        SpikeEvent, SpikeEventBuffer,
    },
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
    super::reactivate_or_reset_stats(&mut stats_by_task, 42, &new_task, 77);

    let stats = stats_by_task.get(&42).unwrap();
    assert_eq!(stats.first_seen_ms, 77);
    assert_eq!(stats.last_seen_ms, 77);
    assert_eq!(stats.session_latency.count, 0);
    assert_eq!(stats.process_pid, Some(200));
    assert_eq!(stats.process_comm, "new-game");
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
    let mut known_targets = active_targets.clone();

    let mut stats = task_stats_with_info(42, 100, "old-game", "old-worker", TaskClass::Game, 10);
    stats.session_latency.record(5_000_000);
    let mut stats_by_task = BTreeMap::from([(42, stats)]);

    let mut tree_events = Vec::new();
    super::handle_same_tid_replacements(
        &active_targets,
        &desired_tasks,
        &mut known_targets,
        &mut stats_by_task,
        &mut tree_events,
        77,
        Some(Instant::now()),
    );

    let stats = stats_by_task.get(&42).unwrap();
    assert_eq!(stats.first_seen_ms, 77);
    assert_eq!(stats.last_seen_ms, 77);
    assert_eq!(stats.session_latency.count, 0);
    assert_eq!(stats.process_pid, Some(200));
    assert_eq!(stats.process_comm, "new-helper");
    assert_eq!(stats.comm, "new-worker");
    assert_eq!(stats.class, TaskClass::Helper);
    assert!(stats.active);

    assert_eq!(known_targets.get(&42), Some(&new_task));
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
    super::reactivate_or_reset_stats(&mut stats_by_task, 42, &same_task, 77);

    let stats = stats_by_task.get(&42).unwrap();
    assert_eq!(stats.first_seen_ms, 10);
    assert_eq!(stats.session_latency.count, 1);
    assert_eq!(stats.removed_ms, None);
    assert!(stats.active);
}

#[test]
fn event_comm_updates_only_unknown_existing_name() {
    let config = test_config(vec![7], vec![], None);
    let active_targets = BTreeMap::new();
    let known_targets = BTreeMap::new();
    let mut stats_by_task = BTreeMap::from([(7, metrics::TaskStats::new(7, "?".to_owned(), 0))]);

    super::handle_event(super::HandleEventInput {
        event: scheduler_event(7, "real-name"),
        config: &config,
        started: Instant::now(),
        active_targets: &active_targets,
        known_targets: &known_targets,
        stats_by_task: &mut stats_by_task,
        monotonic_start_ns: None,
        spike_events: None,
    });

    assert_eq!(stats_by_task.get(&7).unwrap().comm, "real-name");

    super::handle_event(super::HandleEventInput {
        event: scheduler_event(7, "later-name"),
        config: &config,
        started: Instant::now(),
        active_targets: &active_targets,
        known_targets: &known_targets,
        stats_by_task: &mut stats_by_task,
        monotonic_start_ns: None,
        spike_events: None,
    });

    assert_eq!(stats_by_task.get(&7).unwrap().comm, "real-name");
}

#[test]
fn recording_spike_events_capture_only_threshold_crossing_events() {
    let config = test_config(vec![7], vec![], None);
    let active_targets = BTreeMap::from([(
        7,
        task_info(7, 77, "KingdomCome.exe", "RenderThread", TaskClass::Game),
    )]);
    let known_targets = BTreeMap::new();
    let mut stats_by_task = BTreeMap::new();
    let mut spike_events = SpikeEventBuffer::default();

    super::handle_event(super::HandleEventInput {
        event: scheduler_event_with_latency(7, "RenderThread", 999_999),
        config: &config,
        started: Instant::now(),
        active_targets: &active_targets,
        known_targets: &known_targets,
        stats_by_task: &mut stats_by_task,
        monotonic_start_ns: Some(100),
        spike_events: Some(&mut spike_events),
    });
    assert!(spike_events.as_slice().is_empty());

    super::handle_event(super::HandleEventInput {
        event: scheduler_event_with_latency(7, "RenderThread", 1_000_000),
        config: &config,
        started: Instant::now(),
        active_targets: &active_targets,
        known_targets: &known_targets,
        stats_by_task: &mut stats_by_task,
        monotonic_start_ns: Some(100),
        spike_events: Some(&mut spike_events),
    });

    assert_eq!(spike_events.as_slice().len(), 1);
    let spike = &spike_events.as_slice()[0];
    assert_eq!(spike.task, 7);
    assert!(spike.active);
    assert_eq!(spike.class, TaskClass::Game);
    assert_eq!(spike.process_pid, Some(77));
    assert_eq!(spike.process_comm, "KingdomCome.exe");
    assert_eq!(spike.comm, "RenderThread");
    assert_eq!(spike.cpu, 0);
    assert_eq!(spike.prio, 120);
    assert_eq!(spike.latency_ns, 1_000_000);
    assert_eq!(spike.wakeup_ns, 100);
    assert_eq!(spike.switch_ns, 1_000_100);
    assert_eq!(spike.elapsed_ms, Some(1));
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

    let diffs = process_tree::diff_tasks(&old_tasks, &new_tasks);

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
fn target_snapshot_adds_fallback_without_o_n_squared_duplicate_scan_behavior() {
    let dir = temp_test_dir("proc-snapshot-fallback");
    create_fake_proc(&dir, 10, 1, "root", "root", &[10]);
    create_fake_proc(&dir, 11, 10, "child", "child", &[]);

    let snapshot = process_tree::target_snapshot_at(&dir, &[], &[10]);

    assert!(snapshot.process_roots.contains(&10));
    assert!(snapshot.process_roots.contains(&11));
    assert!(snapshot.tasks.contains_key(&10));
    assert!(snapshot.tasks.contains_key(&11));
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
    };
    let config = test_config(vec![9, 1, 4], vec![], Some(Duration::from_secs(1)));
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
    let drop_counters = DropCountersSnapshot {
        wakeup_times_insert_failed: 2,
        ringbuf_reserve_failed: 3,
    };

    recorder::finalize_recording(FinalizeRecordingInput {
        recording: &recording,
        config: &config,
        stop_reason: "test",
        active_targets: &active_targets,
        stats_by_task: &stats_by_task,
        interval_records: &interval_records,
        tree_events: &tree_events,
        spike_events: &spike_events,
        spike_events_truncated: true,
        drop_counters,
    })
    .unwrap();

    let session: SessionFile =
        serde_json::from_str(&fs::read_to_string(dir.join("session.json")).unwrap()).unwrap();
    let metadata: recorder::MetadataFile =
        serde_json::from_str(&fs::read_to_string(dir.join("metadata.json")).unwrap()).unwrap();
    let recorded_spike_events: Vec<SpikeEvent> =
        serde_json::from_str(&fs::read_to_string(dir.join("spike_events.json")).unwrap()).unwrap();

    assert_eq!(session.schema_version, SESSION_SCHEMA_VERSION);
    assert_eq!(session.active_expanded_tasks, vec![1, 4, 9]);
    assert_eq!(metadata.active_expanded_tasks, vec![1, 4, 9]);
    assert_eq!(session.spike_event_count, 1);
    assert_eq!(metadata.spike_event_count, 1);
    assert!(session.spike_events_truncated);
    assert!(metadata.spike_events_truncated);
    assert_eq!(session.drop_counters.total(), 5);
    assert_eq!(metadata.drop_counters.total(), 5);
    assert_eq!(session.drop_counters.wakeup_times_insert_failed, 2);
    assert_eq!(session.drop_counters.ringbuf_reserve_failed, 3);
    assert_eq!(recorded_spike_events.len(), 1);
    assert_eq!(recorded_spike_events[0].task, 7);
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
fn report_reads_recorded_session_and_spike_events() {
    let dir = temp_test_dir("report-smoke");
    fs::create_dir_all(&dir).unwrap();

    let recording = RecordingRun {
        run_name: Some("report-test".to_owned()),
        run_dir: dir.clone(),
        started_at: UNIX_EPOCH,
        started_instant: Instant::now(),
        monotonic_start_ns: Some(1_000_000_000),
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
        prio: 120,
        wakeup_ns: 1_010_000_000,
        switch_ns: 1_016_000_000,
    });
    let stats_by_task = BTreeMap::from([(7, stats)]);
    let spike_events = vec![SpikeEvent {
        elapsed_ms: Some(16),
        task: 7,
        active: true,
        class: TaskClass::Game,
        process_pid: Some(7),
        process_comm: "KingdomCome.exe".to_owned(),
        comm: "RenderThread".to_owned(),
        cpu: 0,
        prio: 120,
        latency_ns: 6_000_000,
        wakeup_ns: 1_010_000_000,
        switch_ns: 1_016_000_000,
    }];

    recorder::finalize_recording(FinalizeRecordingInput {
        recording: &recording,
        config: &config,
        stop_reason: "test",
        active_targets: &active_targets,
        stats_by_task: &stats_by_task,
        interval_records: &[],
        tree_events: &[],
        spike_events: &spike_events,
        spike_events_truncated: false,
        drop_counters: DropCountersSnapshot::default(),
    })
    .unwrap();

    crate::report::print_report(&dir, false, 10, 5).unwrap();
    crate::report::print_report(&dir, true, 10, 5).unwrap();

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
    };

    let config = test_config(vec![7], vec![], Some(Duration::from_secs(1)));
    let active_targets: BTreeMap<u32, TaskInfo> = BTreeMap::new();
    let stats_by_task: BTreeMap<u32, metrics::TaskStats> = BTreeMap::new();

    let spike_events = (0..10)
        .map(|idx| SpikeEvent {
            elapsed_ms: Some(idx),
            task: 100 + idx as u32,
            active: true,
            class: TaskClass::Helper,
            process_pid: Some(100 + idx as u32),
            process_comm: format!("proc-{idx}"),
            comm: format!("worker-{idx}"),
            cpu: idx as u32 % 4,
            prio: 120,
            latency_ns: 1_000_000 + idx as u64,
            wakeup_ns: 1_000_000_000 + idx as u64 * 100_000,
            switch_ns: 1_001_000_000 + idx as u64 * 100_000,
        })
        .collect::<Vec<_>>();

    recorder::finalize_recording(FinalizeRecordingInput {
        recording: &recording,
        config: &config,
        stop_reason: "test",
        active_targets: &active_targets,
        stats_by_task: &stats_by_task,
        interval_records: &[],
        tree_events: &[],
        spike_events: &spike_events,
        spike_events_truncated: false,
        drop_counters: DropCountersSnapshot::default(),
    })
    .unwrap();

    let session_path = dir.join("session.json");
    let session: SessionFile =
        serde_json::from_str(&fs::read_to_string(&session_path).unwrap()).unwrap();

    let output = crate::report::render_report(&session_path, &session, Some(&spike_events), 10, 5);

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
fn prepare_recording_refuses_to_overwrite_existing_output_dir() {
    let dir = temp_test_dir("recording-existing-dir");
    fs::create_dir_all(&dir).unwrap();

    let mut config = test_config(vec![7], vec![], None);
    config.recording = Some(RecordingConfig {
        run_name: Some("collision-test".to_owned()),
        out_dir: Some(dir.clone()),
    });

    let err = recorder::prepare_recording(&config).unwrap_err();

    assert!(err.to_string().contains("output directory already exists"));

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
        spike_threshold_ns: 1_000_000,
        verbose: false,
        recording: None,
        max_duration,
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
        comm: comm.to_owned(),
        process_comm: process_comm.to_owned(),
        class,
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
        prio: 120,
        wakeup_ns: 100,
        switch_ns: 100 + latency_ns,
        latency_ns,
        comm: comm_bytes,
    }
}

fn spike_event(task: u32, switch_ns: u64) -> SpikeEvent {
    SpikeEvent {
        elapsed_ms: Some(u128::from(switch_ns / 1_000_000)),
        task,
        active: true,
        class: TaskClass::Helper,
        process_pid: Some(task),
        process_comm: format!("proc-{task}"),
        comm: format!("worker-{task}"),
        cpu: 0,
        prio: 120,
        latency_ns: 1_000_000,
        wakeup_ns: switch_ns.saturating_sub(1_000_000),
        switch_ns,
    }
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

    for tid in tids {
        let task_dir = proc_dir.join("task").join(tid.to_string());
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(task_dir.join("comm"), format!("{name}-{tid}\n")).unwrap();
    }
}
