use crate::{error::ConfigError, layer::ConfigLayer, model::ConfigModel};

/// Resolved configuration model after applying layers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedConfig {
    pub model: ConfigModel,
}

impl ResolvedConfig {
    pub fn into_model(self) -> ConfigModel {
        self.model
    }
}

/// Apply layers in order, with later layers overriding earlier values.
pub fn resolve_layers(
    defaults: ConfigModel,
    layers: impl IntoIterator<Item = ConfigLayer>,
) -> Result<ResolvedConfig, ConfigError> {
    let mut model = defaults;

    for layer in layers {
        apply_layer(&mut model, layer);
    }

    Ok(ResolvedConfig { model })
}

fn apply_layer(model: &mut ConfigModel, layer: ConfigLayer) {
    if let Some(paths) = layer.paths {
        model.paths = Some(paths);
    }
}

#[cfg(test)]
mod tests {
    use stutter_core::paths::StutterPaths;

    use super::resolve_layers;
    use crate::{layer::ConfigLayer, model::ConfigModel};

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
        let layer = ConfigLayer::with_paths(paths("/layer"));

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
}
