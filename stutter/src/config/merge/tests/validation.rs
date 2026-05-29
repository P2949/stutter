use super::*;

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
