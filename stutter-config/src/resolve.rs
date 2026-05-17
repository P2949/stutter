use crate::{
    error::ConfigError,
    layer::{ConfigLayer, ConfigOverride, ConfigProvenance},
    model::ConfigModel,
};

/// Input to runtime configuration resolution.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RuntimeConfigInput {
    pub defaults: ConfigModel,
    pub layers: Vec<ConfigLayer>,
}

impl RuntimeConfigInput {
    pub fn new(defaults: ConfigModel) -> Self {
        Self {
            defaults,
            layers: Vec::new(),
        }
    }

    pub fn with_layers(
        defaults: ConfigModel,
        layers: impl IntoIterator<Item = ConfigLayer>,
    ) -> Self {
        Self {
            defaults,
            layers: layers.into_iter().collect(),
        }
    }

    pub fn add_layer(&mut self, layer: ConfigLayer) {
        self.layers.push(layer);
    }

    pub fn defaults(&self) -> &ConfigModel {
        &self.defaults
    }

    pub fn layers(&self) -> &[ConfigLayer] {
        &self.layers
    }
}

/// Resolved configuration model after applying layers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedConfig {
    pub model: ConfigModel,
    pub provenance: Vec<ConfigProvenance>,
}

impl ResolvedConfig {
    pub fn into_model(self) -> ConfigModel {
        self.model
    }

    pub fn provenance(&self) -> &[ConfigProvenance] {
        &self.provenance
    }
}

/// Resolve a runtime config input by applying its layers in order.
pub fn resolve_runtime_config(input: RuntimeConfigInput) -> Result<ResolvedConfig, ConfigError> {
    resolve_layers(input.defaults, input.layers)
}

/// Apply layers in order, with later layers overriding earlier values.
pub fn resolve_layers(
    defaults: ConfigModel,
    layers: impl IntoIterator<Item = ConfigLayer>,
) -> Result<ResolvedConfig, ConfigError> {
    let mut model = defaults;
    let mut provenance = vec![ConfigProvenance::defaults()];

    for layer in layers {
        apply_layer(&mut model, layer, &mut provenance);
    }

    Ok(ResolvedConfig { model, provenance })
}

fn apply_layer(
    model: &mut ConfigModel,
    layer: ConfigLayer,
    provenance: &mut Vec<ConfigProvenance>,
) {
    let ConfigLayer {
        provenance: layer_provenance,
        overrides,
    } = layer;

    provenance.push(layer_provenance);

    for override_value in overrides {
        match override_value {
            ConfigOverride::Paths(paths) => model.set_paths(paths),
        }
    }
}

#[cfg(test)]
mod tests {
    use stutter_core::paths::StutterPaths;

    use super::{RuntimeConfigInput, resolve_layers, resolve_runtime_config};
    use crate::{
        layer::{ConfigLayer, ConfigProvenance, ConfigSourceKind},
        model::ConfigModel,
    };

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
    fn later_layers_override_paths() {
        let defaults = ConfigModel::with_paths(paths("/default"));
        let layer = ConfigLayer::with_paths_from(
            paths("/layer"),
            ConfigProvenance::config_file("test.toml"),
        );

        let resolved = match resolve_layers(defaults, [layer]) {
            Ok(resolved) => resolved,
            Err(err) => panic!("expected config to resolve, got {err}"),
        };

        let resolved_paths = match resolved.model.paths() {
            Some(paths) => paths,
            None => panic!("expected paths to be resolved"),
        };

        assert_eq!(
            resolved_paths.runs_dir,
            std::path::PathBuf::from("/layer/runs")
        );
        assert_eq!(
            resolved
                .provenance()
                .iter()
                .map(|provenance| provenance.source)
                .collect::<Vec<_>>(),
            vec![ConfigSourceKind::Defaults, ConfigSourceKind::ConfigFile]
        );
    }

    #[test]
    fn empty_layers_keep_defaults() {
        let defaults = ConfigModel::with_paths(paths("/default"));

        let resolved = match resolve_layers(defaults, [ConfigLayer::empty()]) {
            Ok(resolved) => resolved,
            Err(err) => panic!("expected config to resolve, got {err}"),
        };

        let resolved_paths = match resolved.model.paths() {
            Some(paths) => paths,
            None => panic!("expected paths to be resolved"),
        };

        assert_eq!(
            resolved_paths.runs_dir,
            std::path::PathBuf::from("/default/runs")
        );
    }

    #[test]
    fn runtime_input_tracks_all_supported_source_kinds_with_provenance() {
        let input = RuntimeConfigInput::with_layers(
            ConfigModel::with_paths(paths("/default")),
            [
                ConfigLayer::with_paths_from(
                    paths("/file"),
                    ConfigProvenance::config_file("stutter.toml"),
                ),
                ConfigLayer::with_paths_from(paths("/preset"), ConfigProvenance::preset("gaming")),
                ConfigLayer::with_paths_from(paths("/cli"), ConfigProvenance::cli_override()),
                ConfigLayer::with_paths_from(paths("/api"), ConfigProvenance::api_override()),
                ConfigLayer::with_paths_from(
                    paths("/daemon"),
                    ConfigProvenance::daemon_policy_override(),
                ),
                ConfigLayer::with_paths_from(
                    paths("/autotune"),
                    ConfigProvenance::autotune_mode_override(),
                ),
            ],
        );

        let resolved = match resolve_runtime_config(input) {
            Ok(resolved) => resolved,
            Err(err) => panic!("expected runtime config to resolve, got {err}"),
        };

        let resolved_paths = match resolved.model.paths() {
            Some(paths) => paths,
            None => panic!("expected paths to be resolved"),
        };

        assert_eq!(
            resolved_paths.runs_dir,
            std::path::PathBuf::from("/autotune/runs")
        );

        assert_eq!(
            resolved
                .provenance()
                .iter()
                .map(|provenance| provenance.source)
                .collect::<Vec<_>>(),
            vec![
                ConfigSourceKind::Defaults,
                ConfigSourceKind::ConfigFile,
                ConfigSourceKind::Preset,
                ConfigSourceKind::CliOverride,
                ConfigSourceKind::ApiOverride,
                ConfigSourceKind::DaemonPolicyOverride,
                ConfigSourceKind::AutotuneModeOverride,
            ]
        );

        assert_eq!(
            resolved.provenance()[1].label.as_deref(),
            Some("stutter.toml")
        );
        assert_eq!(resolved.provenance()[2].label.as_deref(), Some("gaming"));
    }

    #[test]
    fn runtime_input_builder_accepts_incremental_layers() {
        let mut input = RuntimeConfigInput::new(ConfigModel::with_paths(paths("/default")));
        input.add_layer(ConfigLayer::with_paths_from(
            paths("/cli"),
            ConfigProvenance::cli_override(),
        ));

        assert_eq!(
            input.defaults().paths().map(|paths| &paths.runs_dir),
            Some(&std::path::PathBuf::from("/default/runs"))
        );
        assert_eq!(input.layers().len(), 1);

        let resolved = match resolve_runtime_config(input) {
            Ok(resolved) => resolved,
            Err(err) => panic!("expected runtime config to resolve, got {err}"),
        };

        let resolved_paths = match resolved.model.paths() {
            Some(paths) => paths,
            None => panic!("expected paths to be resolved"),
        };

        assert_eq!(
            resolved_paths.runs_dir,
            std::path::PathBuf::from("/cli/runs")
        );
    }
}
