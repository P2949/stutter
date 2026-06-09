use crate::source::ConfigSource;

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
