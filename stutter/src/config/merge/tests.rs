use super::*;
use crate::config::{FocusSource, ForegroundSource, TARGET_PIDS_MAX};

fn last_source_for_field(
    provenance: &[stutter_config::source::FieldProvenance],
    field: &'static str,
) -> Option<ConfigSource> {
    provenance
        .iter()
        .rev()
        .find(|entry| entry.field == field)
        .map(|entry| entry.source)
}

fn trace_for_field<'a>(
    trace: &'a [stutter_config::source::ConfigMergeTrace],
    field: &'static str,
) -> Option<&'a stutter_config::source::ConfigMergeTrace> {
    trace.iter().find(|entry| entry.field == field)
}

fn assert_api_override_invalid_field(layer: MonitorConfigLayer, expected_field: &'static str) {
    let err = merge_config_sources_checked(ConfigSources {
        defaults: DefaultConfig::default(),
        user_file: None,
        preset: None,
        overrides: ApiOverrides { layer }.into(),
    })
    .unwrap_err();

    match err {
        ConfigError::InvalidValue { field, .. } => assert_eq!(field, expected_field),
        other => panic!("expected InvalidValue for {expected_field}, got {other:?}"),
    }
}

#[test]
fn merge_trace_reports_selected_layer_and_reason() {
    let user_file = crate::config_file::UserConfigFile {
        summary_period_ms: Some(250),
        ..Default::default()
    };
    let cli = CliOverrides {
        layer: MonitorConfigLayer {
            summary_period_ms: Some(1_000),
            hwmon: Some(true),
            ..Default::default()
        },
    };

    let effective = merge_config_sources_effective_checked(ConfigSources {
        defaults: DefaultConfig::default(),
        user_file: Some(user_file),
        preset: None,
        overrides: cli.into(),
    })
    .unwrap();

    let summary = trace_for_field(&effective.merge_trace, "timing.summary_period_ms").unwrap();
    assert_eq!(summary.selected_layer, ConfigSource::Cli);
    assert_eq!(
        summary.reason,
        stutter_config::source::MergeReason::LaterLayerOverride
    );

    let hwmon = trace_for_field(&effective.merge_trace, "probes.hwmon").unwrap();
    assert_eq!(hwmon.selected_layer, ConfigSource::Cli);
    assert_eq!(
        hwmon.reason,
        stutter_config::source::MergeReason::LaterLayerOverride
    );

    let max_tasks = trace_for_field(&effective.merge_trace, "target.max_tasks").unwrap();
    assert_eq!(max_tasks.selected_layer, ConfigSource::Default);
    assert_eq!(
        max_tasks.reason,
        stutter_config::source::MergeReason::DefaultValue
    );
}

#[test]
fn merge_config_sources_checked_uses_override_even_when_override_equals_builtin_default() {
    let mut base = MonitorConfig::default();
    base.timing.summary_period_ms = 333;
    base.focus.focus_source = FocusSource::Hybrid;
    base.focus.foreground_source = ForegroundSource::Sway;
    base.focus.auto_focus_min_confidence = 0.75;

    let override_config = MonitorConfig::default();
    let merged = merge_config_sources_checked(ConfigSources {
        defaults: DefaultConfig { config: base },
        user_file: None,
        preset: None,
        overrides: CliOverrides {
            layer: MonitorConfigLayer::from_monitor_config(override_config),
        }
        .into(),
    })
    .unwrap();

    assert_eq!(merged.timing.summary_period_ms, 1_000);
    assert_eq!(merged.focus.focus_source, FocusSource::Heuristic);
    assert_eq!(merged.focus.foreground_source, ForegroundSource::Auto);
    assert_eq!(merged.focus.auto_focus_min_confidence, 0.60);
}

#[test]
fn merge_config_sources_checked_propagates_invalid_user_layer() {
    let user_file = crate::config_file::UserConfigFile {
        focus_source: Some("invalid".to_owned()),
        ..Default::default()
    };

    let err = merge_config_sources_checked(ConfigSources {
        defaults: DefaultConfig::default(),
        user_file: Some(user_file),
        preset: None,
        overrides: CliOverrides {
            layer: MonitorConfigLayer::default(),
        }
        .into(),
    })
    .unwrap_err();

    assert!(matches!(
        err,
        crate::config::ConfigError::InvalidUserLayer(_)
    ));
}

