use crate::{config::source::ConfigSource, config_file::UserConfigFile};

pub const CURRENT_CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigDiagnosticLevel {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDiagnostic {
    pub source: ConfigSource,
    pub level: ConfigDiagnosticLevel,
    pub field: Option<String>,
    pub message: String,
}

impl ConfigDiagnostic {
    pub fn new(
        source: ConfigSource,
        level: ConfigDiagnosticLevel,
        field: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            source,
            level,
            field,
            message: message.into(),
        }
    }

    pub fn warning(
        source: ConfigSource,
        field: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(source, ConfigDiagnosticLevel::Warning, field, message)
    }

    pub fn error(source: ConfigSource, field: Option<String>, message: impl Into<String>) -> Self {
        Self::new(source, ConfigDiagnosticLevel::Error, field, message)
    }
}

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
