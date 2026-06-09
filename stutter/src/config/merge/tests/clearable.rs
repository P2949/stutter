use super::*;

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
