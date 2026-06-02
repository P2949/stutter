//! Tests for monitor target, foreground, and filter CLI parsing.
//!
//! Owns monitor argument regression tests for target/filter behavior. Does not own production CLI
//! argument definitions, merging, or validation.

use super::*;

#[test]
fn parses_pid_targets_and_deduplicates() {
    let config = parse_monitor_config_for_phase15([
        "stutter", "monitor", "--pid", "42", "--pid", "7", "--pid", "42",
    ])
    .unwrap();

    assert_eq!(config.target.target_pids, vec![7, 42]);
    assert!(config.target.tree_pids.is_empty());
}

#[test]
fn parses_tree_pids_and_exclude_tree_pids() {
    let config = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--tree-pid",
        "42",
        "--tree-pid",
        "42",
        "--tree-pid",
        "7",
        "--exclude-tree-pid",
        "100",
        "--exclude-tree-pid",
        "100",
        "--exclude-tree-pid",
        "8",
    ])
    .unwrap();

    assert_eq!(config.target.tree_pids, vec![7, 42]);
    assert_eq!(config.target.exclude_tree_pids, vec![8, 100]);
}

#[test]
fn parses_include_and_exclude_comm_filters() {
    let config = parse_monitor_config_for_phase15([
        "stutter",
        "record",
        "--tree-pid",
        "42",
        "--include-comm",
        "RenderThread",
        "--exclude-comm",
        "steamwebhelper",
    ])
    .unwrap();

    assert_eq!(config.target.include_comm, vec!["RenderThread".to_owned()]);
    assert_eq!(
        config.target.exclude_comm,
        vec!["steamwebhelper".to_owned()]
    );
}

#[test]
fn include_and_exclude_comm_filters_are_sorted_and_deduplicated() {
    let config = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--tree-pid",
        "42",
        "--include-comm",
        "RenderThread",
        "--include-comm",
        "Game",
        "--include-comm",
        "RenderThread",
        "--exclude-comm",
        "steamwebhelper",
        "--exclude-comm",
        "browser",
        "--exclude-comm",
        "steamwebhelper",
    ])
    .unwrap();

    assert_eq!(
        config.target.include_comm,
        vec!["Game".to_owned(), "RenderThread".to_owned()]
    );
    assert_eq!(
        config.target.exclude_comm,
        vec!["browser".to_owned(), "steamwebhelper".to_owned()]
    );
}

#[test]
fn rejects_zero_pid_targets() {
    let err = parse_monitor_config_for_phase15(["stutter", "monitor", "--pid", "0"]).unwrap_err();

    assert!(err.to_string().contains("--pid must be greater than zero"));
}

#[test]
fn rejects_zero_tree_pid_targets() {
    let err =
        parse_monitor_config_for_phase15(["stutter", "monitor", "--tree-pid", "0"]).unwrap_err();

    assert!(
        err.to_string()
            .contains("--tree-pid must be greater than zero")
    );
}

#[test]
fn rejects_zero_exclude_tree_pid() {
    let err = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--tree-pid",
        "42",
        "--exclude-tree-pid",
        "0",
    ])
    .unwrap_err();

    assert!(
        err.to_string()
            .contains("--exclude-tree-pid must be greater than zero")
    );
}

#[test]
fn rejects_zero_max_tasks() {
    let err =
        parse_monitor_config_for_phase15(["stutter", "monitor", "--pid", "42", "--max-tasks", "0"])
            .unwrap_err();

    assert!(
        err.to_string()
            .contains("--max-tasks must be greater than zero")
    );
}

#[test]
fn parses_max_tasks() {
    let config = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--pid",
        "42",
        "--max-tasks",
        "1024",
    ])
    .unwrap();

    assert_eq!(config.target.max_tasks, crate::config::TARGET_PIDS_MAX);
}

#[test]
fn rejects_max_tasks_above_target_pid_capacity() {
    let err = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--pid",
        "42",
        "--max-tasks",
        "1025",
    ])
    .unwrap_err();

    assert!(err.to_string().contains("target.max_tasks"));
}

#[test]
fn native_cgroup_filter_requires_cgroupv2() {
    let result = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--pid",
        "42",
        "--native-cgroup-filter",
    ]);

    assert!(result.is_err());
}

#[test]
fn native_cgroup_filter_sets_config() {
    let config = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--cgroupv2",
        "/sys/fs/cgroup/test.slice",
        "--native-cgroup-filter",
    ])
    .unwrap();

    assert_eq!(
        config.target.cgroupv2.as_deref(),
        Some(std::path::Path::new("/sys/fs/cgroup/test.slice"))
    );
    assert!(config.safety.native_cgroup_filter);
}

