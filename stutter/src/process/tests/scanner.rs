//! Tests for process scanning, classification, cache eviction, and cgroup traversal.
//!
//! Owns process-tree scanner regression tests. Does not own production procfs readers, cache
//! structures, task models, or tree traversal logic.

use std::{fs, path::Path};

use super::*;

#[test]
fn detects_gamescope_as_auto_target() {
    let dir = std::env::temp_dir().join(format!("stutter-auto-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let pid_dir = dir.join("123");
    fs::create_dir_all(&pid_dir).unwrap();
    fs::write(pid_dir.join("status"), "Name:\tgamescope\nPPid:\t1\n").unwrap();
    fs::write(pid_dir.join("cmdline"), "gamescope\0-f\0--\0steam\0").unwrap();
    fs::write(pid_dir.join("stat"), "123 (gamescope) S 1 123 0 0 -1 0 0 0 0 0 0 0 0 20 0 1 0 1000 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n").unwrap();

    let auto = find_auto_target_pids(&dir);
    assert_eq!(auto.len(), 1);
    assert_eq!(auto[0].0, 123);
    assert_eq!(auto[0].1, TaskClass::GameScope);

    fs::remove_dir_all(dir).ok();
}

#[test]
fn prioritizes_gamescope_over_steam_runtime() {
    let dir = std::env::temp_dir().join(format!("stutter-auto-prio-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);

    // Steam Runtime (priority 2)
    let pid1 = dir.join("101");
    fs::create_dir_all(&pid1).unwrap();
    fs::write(pid1.join("status"), "Name:\tpressure-vessel\nPPid:\t1\n").unwrap();
    fs::write(pid1.join("cmdline"), "pressure-vessel-wrap\0").unwrap();
    fs::write(pid1.join("stat"), "101 (pressure-vessel) S 1 101 0 0 -1 0 0 0 0 0 0 0 0 20 0 1 0 1000 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n").unwrap();

    // GameScope (priority 1)
    let pid2 = dir.join("102");
    fs::create_dir_all(&pid2).unwrap();
    fs::write(pid2.join("status"), "Name:\tgamescope\nPPid:\t1\n").unwrap();
    fs::write(pid2.join("cmdline"), "gamescope\0").unwrap();
    fs::write(pid2.join("stat"), "102 (gamescope) S 1 102 0 0 -1 0 0 0 0 0 0 0 0 20 0 1 0 1000 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n").unwrap();

    let auto = find_auto_target_pids(&dir);
    assert_eq!(auto.len(), 1);
    assert_eq!(auto[0].0, 102);
    assert_eq!(auto[0].1, TaskClass::GameScope);

    fs::remove_dir_all(dir).ok();
}

#[test]
fn process_cache_ttl_refresh() {
    let dir = std::env::temp_dir().join(format!("stutter-ttl-test-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    let pid_dir = dir.join("100");
    fs::create_dir_all(&pid_dir).unwrap();
    fs::write(pid_dir.join("status"), "Name:\ttest\nPPid:\t1\n").unwrap();
    fs::write(pid_dir.join("cmdline"), "test\0").unwrap();
    fs::write(
        pid_dir.join("stat"),
        "100 (test) S 1 100 0 0 -1 0 0 0 0 0 0 0 0 20 0 1 0 1000 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
    )
    .unwrap();

    let mut cache = ProcessCache {
        max_cached_generations: 1,
        ..Default::default()
    };

    // scan 1: inserts at generation 1
    let budget = ScanBudget::default_proc_scan();
    let mut budget_report1 = ScanBudgetReport::default();
    let p1 = scan_processes_at(&dir, &mut cache, &budget, &mut budget_report1);
    assert_eq!(p1.len(), 1);
    assert_eq!(cache.generation, 1);
    assert_eq!(cache.entries.get(&100).unwrap().scan_generation, 1);

    // scan 2: delta is 1, cache is still reused (generation 2, cached 1, 2-1=1 <= 1)
    let mut budget_report2 = ScanBudgetReport::default();
    let p2 = scan_processes_at(&dir, &mut cache, &budget, &mut budget_report2);
    assert_eq!(p2.len(), 1);
    assert_eq!(cache.generation, 2);
    assert_eq!(cache.entries.get(&100).unwrap().scan_generation, 1);

    // scan 3: delta is 2, cache is refreshed (generation 3, cached 1, 3-1=2 > 1)
    let mut budget_report3 = ScanBudgetReport::default();
    let p3 = scan_processes_at(&dir, &mut cache, &budget, &mut budget_report3);
    assert_eq!(p3.len(), 1);
    assert_eq!(cache.generation, 3);
    assert_eq!(cache.entries.get(&100).unwrap().scan_generation, 3);

    fs::remove_dir_all(dir).ok();
}

#[test]
fn test_thread_ids_of_at_limited() {
    let dir =
        std::env::temp_dir().join(format!("stutter-test-thread-limit-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);

    let pid_dir = dir.join("100");
    fs::create_dir_all(&pid_dir).unwrap();

    let task_dir = pid_dir.join("task");
    fs::create_dir_all(&task_dir).unwrap();

    for tid in 100..110 {
        fs::create_dir_all(task_dir.join(tid.to_string())).unwrap();
    }

    let mut report = ScanBudgetReport::default();
    let tids = thread_ids_of_at_limited(&dir, 100, 5, &mut report);

    assert_eq!(tids.len(), 5);
    assert_eq!(report.thread_entries_seen, 6);
    assert_eq!(report.thread_entries_skipped, 1);
    assert_eq!(report.processes_thread_limited, 1);

    let _ = fs::remove_dir_all(&dir);
}

fn write_fake_process(
    proc_root: &Path,
    pid: u32,
    comm: &str,
    cmdline: &str,
) -> std::io::Result<()> {
    let dir = proc_root.join(pid.to_string());
    fs::create_dir_all(&dir)?;

    fs::write(
        dir.join("status"),
        format!("Name:\t{comm}\nPid:\t{pid}\nPPid:\t1\nThreads:\t1\n"),
    )?;

    fs::write(dir.join("cmdline"), format!("{cmdline}\0"))?;

    fs::write(
        dir.join("stat"),
        format!(
            "{pid} ({comm}) S 1 {pid} {pid} 0 -1 4194304 0 0 0 0 0 0 0 0 20 0 1 0 12345 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n"
        ),
    )?;

    Ok(())
}

#[test]
fn community_rule_cmdline_basename_classifies_truncated_proton_game() {
    let class = classify_task_with_context(
        "KingdomCome",
        "KingdomCome",
        "/home/me/.steam/steamapps/compatdata/379430/pfx/drive_c/KingdomCome.exe --windowed",
        "/usr/bin/wine",
        "/user.slice/app-mystery-379430.scope",
        None,
    );

    assert_eq!(class, TaskClass::Game);
}

#[test]
fn ambiguous_community_rule_outside_steam_context_is_not_game() {
    let class = classify_task_with_context(
        "build.exe",
        "build.exe",
        "/tmp/build.exe --compile",
        "/tmp/build.exe",
        "/user.slice/app-builder.scope",
        None,
    );

    assert_ne!(class, TaskClass::Game);
}

#[test]
fn ambiguous_community_rule_inside_compatdata_context_can_be_game() {
    let class = classify_task_with_context(
        "build.exe",
        "build.exe",
        "/home/me/.steam/steamapps/compatdata/123/pfx/drive_c/build.exe",
        "/usr/bin/wine",
        "/user.slice/app-mystery-123.scope",
        None,
    );

    assert_eq!(class, TaskClass::Game);
}

#[test]
fn hardcoded_audio_classification_wins_over_game_like_context() {
    let class = classify_task_with_context(
        "pipewire",
        "pipewire",
        "/home/me/.steam/steamapps/compatdata/379430/pfx/drive_c/KingdomCome.exe",
        "/home/me/.steam/steamapps/common/KingdomCome/KingdomCome.exe",
        "/user.slice/app-mystery-379430.scope",
        Some(2),
    );

    assert_eq!(class, TaskClass::AudioRealtime);
}

#[test]
fn process_cache_evicts_pid_missing_from_next_scan() {
    let temp = tempfile::tempdir().unwrap();
    let proc_root = temp.path();

    write_fake_process(proc_root, 100, "old", "old-cmd").unwrap();

    let mut cache = ProcessCache::default();
    let budget = ScanBudget::default_proc_scan();
    let mut budget_report = ScanBudgetReport::default();

    let first = scan_processes_at(proc_root, &mut cache, &budget, &mut budget_report);
    assert!(first.contains_key(&100));
    assert!(cache.entries.contains_key(&100));

    fs::remove_dir_all(proc_root.join("100")).unwrap();

    let mut budget_report2 = ScanBudgetReport::default();
    let second = scan_processes_at(proc_root, &mut cache, &budget, &mut budget_report2);
    assert!(!second.contains_key(&100));
    assert!(!cache.entries.contains_key(&100));
}

#[test]
fn process_cache_replaced_when_pid_recreated_with_new_comm() {
    let temp = tempfile::tempdir().unwrap();
    let proc_root = temp.path();

    write_fake_process(proc_root, 100, "old", "old-cmd").unwrap();

    let mut cache = ProcessCache::default();
    let budget = ScanBudget::default_proc_scan();

    let mut budget_report1 = ScanBudgetReport::default();
    let first = scan_processes_at(proc_root, &mut cache, &budget, &mut budget_report1);
    assert_eq!(first.get(&100).unwrap().comm, "old");
    assert_eq!(cache.entries.get(&100).unwrap().info.comm, "old");

    fs::remove_dir_all(proc_root.join("100")).unwrap();

    let mut budget_report2 = ScanBudgetReport::default();
    let _ = scan_processes_at(proc_root, &mut cache, &budget, &mut budget_report2);
    assert!(!cache.entries.contains_key(&100));

    write_fake_process(proc_root, 100, "new", "new-cmd").unwrap();

    let mut budget_report3 = ScanBudgetReport::default();
    let third = scan_processes_at(proc_root, &mut cache, &budget, &mut budget_report3);
    assert_eq!(third.get(&100).unwrap().comm, "new");
    assert_eq!(cache.entries.get(&100).unwrap().info.comm, "new");
}