#[test]
fn merge_rejects_zero_summary_period_from_api_override() {
    assert_api_override_invalid_field(
        MonitorConfigLayer {
            summary_period_ms: Some(0),
            ..Default::default()
        },
        "timing.summary_period_ms",
    );
}

#[test]
fn merge_rejects_zero_target_max_tasks_from_api_override() {
    assert_api_override_invalid_field(
        MonitorConfigLayer {
            max_tasks: Some(0),
            ..Default::default()
        },
        "target.max_tasks",
    );
}

#[test]
fn merge_rejects_target_max_tasks_above_shared_limit_from_api_override() {
    assert_api_override_invalid_field(
        MonitorConfigLayer {
            max_tasks: Some(TARGET_PIDS_MAX + 1),
            ..Default::default()
        },
        "target.max_tasks",
    );
}

#[test]
fn merge_rejects_ringbuf_size_below_loader_minimum_from_api_override() {
    assert_api_override_invalid_field(
        MonitorConfigLayer {
            ringbuf_size_kb: Some(Some(63)),
            ..Default::default()
        },
        "ebpf_sizing.ringbuf_size_kb",
    );
}

#[test]
fn merge_rejects_zero_wakeup_map_factor_from_api_override() {
    assert_api_override_invalid_field(
        MonitorConfigLayer {
            wakeup_map_factor: Some(Some(0)),
            ..Default::default()
        },
        "ebpf_sizing.wakeup_map_factor",
    );
}

#[test]
fn ebpf_sizing_rejects_zero_extended_fields() {
    for (expected_field, layer) in [
        (
            "ebpf_sizing.target_pids_entries",
            MonitorConfigLayer {
                target_pids_entries: Some(Some(0)),
                ..Default::default()
            },
        ),
        (
            "ebpf_sizing.target_cgroup_ids_entries",
            MonitorConfigLayer {
                target_cgroup_ids_entries: Some(Some(0)),
                ..Default::default()
            },
        ),
        (
            "ebpf_sizing.target_irqs_entries",
            MonitorConfigLayer {
                target_irqs_entries: Some(Some(0)),
                ..Default::default()
            },
        ),
        (
            "ebpf_sizing.runnable_task_cpu_factor",
            MonitorConfigLayer {
                runnable_task_cpu_factor: Some(Some(0)),
                ..Default::default()
            },
        ),
        (
            "ebpf_sizing.prev_faults_factor",
            MonitorConfigLayer {
                prev_faults_factor: Some(Some(0)),
                ..Default::default()
            },
        ),
        (
            "ebpf_sizing.irq_start_entries",
            MonitorConfigLayer {
                irq_start_entries: Some(Some(0)),
                ..Default::default()
            },
        ),
        (
            "ebpf_sizing.block_start_entries",
            MonitorConfigLayer {
                block_start_entries: Some(Some(0)),
                ..Default::default()
            },
        ),
        (
            "ebpf_sizing.kms_flip_start_entries",
            MonitorConfigLayer {
                kms_flip_start_entries: Some(Some(0)),
                ..Default::default()
            },
        ),
        (
            "ebpf_sizing.drm_fence_wait_start_entries",
            MonitorConfigLayer {
                drm_fence_wait_start_entries: Some(Some(0)),
                ..Default::default()
            },
        ),
        (
            "ebpf_sizing.drm_fence_signal_entries",
            MonitorConfigLayer {
                drm_fence_signal_entries: Some(Some(0)),
                ..Default::default()
            },
        ),
    ] {
        assert_api_override_invalid_field(layer, expected_field);
    }
}

#[test]
fn target_pids_entries_must_cover_target_max_tasks() {
    assert_api_override_invalid_field(
        MonitorConfigLayer {
            max_tasks: Some(256),
            target_pids_entries: Some(Some(128)),
            ..Default::default()
        },
        "ebpf_sizing.target_pids_entries",
    );
}

