use super::*;

#[test]
fn merge_config_sources_lossy_for_tests_returns_default_on_invalid_user_layer() {
    let mut default_config = MonitorConfig::default();
    default_config.timing.summary_period_ms = 333;

    let user_file = crate::config_file::UserConfigFile {
        focus_source: Some("invalid".to_owned()),
        ..Default::default()
    };

    let merged = merge_config_sources_lossy_for_tests(ConfigSources {
        defaults: DefaultConfig {
            config: default_config,
        },
        user_file: Some(user_file),
        preset: None,
        overrides: CliOverrides {
            layer: MonitorConfigLayer::default(),
        }
        .into(),
    });

    assert_eq!(
        merged.timing.summary_period_ms,
        MonitorConfig::default().timing.summary_period_ms
    );
    assert_eq!(
        merged.focus.focus_source,
        MonitorConfig::default().focus.focus_source
    );
}

#[test]
fn merge_config_sources_applies_user_file_before_cli_overrides() {
    let cli_layer = MonitorConfigLayer {
        summary_period_ms: Some(1_000),
        foreground_include_title: Some(false),
        ..Default::default()
    };

    let user_file = crate::config_file::UserConfigFile {
        summary_ms: Some(333),
        spike_us: Some(2_500),
        hwmon: Some(true),
        cpu_freq: Some(true),
        max_tasks: Some(77),
        retain_intervals: Some(12),
        retention_max_run_count: Some(20),
        retention_max_total_bytes: Some(2_000_000),
        retention_max_age_seconds: Some(86_400),
        retention_min_free_bytes: Some(1_000_000_000),
        foreground_window: Some(true),
        focus_source: Some("hybrid".to_owned()),
        foreground_source: Some("sway".to_owned()),
        foreground_poll_ms: Some(444),
        foreground_max_stale_ms: Some(555),
        foreground_include_title: Some(true),
        ..Default::default()
    };

    let merged = merge_config_sources_checked(ConfigSources {
        defaults: DefaultConfig::default(),
        user_file: Some(user_file),
        preset: None,
        overrides: CliOverrides { layer: cli_layer }.into(),
    })
    .unwrap();

    assert_eq!(merged.timing.summary_period_ms, 1_000);
    assert_eq!(merged.timing.spike_threshold_ns, 2_500_000);
    assert!(merged.probes.hwmon);
    assert!(merged.probes.cpu_freq);
    assert_eq!(merged.target.max_tasks, 77);
    assert_eq!(merged.recording.retain_intervals, Some(12));
    assert_eq!(merged.recording.retention.max_run_count, Some(20));
    assert_eq!(merged.recording.retention.max_total_bytes, Some(2_000_000));
    assert_eq!(merged.recording.retention.max_age_seconds, Some(86_400));
    assert_eq!(
        merged.recording.retention.min_free_bytes,
        Some(1_000_000_000)
    );
    assert!(merged.focus.foreground_window);
    assert_eq!(merged.focus.focus_source, FocusSource::Hybrid);
    assert_eq!(merged.focus.foreground_source, ForegroundSource::Sway);
    assert_eq!(merged.focus.foreground_poll_ms, 444);
    assert_eq!(merged.focus.foreground_max_stale_ms, 555);
    assert!(!merged.focus.foreground_include_title);
}

#[test]
fn merge_config_sources_checked_defaults_only_matches_monitor_config_default() {
    let merged = merge_config_sources_checked(ConfigSources {
        defaults: DefaultConfig::default(),
        user_file: None,
        preset: None,
        overrides: CliOverrides {
            layer: MonitorConfigLayer::default(),
        }
        .into(),
    })
    .unwrap();

    assert_eq!(merged, MonitorConfig::default());
}

#[test]
fn merge_config_sources_checked_user_file_overrides_builtin_defaults() {
    let user_file = crate::config_file::UserConfigFile {
        summary_period_ms: Some(250),
        hwmon: Some(true),
        focus_source: Some("foreground".to_owned()),
        foreground_poll_ms: Some(333),
        foreground_include_title: Some(true),
        ..Default::default()
    };

    let merged = merge_config_sources_checked(ConfigSources {
        defaults: DefaultConfig::default(),
        user_file: Some(user_file),
        preset: None,
        overrides: CliOverrides {
            layer: MonitorConfigLayer::default(),
        }
        .into(),
    })
    .unwrap();

    assert_eq!(merged.timing.summary_period_ms, 250);
    assert!(merged.probes.hwmon);
    assert_eq!(merged.focus.focus_source, FocusSource::Foreground);
    assert_eq!(merged.focus.foreground_poll_ms, 333);
    assert!(merged.focus.foreground_include_title);
}

#[test]
fn merge_config_sources_checked_cli_and_api_layers_have_same_value_semantics() {
    let user_file = crate::config_file::UserConfigFile {
        summary_period_ms: Some(250),
        hwmon: Some(true),
        focus_source: Some("foreground".to_owned()),
        foreground_source: Some("sway".to_owned()),
        foreground_poll_ms: Some(333),
        ..Default::default()
    };
    let preset = PresetConfig {
        layer: MonitorConfigLayer {
            summary_period_ms: Some(500),
            block_io: Some(true),
            run_name: Some(Some("preset-run".to_owned())),
            ..Default::default()
        },
    };
    let override_layer = MonitorConfigLayer {
        summary_period_ms: Some(1_000),
        hwmon: Some(false),
        focus_source: Some(FocusSource::Heuristic),
        foreground_source: Some(ForegroundSource::Auto),
        foreground_poll_ms: Some(1_000),
        block_io: Some(false),
        run_name: Some(None),
        ..Default::default()
    };

    let cli_merged = merge_config_sources_checked(ConfigSources {
        defaults: DefaultConfig::default(),
        user_file: Some(user_file.clone()),
        preset: Some(preset.clone()),
        overrides: CliOverrides {
            layer: override_layer.clone(),
        }
        .into(),
    })
    .unwrap();

    let api_merged = merge_config_sources_checked(ConfigSources {
        defaults: DefaultConfig::default(),
        user_file: Some(user_file),
        preset: Some(preset),
        overrides: ApiOverrides {
            layer: override_layer,
        }
        .into(),
    })
    .unwrap();

    assert_eq!(cli_merged, api_merged);
    assert_eq!(api_merged.timing.summary_period_ms, 1_000);
    assert!(!api_merged.probes.hwmon);
    assert!(!api_merged.hwmon.enabled);
    assert_eq!(api_merged.focus.focus_source, FocusSource::Heuristic);
    assert_eq!(api_merged.focus.foreground_source, ForegroundSource::Auto);
    assert_eq!(api_merged.focus.foreground_poll_ms, 1_000);
    assert!(!api_merged.probes.block_io);
    assert_eq!(api_merged.recording.run_name, None);
}
