use super::*;

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
