use super::{
    effective::{self, EffectiveMonitorConfig},
    layer::MonitorConfigLayer,
    model::MonitorConfig,
    source::ConfigSource,
};
use crate::config::ConfigError;

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
mod tests;
