use super::{
    effective::{self, ConfigError},
    layer::MonitorConfigLayer,
    model::MonitorConfig,
};

#[derive(Debug, Clone, Default)]
pub struct DefaultConfig {
    pub config: MonitorConfig,
}

#[derive(Debug, Clone, Default)]
pub struct PresetConfig {
    pub config: MonitorConfig,
}

#[derive(Debug, Clone, Default)]
pub struct CliOverrides {
    pub config: MonitorConfig,
    pub layer: Option<MonitorConfigLayer>,
}

#[derive(Debug, Clone, Default)]
pub struct ConfigSources {
    pub defaults: DefaultConfig,
    pub user_file: Option<crate::config_file::UserConfigFile>,
    pub preset: Option<PresetConfig>,
    pub cli: CliOverrides,
}

pub fn merge_config_sources(sources: ConfigSources) -> MonitorConfig {
    merge_config_sources_checked(sources).unwrap_or_else(|err| {
        log::warn!("presence_aware_config_merge_failed err={err}");
        MonitorConfig::default()
    })
}

pub fn merge_config_sources_checked(sources: ConfigSources) -> Result<MonitorConfig, ConfigError> {
    let default_config = sources.defaults.config;
    let user_layer = sources
        .user_file
        .as_ref()
        .map(MonitorConfigLayer::from_user_file)
        .transpose()
        .map_err(ConfigError::InvalidUserLayer)?;

    let preset_layer = sources
        .preset
        .map(|preset| MonitorConfigLayer::from_monitor_config(preset.config));

    let cli_layer = sources
        .cli
        .layer
        .unwrap_or_else(|| MonitorConfigLayer::from_monitor_config(sources.cli.config));

    Ok(effective::EffectiveMonitorConfig::from_layers(
        default_config,
        user_layer,
        preset_layer,
        cli_layer,
    )?
    .into_monitor_config())
}

pub fn merge_user_file(config: MonitorConfig) -> MonitorConfig {
    match crate::config_file::load_user_config() {
        Ok(Some(user_file)) => {
            let sources = ConfigSources {
                defaults: DefaultConfig { config },
                user_file: Some(user_file),
                preset: None,
                cli: CliOverrides {
                    config: MonitorConfig::default(),
                    layer: Some(MonitorConfigLayer::default()),
                },
            };
            merge_config_sources(sources)
        }
        Ok(None) => config,
        Err(err) => {
            log::warn!("failed_to_load_user_config_for_monitor_config err={err:#}");
            config
        }
    }
}

pub fn merge_monitor_config(base: MonitorConfig, override_config: MonitorConfig) -> MonitorConfig {
    let sources = ConfigSources {
        defaults: DefaultConfig { config: base },
        user_file: None,
        preset: None,
        cli: CliOverrides {
            config: override_config,
            layer: None,
        },
    };
    merge_config_sources(sources)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{FocusSource, ForegroundSourceArg};

    #[test]
    fn merge_monitor_config_uses_override_even_when_override_equals_builtin_default() {
        let mut base = MonitorConfig::default();
        base.timing.summary_period_ms = 333;
        base.focus.focus_source = FocusSource::Hybrid;
        base.focus.foreground_source = ForegroundSourceArg::Sway;
        base.focus.auto_focus_min_confidence = 0.75;

        let override_config = MonitorConfig::default();
        let merged = merge_monitor_config(base, override_config);

        assert_eq!(merged.timing.summary_period_ms, 1_000);
        assert_eq!(merged.focus.focus_source, FocusSource::Heuristic);
        assert_eq!(merged.focus.foreground_source, ForegroundSourceArg::Auto);
        assert_eq!(merged.focus.auto_focus_min_confidence, 0.60);
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
            cli: CliOverrides {
                config: MonitorConfig::default(),
                layer: Some(cli_layer),
            },
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
        assert_eq!(merged.focus.foreground_source, ForegroundSourceArg::Sway);
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
                config: MonitorConfig::default(),
                layer: Some(MonitorConfigLayer::default()),
            },
        })
        .unwrap();

        assert!(!merged.probes.cpu_freq);
    }
}
