use stutter_core::paths::StutterPaths;

use crate::model::ConfigModel;

/// Source category for a configuration layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum ConfigSourceKind {
    Defaults,
    ConfigFile,
    Preset,
    CliOverride,
    ApiOverride,
    DaemonPolicyOverride,
    AutotuneModeOverride,
}

impl ConfigSourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Defaults => "defaults",
            Self::ConfigFile => "config-file",
            Self::Preset => "preset",
            Self::CliOverride => "cli-override",
            Self::ApiOverride => "api-override",
            Self::DaemonPolicyOverride => "daemon-policy-override",
            Self::AutotuneModeOverride => "autotune-mode-override",
        }
    }
}

/// Provenance for a configuration layer or override.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigProvenance {
    pub source: ConfigSourceKind,
    pub label: Option<String>,
}

impl ConfigProvenance {
    pub fn new(source: ConfigSourceKind, label: Option<impl Into<String>>) -> Self {
        Self {
            source,
            label: label.map(Into::into),
        }
    }

    pub fn from_source(source: ConfigSourceKind) -> Self {
        Self {
            source,
            label: None,
        }
    }

    pub fn defaults() -> Self {
        Self::from_source(ConfigSourceKind::Defaults)
    }

    pub fn config_file(label: impl Into<String>) -> Self {
        Self::new(ConfigSourceKind::ConfigFile, Some(label))
    }

    pub fn preset(label: impl Into<String>) -> Self {
        Self::new(ConfigSourceKind::Preset, Some(label))
    }

    pub fn cli_override() -> Self {
        Self::from_source(ConfigSourceKind::CliOverride)
    }

    pub fn api_override() -> Self {
        Self::from_source(ConfigSourceKind::ApiOverride)
    }

    pub fn daemon_policy_override() -> Self {
        Self::from_source(ConfigSourceKind::DaemonPolicyOverride)
    }

    pub fn autotune_mode_override() -> Self {
        Self::from_source(ConfigSourceKind::AutotuneModeOverride)
    }
}

/// A single value override inside a configuration layer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigOverride {
    Paths(StutterPaths),
}

impl ConfigOverride {
    pub fn paths(paths: StutterPaths) -> Self {
        Self::Paths(paths)
    }
}

/// Optional configuration layer applied over a base model.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigLayer {
    pub provenance: ConfigProvenance,
    pub overrides: Vec<ConfigOverride>,
}

impl ConfigLayer {
    pub fn empty() -> Self {
        Self {
            provenance: ConfigProvenance::defaults(),
            overrides: Vec::new(),
        }
    }

    pub fn from_source(source: ConfigSourceKind) -> Self {
        Self {
            provenance: ConfigProvenance::from_source(source),
            overrides: Vec::new(),
        }
    }

    pub fn from_provenance(provenance: ConfigProvenance) -> Self {
        Self {
            provenance,
            overrides: Vec::new(),
        }
    }

    pub fn with_paths(paths: StutterPaths) -> Self {
        Self::with_paths_from(paths, ConfigProvenance::cli_override())
    }

    pub fn with_paths_from(paths: StutterPaths, provenance: ConfigProvenance) -> Self {
        Self {
            provenance,
            overrides: vec![ConfigOverride::paths(paths)],
        }
    }

    pub fn with_override(override_value: ConfigOverride, provenance: ConfigProvenance) -> Self {
        Self {
            provenance,
            overrides: vec![override_value],
        }
    }

    pub fn with_overrides(
        overrides: impl IntoIterator<Item = ConfigOverride>,
        provenance: ConfigProvenance,
    ) -> Self {
        Self {
            provenance,
            overrides: overrides.into_iter().collect(),
        }
    }

    pub fn add_override(&mut self, override_value: ConfigOverride) {
        self.overrides.push(override_value);
    }

    pub fn paths(&self) -> Option<&StutterPaths> {
        match self.overrides.first()? {
            ConfigOverride::Paths(paths) => Some(paths),
        }
    }

    pub fn source_kind(&self) -> ConfigSourceKind {
        self.provenance.source
    }

    pub fn provenance(&self) -> &ConfigProvenance {
        &self.provenance
    }

    pub fn overrides(&self) -> &[ConfigOverride] {
        &self.overrides
    }
}

impl Default for ConfigLayer {
    fn default() -> Self {
        Self::empty()
    }
}

impl From<ConfigModel> for ConfigLayer {
    fn from(model: ConfigModel) -> Self {
        let mut layer = Self::from_source(ConfigSourceKind::Defaults);
        if let Some(paths) = model.paths {
            layer.add_override(ConfigOverride::paths(paths));
        }
        layer
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use stutter_core::paths::StutterPaths;

    use super::{ConfigLayer, ConfigProvenance, ConfigSourceKind};

    fn paths(root: &str) -> StutterPaths {
        StutterPaths::new(
            format!("{root}/state"),
            format!("{root}/config"),
            format!("{root}/cache"),
            format!("{root}/runs"),
            format!("{root}/audit.jsonl"),
            format!("{root}/daemon-state.json"),
            format!("{root}/agent.sock"),
        )
    }

    #[test]
    fn source_kind_names_are_stable() {
        assert_eq!(ConfigSourceKind::Defaults.as_str(), "defaults");
        assert_eq!(ConfigSourceKind::ConfigFile.as_str(), "config-file");
        assert_eq!(ConfigSourceKind::Preset.as_str(), "preset");
        assert_eq!(ConfigSourceKind::CliOverride.as_str(), "cli-override");
        assert_eq!(ConfigSourceKind::ApiOverride.as_str(), "api-override");
        assert_eq!(
            ConfigSourceKind::DaemonPolicyOverride.as_str(),
            "daemon-policy-override"
        );
        assert_eq!(
            ConfigSourceKind::AutotuneModeOverride.as_str(),
            "autotune-mode-override"
        );
    }

    #[test]
    fn layer_records_paths_and_provenance() {
        let layer = ConfigLayer::with_paths_from(
            paths("/config"),
            ConfigProvenance::config_file("test.toml"),
        );

        assert_eq!(layer.source_kind(), ConfigSourceKind::ConfigFile);
        assert_eq!(layer.provenance().label.as_deref(), Some("test.toml"));

        let paths = match layer.paths() {
            Some(paths) => paths,
            None => panic!("expected paths override"),
        };

        assert_eq!(paths.runs_dir, PathBuf::from("/config/runs"));
    }

    #[test]
    fn empty_layer_has_defaults_provenance_and_no_overrides() {
        let layer = ConfigLayer::empty();

        assert_eq!(layer.source_kind(), ConfigSourceKind::Defaults);
        assert!(layer.overrides().is_empty());
        assert!(layer.paths().is_none());
    }
}