#[test]
fn native_cgroup_filter_defaults_false() {
    let config = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--cgroupv2",
        "/sys/fs/cgroup/test.slice",
    ])
    .unwrap();

    assert_eq!(
        config.target.cgroupv2.as_deref(),
        Some(std::path::Path::new("/sys/fs/cgroup/test.slice"))
    );
    assert!(!config.safety.native_cgroup_filter);
}

#[test]
fn cli_accepts_auto_focus_foreground_source() {
    let config = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--auto-focus",
        "--focus-source",
        "foreground",
        "--foreground-source",
        "sway",
    ])
    .unwrap();

    assert!(config.focus.auto_focus);
    assert_eq!(config.focus.focus_source, FocusSource::Foreground);
    assert!(config.focus.foreground_window);
    assert_eq!(config.focus.foreground_source, ForegroundSource::Sway);
}

#[test]
fn foreground_include_title_requires_foreground_window_or_auto_focus_foreground() {
    let err =
        parse_monitor_config_for_phase15(["stutter", "monitor", "--foreground-include-title"])
            .unwrap_err()
            .to_string();

    assert!(err.contains(
        "--foreground-include-title requires --foreground-window or --auto-focus with --focus-source foreground or hybrid"
    ));

    let foreground_window = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--foreground-window",
        "--foreground-include-title",
    ])
    .unwrap();
    assert!(foreground_window.focus.foreground_window);
    assert!(foreground_window.focus.foreground_include_title);

    let foreground_focus = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--auto-focus",
        "--focus-source",
        "foreground",
        "--foreground-include-title",
    ])
    .unwrap();
    assert!(foreground_focus.focus.auto_focus);
    assert_eq!(foreground_focus.focus.focus_source, FocusSource::Foreground);
    assert!(foreground_focus.focus.foreground_window);
    assert!(foreground_focus.focus.foreground_include_title);
}

#[test]
fn config_file_sets_summary_when_cli_omitted() {
    let args = MonitorArgs {
        watch_poll_ms: 2000,
        cpu_perf_max_tasks: 128,
        ..MonitorArgs::default()
    };
    let file_config = crate::config_file::UserConfigFile {
        summary_ms: Some(500),
        ..Default::default()
    };

    let config =
        monitor_config_from_monitor_args_with_file(args, Some(file_config), RecordingMode::Monitor)
            .unwrap();

    assert_eq!(config.timing.summary_period_ms, 500);
}

#[test]
fn cli_summary_overrides_config_file_summary() {
    let args = MonitorArgs {
        summary_period_ms: Some(200),
        watch_poll_ms: 2000,
        cpu_perf_max_tasks: 128,
        ..MonitorArgs::default()
    };
    let file_config = crate::config_file::UserConfigFile {
        summary_ms: Some(500),
        ..Default::default()
    };

    let config =
        monitor_config_from_monitor_args_with_file(args, Some(file_config), RecordingMode::Monitor)
            .unwrap();

    assert_eq!(config.timing.summary_period_ms, 200);
}

#[test]
fn live_diagnosis_cluster_window_accepts_config_and_cli_override() {
    let args = MonitorArgs {
        watch_poll_ms: 2000,
        cpu_perf_max_tasks: 128,
        ..MonitorArgs::default()
    };
    let file_config = crate::config_file::UserConfigFile {
        live_diagnosis_cluster_window_ms: Some(7),
        ..Default::default()
    };

    let config =
        monitor_config_from_monitor_args_with_file(args, Some(file_config), RecordingMode::Monitor)
            .unwrap();

    assert_eq!(config.diagnosis.live_cluster_window_ms, 7);

    let args = MonitorArgs {
        live_diagnosis_cluster_window_ms: Some(11),
        watch_poll_ms: 2000,
        cpu_perf_max_tasks: 128,
        ..MonitorArgs::default()
    };
    let file_config = crate::config_file::UserConfigFile {
        live_diagnosis_cluster_window_ms: Some(7),
        ..Default::default()
    };

    let config = monitor_config_from_monitor_args_with_file_and_presence(
        args,
        Some(file_config),
        RecordingMode::Monitor,
        MonitorArgPresence {
            live_diagnosis_cluster_window_ms: true,
            ..MonitorArgPresence::default()
        },
    )
    .unwrap();

    assert_eq!(config.diagnosis.live_cluster_window_ms, 11);
}

#[test]
fn rejects_zero_live_diagnosis_cluster_window() {
    let err = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--live-diagnosis-cluster-window-ms",
        "0",
    ])
    .unwrap_err();

    assert!(
        err.to_string()
            .contains("--live-diagnosis-cluster-window-ms must be greater than zero")
    );
}

#[test]
fn include_comm_from_config_used_when_cli_omitted() {
    let args = MonitorArgs {
        watch_poll_ms: 2000,
        cpu_perf_max_tasks: 128,
        ..MonitorArgs::default()
    };
    let file_config = crate::config_file::UserConfigFile {
        include_comm: Some(vec!["Game".to_owned(), "Render".to_owned()]),
        ..Default::default()
    };

    let config =
        monitor_config_from_monitor_args_with_file(args, Some(file_config), RecordingMode::Monitor)
            .unwrap();

    assert_eq!(
        config.target.include_comm,
        vec!["Game".to_owned(), "Render".to_owned()]
    );
}

