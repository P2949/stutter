use thiserror::Error;

#[derive(Debug, Error)]
pub enum StutterError {
    #[error("config error: {0}")]
    Config(#[from] ConfigError),
    #[error("target error: {0}")]
    Target(#[from] TargetError),
    #[error("eBPF error: {0}")]
    Ebpf(#[from] EbpfError),
    #[error("probe error: {0}")]
    Probe(#[from] ProbeError),
    #[error("recording error: {0}")]
    Recording(#[from] RecordingError),
    #[error("artifact error: {0}")]
    Artifact(#[from] ArtifactError),
    #[error("report error: {0}")]
    Report(#[from] ReportError),
    #[error("action error: {0}")]
    Action(#[from] crate::actions::ActionError),
    #[error("remote error: {0}")]
    Remote(#[from] RemoteError),
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to load user config: {0:#}")]
    UserConfig(#[source] anyhow::Error),
    #[error("failed to parse user config TOML: {0:#}")]
    InvalidUserConfigToml(#[source] anyhow::Error),
    #[error("invalid config_version: {message}")]
    InvalidConfigVersion { message: String },
    #[error("unsupported config_version {version}; current supported version is {current}")]
    UnsupportedConfigVersion { version: u32, current: u32 },
    #[error("failed to resolve monitor preset: {0:#}")]
    InvalidPreset(#[source] anyhow::Error),
    #[error("failed to convert user config to monitor layer: {0:#}")]
    InvalidUserLayer(#[source] anyhow::Error),
    #[error("invalid target filter in {field} pattern {pattern:?}: {source:#}")]
    InvalidTargetFilter {
        field: &'static str,
        pattern: String,
        #[source]
        source: anyhow::Error,
    },
}

#[derive(Debug, Error)]
pub enum TargetError {
    #[error(transparent)]
    Source(#[from] anyhow::Error),
}

#[derive(Debug, Error)]
pub enum EbpfError {
    #[error(transparent)]
    Source(#[from] anyhow::Error),
}

#[derive(Debug, Error)]
pub enum ProbeError {
    #[error(transparent)]
    Source(#[from] anyhow::Error),
}

#[derive(Debug, Error)]
pub enum RecordingError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Source(#[from] anyhow::Error),
}

#[derive(Debug, Error)]
pub enum ArtifactError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Source(#[from] anyhow::Error),
}

#[derive(Debug, Error)]
pub enum ReportError {
    #[error(transparent)]
    Source(#[from] anyhow::Error),
}

#[derive(Debug, Error)]
pub enum RemoteError {
    #[error(transparent)]
    Request(#[from] reqwest::Error),
    #[error(transparent)]
    Source(#[from] anyhow::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeWarning {
    pub probe: String,
    pub message: String,
}

impl ProbeWarning {
    pub fn new(probe: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            probe: probe.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataQualityWarning {
    pub category: String,
    pub message: String,
}

impl DataQualityWarning {
    pub fn new(category: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            category: category.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputWarning {
    pub sink: String,
    pub message: String,
}

impl OutputWarning {
    pub fn new(sink: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            sink: sink.into(),
            message: message.into(),
        }
    }
}
