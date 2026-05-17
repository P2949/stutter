#![forbid(unsafe_code)]

//! Configuration model scaffolding shared by future `stutter` crates.
//!
//! This crate must remain independent from the main `stutter` application crate.

pub mod error;
pub mod layer;
pub mod model;
pub mod resolve;

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use stutter_core::paths::StutterPaths;

    use super::{
        error::ConfigError, layer::ConfigLayer, model::ConfigModel, resolve::resolve_layers,
    };

    fn test_paths(root: &str) -> StutterPaths {
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
    fn config_crate_exposes_minimal_model_layer_and_resolver() {
        let defaults = ConfigModel::with_paths(test_paths("/default"));
        let override_layer = ConfigLayer::with_paths(test_paths("/override"));

        let resolved = match resolve_layers(defaults, [override_layer]) {
            Ok(resolved) => resolved,
            Err(err) => panic!("expected config layers to resolve, got {err}"),
        };

        let paths = match resolved.model.paths() {
            Some(paths) => paths,
            None => panic!("expected resolved paths"),
        };

        assert_eq!(paths.state_dir, PathBuf::from("/override/state"));
        assert_eq!(paths.config_dir, PathBuf::from("/override/config"));
        assert_eq!(paths.cache_dir, PathBuf::from("/override/cache"));
        assert_eq!(paths.runs_dir, PathBuf::from("/override/runs"));
        assert_eq!(paths.audit_log, PathBuf::from("/override/audit.jsonl"));
        assert_eq!(
            paths.daemon_state,
            PathBuf::from("/override/daemon-state.json")
        );
        assert_eq!(paths.agent_socket, PathBuf::from("/override/agent.sock"));

        let error = ConfigError::missing_required_field("paths.runs_dir");
        assert_eq!(
            error.to_string(),
            "missing required config field 'paths.runs_dir'"
        );
    }
}