#[test]
fn ebpf_sizing_rejects_absurd_large_correlation_maps() {
    assert_api_override_invalid_field(
        MonitorConfigLayer {
            block_start_entries: Some(Some(1_048_577)),
            ..Default::default()
        },
        "ebpf_sizing.block_start_entries",
    );
}

#[test]
fn ebpf_sizing_rejects_absurd_large_scaled_maps() {
    assert_api_override_invalid_field(
        MonitorConfigLayer {
            target_pids_entries: Some(Some(2_048)),
            runnable_task_cpu_factor: Some(Some(1_000)),
            ..Default::default()
        },
        "ebpf_sizing.runnable_task_cpu_factor",
    );
}

#[test]
fn monitor_config_rejects_zero_mangohud_timing() {
    for (expected_field, layer) in [
        (
            "mangohud.tail_idle_sleep_ms",
            MonitorConfigLayer {
                mangohud_tail_idle_sleep_ms: Some(0),
                ..Default::default()
            },
        ),
        (
            "mangohud.alignment_poll_ms",
            MonitorConfigLayer {
                mangohud_alignment_poll_ms: Some(0),
                ..Default::default()
            },
        ),
    ] {
        assert_api_override_invalid_field(layer, expected_field);
    }
}

#[test]
fn monitor_config_rejects_absurd_mangohud_timing() {
    for (expected_field, layer) in [
        (
            "mangohud.tail_idle_sleep_ms",
            MonitorConfigLayer {
                mangohud_tail_idle_sleep_ms: Some(5_001),
                ..Default::default()
            },
        ),
        (
            "mangohud.alignment_poll_ms",
            MonitorConfigLayer {
                mangohud_alignment_poll_ms: Some(10_001),
                ..Default::default()
            },
        ),
    ] {
        assert_api_override_invalid_field(layer, expected_field);
    }
}

#[test]
fn monitor_config_rejects_zero_desktop_alert_timeout() {
    assert_api_override_invalid_field(
        MonitorConfigLayer {
            alert_desktop_timeout_ms: Some(0),
            ..Default::default()
        },
        "alerts.desktop_timeout_ms",
    );
}

#[test]
fn monitor_config_rejects_absurd_desktop_alert_timeout() {
    assert_api_override_invalid_field(
        MonitorConfigLayer {
            alert_desktop_timeout_ms: Some(120_001),
            ..Default::default()
        },
        "alerts.desktop_timeout_ms",
    );
}

#[test]
fn merge_rejects_empty_otel_service_name_from_api_override() {
    assert_api_override_invalid_field(
        MonitorConfigLayer {
            otel_service_name: Some("  ".to_owned()),
            ..Default::default()
        },
        "outputs.otel_service_name",
    );
}

#[test]
fn merge_rejects_zero_live_diagnosis_window_from_api_override() {
    assert_api_override_invalid_field(
        MonitorConfigLayer {
            live_diagnosis_cluster_window_ms: Some(0),
            ..Default::default()
        },
        "diagnosis.live_cluster_window_ms",
    );
}

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
fn merge_config_sources_effective_checked_reports_user_preset_and_cli_precedence() {
    let user_file = crate::config_file::UserConfigFile {
        hwmon: Some(false),
        ..Default::default()
    };
    let preset = PresetConfig {
        layer: MonitorConfigLayer {
            hwmon: Some(true),
            ..Default::default()
        },
    };
    let cli = CliOverrides {
        layer: MonitorConfigLayer {
            hwmon: Some(false),
            ..Default::default()
        },
    };

    let effective = merge_config_sources_effective_checked(ConfigSources {
        defaults: DefaultConfig::default(),
        user_file: Some(user_file),
        preset: Some(preset),
        overrides: cli.into(),
    })
    .unwrap();

    assert!(!effective.config.probes.hwmon);
    assert_eq!(
        last_source_for_field(&effective.provenance, "probes.hwmon"),
        Some(ConfigSource::Cli)
    );

    let sources: Vec<_> = effective
        .provenance
        .iter()
        .filter(|entry| entry.field == "probes.hwmon")
        .map(|entry| entry.source)
        .collect();

    assert!(sources.contains(&ConfigSource::Default));
    assert!(sources.contains(&ConfigSource::UserFile));
    assert!(sources.contains(&ConfigSource::Preset));
    assert!(sources.contains(&ConfigSource::Cli));
}

