pub use stutter_config::schema::ConfigDiagnostic;
#[cfg(test)]
pub(crate) use stutter_config::schema::ConfigDiagnosticLevel;

use crate::config_file::UserConfigFile;

pub const CURRENT_CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct RawConfigFile {
    pub config_version: Option<u32>,
    pub flattened: toml::Value,
}

#[derive(Debug, Clone)]
pub struct ParsedUserConfigFile {
    pub version: u32,
    pub file: UserConfigFile,
    pub diagnostics: Vec<ConfigDiagnostic>,
}

impl ParsedUserConfigFile {
    pub fn new(version: u32, mut file: UserConfigFile, diagnostics: Vec<ConfigDiagnostic>) -> Self {
        file.config_version = Some(version);
        file.diagnostics = diagnostics.clone();

        Self {
            version,
            file,
            diagnostics,
        }
    }
}
