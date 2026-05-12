#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    Default,
    UserFile,
    Preset,
    Cli,
    Api,
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