#[test]
fn merge_config_sources_effective_checked_reports_preset_over_user_file() {
    let user_file = crate::config_file::UserConfigFile {
        summary_period_ms: Some(250),
        ..Default::default()
    };
    let preset = PresetConfig {
        layer: MonitorConfigLayer {
            summary_period_ms: Some(500),
            ..Default::default()
        },
    };

    let effective = merge_config_sources_effective_checked(ConfigSources {
        defaults: DefaultConfig::default(),
        user_file: Some(user_file),
        preset: Some(preset),
        overrides: CliOverrides {
            layer: MonitorConfigLayer::default(),
        }
        .into(),
    })
    .unwrap();

    assert_eq!(effective.config.timing.summary_period_ms, 500);
    assert_eq!(
        last_source_for_field(&effective.provenance, "timing.summary_period_ms"),
        Some(ConfigSource::Preset)
    );

    let sources: Vec<_> = effective
        .provenance
        .iter()
        .filter(|entry| entry.field == "timing.summary_period_ms")
        .map(|entry| entry.source)
        .collect();

    assert!(sources.contains(&ConfigSource::Default));
    assert!(sources.contains(&ConfigSource::UserFile));
    assert!(sources.contains(&ConfigSource::Preset));
}

#[test]
fn merge_config_sources_effective_checked_reports_api_override_over_user_file() {
    let user_file = crate::config_file::UserConfigFile {
        summary_period_ms: Some(250),
        ..Default::default()
    };
    let api = ApiOverrides {
        layer: MonitorConfigLayer {
            summary_period_ms: Some(444),
            ..Default::default()
        },
    };

    let effective = merge_config_sources_effective_checked(ConfigSources {
        defaults: DefaultConfig::default(),
        user_file: Some(user_file),
        preset: None,
        overrides: api.into(),
    })
    .unwrap();

    assert_eq!(effective.config.timing.summary_period_ms, 444);
    assert_eq!(
        last_source_for_field(&effective.provenance, "timing.summary_period_ms"),
        Some(ConfigSource::Api)
    );

    let sources: Vec<_> = effective
        .provenance
        .iter()
        .filter(|entry| entry.field == "timing.summary_period_ms")
        .map(|entry| entry.source)
        .collect();

    assert!(sources.contains(&ConfigSource::Default));
    assert!(sources.contains(&ConfigSource::UserFile));
    assert!(sources.contains(&ConfigSource::Api));
}

#[test]
fn merge_config_sources_effective_checked_reports_api_override_source() {
    let api = ApiOverrides {
        layer: MonitorConfigLayer {
            summary_period_ms: Some(444),
            ..Default::default()
        },
    };

    let effective = merge_config_sources_effective_checked(ConfigSources {
        defaults: DefaultConfig::default(),
        user_file: None,
        preset: None,
        overrides: api.into(),
    })
    .unwrap();

    assert_eq!(effective.config.timing.summary_period_ms, 444);
    assert_eq!(
        last_source_for_field(&effective.provenance, "timing.summary_period_ms"),
        Some(ConfigSource::Api)
    );
}

#[test]
fn ebpf_sizing_block_start_entries_runtime_override_wins() {
    let user_file = crate::config_file::UserConfigFile {
        ebpf_sizing: Some(crate::config_file::EbpfSizingConfigFile {
            block_start_entries: Some(16_384),
            ..Default::default()
        }),
        ..Default::default()
    };
    let preset = PresetConfig {
        layer: MonitorConfigLayer {
            block_start_entries: Some(Some(32_768)),
            ..Default::default()
        },
    };
    let cli = CliOverrides {
        layer: MonitorConfigLayer {
            block_start_entries: Some(Some(65_536)),
            ..Default::default()
        },
    };

    let effective = merge_config_sources_effective_checked(ConfigSources {
        defaults: DefaultConfig::default(),
        user_file: Some(user_file),
        preset: Some(preset),
        overrides: cli.into(),
    })
    .unwrap();

    assert_eq!(
        effective.config.ebpf_sizing.block_start_entries,
        Some(65_536)
    );
    assert_eq!(
        last_source_for_field(&effective.provenance, "ebpf_sizing.block_start_entries"),
        Some(ConfigSource::Cli)
    );
}

