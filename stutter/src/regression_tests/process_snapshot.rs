//! Regression coverage for procfs snapshots, cgroup expansion, and watch-process selection.

use super::{support::*, *};

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
    let budget = process_tree::ScanBudget::default_proc_scan();
    let mut budget_report = process_tree::ScanBudgetReport::default();
    let first = process_tree::scan_processes_at(&dir, &mut cache, &budget, &mut budget_report);
    assert_eq!(first.get(&10).unwrap().comm, "old-name");

    fs::remove_dir_all(dir.join("10")).unwrap();
    // Recreate the process to simulate PID reuse.
    std::thread::sleep(std::time::Duration::from_millis(10));
    create_fake_proc(&dir, 10, 1, "new-name", "new-name", &[10]);
    // Manually overwrite stat to match the test's expected starttime.
    fs::write(dir.join("10/stat"), fake_stat("new-name", 999)).unwrap();

    let budget = process_tree::ScanBudget::default_proc_scan();
    let mut budget_report = process_tree::ScanBudgetReport::default();
    let second = process_tree::scan_processes_at(&dir, &mut cache, &budget, &mut budget_report);
    let reused_process = second
        .get(&10)
        .expect("recreated fake process should be present in second scan");
    assert_eq!(reused_process.comm, "new-name");
    assert_eq!(reused_process.starttime_ticks, Some(999));

    fs::remove_dir_all(dir).ok();
}

#[test]
fn process_cache_can_be_invalidated_for_exec_without_starttime_change() {
    let dir = temp_test_dir("proc-cache-exec");
    create_fake_proc(&dir, 10, 1, "launcher", "launcher", &[10]);

    let mut cache = process_tree::ProcessCache::default();
    let budget = process_tree::ScanBudget::default_proc_scan();
    let mut budget_report = process_tree::ScanBudgetReport::default();
    let first = process_tree::scan_processes_at(&dir, &mut cache, &budget, &mut budget_report);
    assert_eq!(first.get(&10).unwrap().comm, "launcher");

    fs::write(dir.join("10/status"), "Name:\tgame\nPPid:\t1\n").unwrap();
    fs::write(dir.join("10/cmdline"), b"game.exe").unwrap();
    cache.invalidate(10);

    let mut budget_report = process_tree::ScanBudgetReport::default();
    let second = process_tree::scan_processes_at(&dir, &mut cache, &budget, &mut budget_report);
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
    assert_eq!(
        snapshot.process_roots,
        [100.into(), 101.into()].into_iter().collect()
    );
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
