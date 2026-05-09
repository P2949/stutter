#![allow(dead_code)]

use super::model::MonitorConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    Default,
    UserFile,
    Preset,
    Cli,
}

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
}

#[derive(Debug, Clone, Default)]
pub struct ConfigSources {
    pub defaults: DefaultConfig,
    pub user_file: Option<crate::config_file::UserConfigFile>,
    pub preset: Option<PresetConfig>,
    pub cli: CliOverrides,
}

pub fn merge_config_sources(sources: ConfigSources) -> MonitorConfig {
    let mut config = sources.defaults.config;

    if sources.user_file.is_some() {
        config = merge_user_file(config);
    }

    if let Some(preset) = sources.preset {
        config = merge_monitor_config(config, preset.config);
    }

    merge_monitor_config(config, sources.cli.config)
}

fn merge_user_file(config: MonitorConfig) -> MonitorConfig {
    config
}

fn merge_monitor_config(_base: MonitorConfig, override_config: MonitorConfig) -> MonitorConfig {
    override_config
}
