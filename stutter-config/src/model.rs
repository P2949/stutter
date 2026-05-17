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
}
