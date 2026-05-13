use super::{
    effective::{self, EffectiveMonitorConfig},
    layer::MonitorConfigLayer,
    model::MonitorConfig,
    source::ConfigSource,
};
use crate::error::ConfigError;

#[derive(Debug, Clone, Default)]
pub struct DefaultConfig {
    pub config: MonitorConfig,
}

#[derive(Debug, Clone, Default)]
pub struct PresetConfig {
    pub layer: MonitorConfigLayer,
}

#[derive(Debug, Clone, Default)]
pub struct CliOverrides {
    pub layer: MonitorConfigLayer,
}

#[derive(Debug, Clone, Default)]
pub struct ApiOverrides {
    pub layer: MonitorConfigLayer,
}

#[derive(Debug, Clone)]
pub enum RuntimeOverrides {
    Cli(CliOverrides),
    Api(ApiOverrides),
}

impl Default for RuntimeOverrides {
    fn default() -> Self {
        Self::Cli(CliOverrides::default())
    }
}

impl From<CliOverrides> for RuntimeOverrides {
    fn from(value: CliOverrides) -> Self {
        Self::Cli(value)
    }
}

impl From<ApiOverrides> for RuntimeOverrides {
    fn from(value: ApiOverrides) -> Self {
        Self::Api(value)
    }
}

impl RuntimeOverrides {
    fn into_parts(self) -> (MonitorConfigLayer, ConfigSource) {
        match self {
            Self::Cli(overrides) => (overrides.layer, ConfigSource::Cli),
            Self::Api(overrides) => (overrides.layer, ConfigSource::Api),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ConfigSources {
    pub defaults: DefaultConfig,
    pub user_file: Option<crate::config_file::UserConfigFile>,
    pub preset: Option<PresetConfig>,
    pub overrides: RuntimeOverrides,
}

pub fn merge_config_sources_effective_checked(
    sources: ConfigSources,
) -> Result<EffectiveMonitorConfig, ConfigError> {
    let default_config = sources.defaults.config;
    let diagnostics = sources
        .user_file
        .as_ref()
        .map(|user_file| user_file.diagnostics.clone())
        .unwrap_or_default();
    let user_layer = sources
        .user_file
        .as_ref()
        .map(MonitorConfigLayer::from_user_file)
        .transpose()
        .map_err(ConfigError::InvalidUserLayer)?;

    let preset_layer = sources.preset.map(|preset| preset.layer);
    let (override_layer, override_source) = sources.overrides.into_parts();

    effective::EffectiveMonitorConfig::from_layers_with_sources(
        default_config,
        user_layer,
        preset_layer,
        override_layer,
        override_source,
        diagnostics,
    )
}

pub fn merge_config_sources_checked(sources: ConfigSources) -> Result<MonitorConfig, ConfigError> {
    Ok(merge_config_sources_effective_checked(sources)?.into_monitor_config())
}

#[cfg(test)]
fn merge_config_sources_lossy_for_tests(sources: ConfigSources) -> MonitorConfig {
    merge_config_sources_checked(sources).unwrap_or_else(|err| {
        log::warn!("presence_aware_config_merge_failed err={err}");
        MonitorConfig::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{FocusSource, ForegroundSource};

    fn last_source_for_field(
        provenance: &[crate::config::source::FieldProvenance],
        field: &'static str,
    ) -> Option<ConfigSource> {
        provenance
            .iter()
            .rev()
            .find(|entry| entry.field == field)
            .map(|entry| entry.source)
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
            crate::error::ConfigError::InvalidUserLayer(_)
        ));
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
}