#[test]
fn ebpf_sizing_target_irqs_entries_user_config_is_preserved_without_override() {
    let user_file = crate::config_file::UserConfigFile {
        ebpf_sizing: Some(crate::config_file::EbpfSizingConfigFile {
            target_irqs_entries: Some(256),
            ..Default::default()
        }),
        ..Default::default()
    };

    let effective = merge_config_sources_effective_checked(ConfigSources {
        defaults: DefaultConfig::default(),
        user_file: Some(user_file),
        preset: None,
        overrides: CliOverrides {
            layer: MonitorConfigLayer::default(),
        }
        .into(),
    })
    .unwrap();

    assert_eq!(effective.config.ebpf_sizing.target_irqs_entries, Some(256));
    assert_eq!(
        last_source_for_field(&effective.provenance, "ebpf_sizing.target_irqs_entries"),
        Some(ConfigSource::UserFile)
    );
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
fn mangohud_timing_defaults_preserve_existing_behavior() {
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

    assert_eq!(merged.mangohud.tail_idle_sleep_ms, 75);
    assert_eq!(merged.mangohud.alignment_poll_ms, 500);
    assert_eq!(merged.alerts.desktop_timeout_ms, 10_000);
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

#[test]
fn merge_config_sources_checked_api_default_values_and_false_override_lower_layers() {
    let mut default_config = MonitorConfig::default();
    default_config.timing.summary_period_ms = 333;
    default_config.focus.focus_source = FocusSource::Hybrid;
    default_config.focus.foreground_source = ForegroundSource::Sway;
    default_config.focus.auto_focus_min_confidence = 0.75;
    default_config.probes.hwmon = true;
    default_config.hwmon.enabled = true;

    let api = ApiOverrides {
        layer: MonitorConfigLayer {
            summary_period_ms: Some(1_000),
            focus_source: Some(FocusSource::Heuristic),
            foreground_source: Some(ForegroundSource::Auto),
            auto_focus_min_confidence: Some(0.60),
            hwmon: Some(false),
            ..Default::default()
        },
    };

    let merged = merge_config_sources_checked(ConfigSources {
        defaults: DefaultConfig {
            config: default_config,
        },
        user_file: None,
        preset: None,
        overrides: api.into(),
    })
    .unwrap();

    assert_eq!(merged.timing.summary_period_ms, 1_000);
    assert_eq!(merged.focus.focus_source, FocusSource::Heuristic);
    assert_eq!(merged.focus.foreground_source, ForegroundSource::Auto);
    assert_eq!(merged.focus.auto_focus_min_confidence, 0.60);
    assert!(!merged.probes.hwmon);
    assert!(!merged.hwmon.enabled);
}

#[test]
fn merge_config_sources_checked_clearable_options_clear_lower_layer_values() {
    let mut default_config = MonitorConfig::default();
    default_config.recording.run_name = Some("baseline".to_owned());
    default_config.outputs.metrics_port = Some(9898);

    let cli = CliOverrides {
        layer: MonitorConfigLayer {
            run_name: Some(None),
            metrics_port: Some(None),
            ..Default::default()
        },
    };

    let merged = merge_config_sources_checked(ConfigSources {
        defaults: DefaultConfig {
            config: default_config,
        },
        user_file: None,
        preset: None,
        overrides: cli.into(),
    })
    .unwrap();

    assert_eq!(merged.recording.run_name, None);
    assert_eq!(merged.outputs.metrics_port, None);
}

#[test]
fn user_file_no_cpu_freq_overrides_cpu_freq_true() {
    let user_file = crate::config_file::UserConfigFile {
        cpu_freq: Some(true),
        no_cpu_freq: Some(true),
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

    assert!(!merged.probes.cpu_freq);
}
