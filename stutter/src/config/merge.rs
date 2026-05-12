use super::{effective, layer::MonitorConfigLayer, model::MonitorConfig};
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

pub type ApiOverrides = CliOverrides;
pub type RuntimeOverrides = CliOverrides;

#[derive(Debug, Clone, Default)]
pub struct ConfigSources {
    pub defaults: DefaultConfig,
    pub user_file: Option<crate::config_file::UserConfigFile>,
    pub preset: Option<PresetConfig>,
    pub cli: CliOverrides,
}

pub fn merge_config_sources_checked(sources: ConfigSources) -> Result<MonitorConfig, ConfigError> {
    let default_config = sources.defaults.config;
    let user_layer = sources
        .user_file
        .as_ref()
        .map(MonitorConfigLayer::from_user_file)
        .transpose()
        .map_err(ConfigError::InvalidUserLayer)?;

    let preset_layer = sources.preset.map(|preset| preset.layer);

    Ok(effective::EffectiveMonitorConfig::from_layers(
        default_config,
        user_layer,
        preset_layer,
        sources.cli.layer,
    )?
    .into_monitor_config())
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
            cli: CliOverrides {
                layer: MonitorConfigLayer::from_monitor_config(override_config),
            },
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
            cli: CliOverrides {
                layer: MonitorConfigLayer::default(),
            },
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
            cli: CliOverrides {
                layer: MonitorConfigLayer::default(),
            },
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
            cli: CliOverrides { layer: cli_layer },
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
            cli: CliOverrides {
                layer: MonitorConfigLayer::default(),
            },
        })
        .unwrap();

        assert!(!merged.probes.cpu_freq);
    }
}