#[test]
fn cli_include_comm_overrides_config_list() {
    let args = MonitorArgs {
        include_comm: vec!["RenderThread".to_owned()],
        watch_poll_ms: 2000,
        cpu_perf_max_tasks: 128,
        ..MonitorArgs::default()
    };
    let file_config = crate::config_file::UserConfigFile {
        include_comm: Some(vec!["Game".to_owned()]),
        ..Default::default()
    };

    let config =
        monitor_config_from_monitor_args_with_file(args, Some(file_config), RecordingMode::Monitor)
            .unwrap();

    assert_eq!(config.target.include_comm, vec!["RenderThread".to_owned()]);
}

#[test]
fn exclude_comm_from_config_used_when_cli_omitted() {
    let args = MonitorArgs {
        watch_poll_ms: 2000,
        cpu_perf_max_tasks: 128,
        ..MonitorArgs::default()
    };
    let file_config = crate::config_file::UserConfigFile {
        exclude_comm: Some(vec!["steamwebhelper".to_owned(), "browser".to_owned()]),
        ..Default::default()
    };

    let config =
        monitor_config_from_monitor_args_with_file(args, Some(file_config), RecordingMode::Monitor)
            .unwrap();

    assert_eq!(
        config.target.exclude_comm,
        vec!["browser".to_owned(), "steamwebhelper".to_owned()]
    );
}

#[test]
fn cli_exclude_comm_overrides_config_list() {
    let args = MonitorArgs {
        exclude_comm: vec!["steamwebhelper".to_owned()],
        watch_poll_ms: 2000,
        cpu_perf_max_tasks: 128,
        ..MonitorArgs::default()
    };
    let file_config = crate::config_file::UserConfigFile {
        exclude_comm: Some(vec!["browser".to_owned()]),
        ..Default::default()
    };

    let config =
        monitor_config_from_monitor_args_with_file(args, Some(file_config), RecordingMode::Monitor)
            .unwrap();

    assert_eq!(
        config.target.exclude_comm,
        vec!["steamwebhelper".to_owned()]
    );
}

#[test]
fn file_focus_source_used_when_cli_omitted() {
    let args = MonitorArgs {
        watch_poll_ms: 2000,
        cpu_perf_max_tasks: 128,
        ..MonitorArgs::default()
    };
    let file_config = crate::config_file::UserConfigFile {
        focus_source: Some("foreground".to_owned()),
        foreground_source: Some("sway".to_owned()),
        ..Default::default()
    };

    let config =
        monitor_config_from_monitor_args_with_file(args, Some(file_config), RecordingMode::Monitor)
            .unwrap();

    assert_eq!(config.focus.focus_source, FocusSource::Foreground);
    assert_eq!(config.focus.foreground_source, ForegroundSource::Sway);
    assert!(config.focus.foreground_window);
}

#[test]
fn cli_focus_source_overrides_config_file_focus_source() {
    let cli = Cli::try_parse_from([
        "stutter",
        "monitor",
        "--focus-source",
        "hybrid",
        "--foreground-source",
        "hyprland",
    ])
    .unwrap();
    let matches = Cli::command().get_matches_from([
        "stutter",
        "monitor",
        "--focus-source",
        "hybrid",
        "--foreground-source",
        "hyprland",
    ]);
    let presence = monitor_arg_presence_from_matches(&matches, Some("monitor"));

    let Some(Command::Monitor(args)) = cli.command else {
        panic!("expected monitor command");
    };

    let file_config = crate::config_file::UserConfigFile {
        focus_source: Some("foreground".to_owned()),
        foreground_source: Some("sway".to_owned()),
        ..Default::default()
    };

    let config = monitor_config_from_monitor_args_with_file_and_presence(
        args,
        Some(file_config),
        RecordingMode::Monitor,
        presence,
    )
    .unwrap();

    assert_eq!(config.focus.focus_source, FocusSource::Hybrid);
    assert_eq!(config.focus.foreground_source, ForegroundSource::Hyprland);
    assert!(config.focus.foreground_window);
}

#[test]
fn cli_accepts_gnome_and_kde_foreground_sources() {
    let gnome = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--focus-source",
        "foreground",
        "--foreground-source",
        "gnome",
    ])
    .unwrap();
    assert_eq!(gnome.focus.foreground_source, ForegroundSource::Gnome);

    let kde = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--focus-source",
        "foreground",
        "--foreground-source",
        "kde",
    ])
    .unwrap();
    assert_eq!(kde.focus.foreground_source, ForegroundSource::Kde);
}
