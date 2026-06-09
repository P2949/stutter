//! Regression coverage for target config, task classification, and task diff ordering.

use super::{support::*, *};

#[test]
fn diff_tasks_orders_removed_before_added_by_tid() {
    let old_tasks = BTreeMap::from([
        (
            2.into(),
            task_info(2, 20, "old", "old-2", TaskClass::Helper),
        ),
        (
            4.into(),
            task_info(4, 40, "old", "old-4", TaskClass::Helper),
        ),
    ]);
    let new_tasks = BTreeMap::from([
        (1.into(), task_info(1, 10, "new", "new-1", TaskClass::Game)),
        (3.into(), task_info(3, 30, "new", "new-3", TaskClass::Game)),
    ]);

    let diffs = process_tree::diff_tasks_ref(&old_tasks, &new_tasks);

    let actions_and_tids = diffs
        .iter()
        .map(|diff| (&diff.action, diff.task.task_id().as_u32()))
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
fn config_reports_explicit_targets_for_manual_tree_watch_and_cgroup() {
    let mut config = test_config(Vec::new(), Vec::new(), None);
    assert!(!config.has_explicit_target());

    config.target.target_pids = vec![123];
    assert!(config.has_explicit_target());

    config.target.target_pids.clear();
    config.target.tree_pids = vec![456];
    assert!(config.has_explicit_target());

    config.target.tree_pids.clear();
    config.target.watch_process = Some("gamescope".to_owned());
    assert!(config.has_explicit_target());

    config.target.watch_process = None;
    config.target.cgroupv2 = Some(PathBuf::from("/sys/fs/cgroup/user.slice"));
    assert!(config.has_explicit_target());
}

#[test]
fn config_auto_focus_enabled_ignores_explicit_targets() {
    let mut config = test_config(Vec::new(), Vec::new(), None);
    config.focus.auto_focus = true;
    config.focus.auto_focus_poll_ms = 250;
    config.focus.auto_focus_min_confidence = 0.75;
    config.focus.auto_focus_switch_cooldown_ms = 7500;
    config.focus.auto_focus_switch_margin = 0.30;
    config.focus.auto_focus_required_polls = 3;
    config.focus.auto_focus_max_roots = 2;

    assert!(config.auto_focus_enabled());

    let auto_focus = &config.focus;
    assert!(auto_focus.auto_focus);
    assert_eq!(auto_focus.auto_focus_poll_ms, 250);
    assert_eq!(auto_focus.auto_focus_min_confidence, 0.75);
    assert_eq!(auto_focus.auto_focus_switch_cooldown_ms, 7500);
    assert_eq!(auto_focus.auto_focus_switch_margin, 0.30);
    assert_eq!(auto_focus.auto_focus_required_polls, 3);
    assert_eq!(auto_focus.auto_focus_max_roots, 2);

    config.target.tree_pids = vec![999];
    assert!(config.focus.auto_focus);
    assert!(!config.auto_focus_enabled());

    let auto_focus = &config.focus;
    assert!(auto_focus.auto_focus);
    assert_eq!(auto_focus.auto_focus_poll_ms, 250);
    assert_eq!(auto_focus.auto_focus_min_confidence, 0.75);
    assert_eq!(auto_focus.auto_focus_switch_cooldown_ms, 7500);
    assert_eq!(auto_focus.auto_focus_switch_margin, 0.30);
    assert_eq!(auto_focus.auto_focus_required_polls, 3);
    assert_eq!(auto_focus.auto_focus_max_roots, 2);
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
        TaskClass::Service
    );
}

#[test]
fn classify_task_v1_rules_and_priority_bands() {
    assert_eq!(
        process_tree::classify_task_with_context("pipewire", "pipewire", "", "", "", None),
        TaskClass::AudioRealtime
    );
    assert_eq!(
        process_tree::priority_band_for_class(TaskClass::AudioRealtime, Some(1)),
        process_tree::PriorityBand::CriticalRealtime
    );

    assert_eq!(
        process_tree::classify_task_with_context(
            "audio-thread",
            "game",
            "audio callback",
            "",
            "",
            Some(2),
        ),
        TaskClass::AudioRealtime
    );

    assert_eq!(
        process_tree::classify_task_with_context("libinput", "sway", "", "", "", None),
        TaskClass::Input
    );

    assert_eq!(
        process_tree::classify_task_with_context(
            "RenderThread",
            "Game.exe",
            "DXVK render thread",
            "/home/user/.steam/steamapps/common/Game/Game.exe",
            "",
            None,
        ),
        TaskClass::GameRenderThread
    );

    assert_eq!(
        process_tree::classify_task_with_context(
            "GameWorker",
            "Game.exe",
            "worker thread",
            "/home/user/.steam/steamapps/common/Game/Game.exe",
            "",
            None,
        ),
        TaskClass::GameWorkerThread
    );

    assert_eq!(
        process_tree::classify_task_with_context(
            "pressure-vessel",
            "pressure-vessel",
            "",
            "/home/user/.steam/steamapps/common/Game/pressure-vessel",
            "",
            None,
        ),
        TaskClass::Game
    );

    assert_eq!(
        process_tree::classify_task_with_context(
            "GPU Process",
            "firefox",
            "firefox -contentproc --type=gpu-process",
            "",
            "",
            None,
        ),
        TaskClass::BrowserGpu
    );

    assert_eq!(
        process_tree::classify_task_with_context(
            "Web Content",
            "firefox",
            "firefox isolated web content",
            "",
            "",
            None,
        ),
        TaskClass::BrowserRenderer
    );

    assert_eq!(
        process_tree::classify_task_with_context("rustc", "rustc", "", "", "", None),
        TaskClass::Compiler
    );
    assert_eq!(
        process_tree::priority_band_for_class(TaskClass::Compiler, None),
        process_tree::PriorityBand::Throughput
    );

    assert_eq!(
        process_tree::classify_task_with_context("clangd", "clangd", "", "", "", None),
        TaskClass::Indexer
    );

    assert_eq!(
        process_tree::classify_task_with_context("emerge", "emerge", "", "", "", None),
        TaskClass::PackageManager
    );

    assert_eq!(
        process_tree::classify_task_with_context(
            "[kworker/0:1]",
            "[kworker/0:1]",
            "",
            "",
            "",
            None
        ),
        TaskClass::KernelThread
    );

    assert_eq!(
        process_tree::classify_task_with_context(
            "irq/123-amdgpu",
            "irq/123-amdgpu",
            "",
            "",
            "",
            None
        ),
        TaskClass::IrqThread
    );

    assert_eq!(
        process_tree::classify_task_with_context(
            "NetworkManager",
            "NetworkManager",
            "",
            "",
            "",
            None
        ),
        TaskClass::NetworkDaemon
    );
}
