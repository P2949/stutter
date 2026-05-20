//! Regression coverage for task identity reuse and event-driven task naming.

use super::{support::*, *};

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
    let monitor_config = config;
    let stats_by_task = BTreeMap::from([(7, metrics::TaskStats::new(7, "?".to_owned(), 0))]);

    let first_event = scheduler_event(7, "real-name");
    let mut tasks = tasks::TaskTracker {
        stats_by_task,
        ..Default::default()
    };
    let mut recorder = recorder::LiveRecorder::default();

    events::handle_event(events::HandleEventInput {
        event: &first_event,
        config: &monitor_config,
        started: Instant::now(),
        tasks: &mut tasks,
        monotonic_start_ns: None,
        recorder: &mut recorder,
        diagnostics: Default::default(),
    });

    assert_eq!(tasks.stats_by_task.get(&7).unwrap().comm, "real-name");

    let second_event = scheduler_event(7, "later-name");
    events::handle_event(events::HandleEventInput {
        event: &second_event,
        config: &monitor_config,
        started: Instant::now(),
        tasks: &mut tasks,
        monotonic_start_ns: None,
        recorder: &mut recorder,
        diagnostics: Default::default(),
    });

    assert_eq!(tasks.stats_by_task.get(&7).unwrap().comm, "real-name");
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
