use stutter_core::paths::StutterPaths;

use crate::model::ConfigModel;

/// Optional configuration layer applied over a base model.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConfigLayer {
    pub paths: Option<StutterPaths>,
}

impl ConfigLayer {
    pub const fn empty() -> Self {
        Self { paths: None }
    }

    pub fn with_paths(paths: StutterPaths) -> Self {
        Self { paths: Some(paths) }
    }

    pub fn paths(&self) -> Option<&StutterPaths> {
        self.paths.as_ref()
    }
}

impl From<ConfigModel> for ConfigLayer {
    fn from(model: ConfigModel) -> Self {
        Self { paths: model.paths }
    }
}
