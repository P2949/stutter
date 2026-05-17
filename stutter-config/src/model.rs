use stutter_core::paths::StutterPaths;

/// Minimal configuration model placeholder for future shared config migration.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConfigModel {
    pub paths: Option<StutterPaths>,
}

impl ConfigModel {
    pub const fn new() -> Self {
        Self { paths: None }
    }

    pub fn with_paths(paths: StutterPaths) -> Self {
        Self { paths: Some(paths) }
    }

    pub fn paths(&self) -> Option<&StutterPaths> {
        self.paths.as_ref()
    }

    pub fn set_paths(&mut self, paths: StutterPaths) {
        self.paths = Some(paths);
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use stutter_core::paths::StutterPaths;

    use super::ConfigModel;

    #[test]
    fn config_model_stores_paths() {
        let paths = StutterPaths::new(
            "/state",
            "/config",
            "/cache",
            "/runs",
            "/audit.jsonl",
            "/daemon-state.json",
            "/agent.sock",
        );

        let mut model = ConfigModel::new();
        assert!(model.paths().is_none());

        model.set_paths(paths);

        let paths = match model.paths() {
            Some(paths) => paths,
            None => panic!("expected paths to be set"),
        };

        assert_eq!(paths.runs_dir, PathBuf::from("/runs"));
        assert_eq!(paths.agent_socket, PathBuf::from("/agent.sock"));
    }
}
