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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeReason {
    DefaultValue,
    LayerValue,
    LaterLayerOverride,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigMergeTrace {
    pub field: &'static str,
    pub selected_layer: ConfigSource,
    pub reason: MergeReason,
}

impl ConfigMergeTrace {
    pub fn new(field: &'static str, selected_layer: ConfigSource, reason: MergeReason) -> Self {
        Self {
            field,
            selected_layer,
            reason,
        }
    }
}
