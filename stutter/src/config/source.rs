#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    Default,
    UserFile,
    Preset,
    Cli,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldProvenance {
    pub field: &'static str,
    pub source: ConfigSource,
}

impl FieldProvenance {
    pub fn new(field: &'static str, source: ConfigSource) -> Self {
        Self { field, source }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDiagnostic {
    pub message: String,
}

impl ConfigDiagnostic {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}
