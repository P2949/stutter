use std::path::PathBuf;

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
    #[error("profile error: {0}")]
    Profile(#[from] ProfileError),
    #[error("procfs error: {0}")]
    Procfs(#[from] ProcfsError),
    #[error("eBPF load error: {0}")]
    EbpfLoad(#[from] EbpfLoadError),
    #[error("autotune plan error: {0}")]
    AutotunePlan(#[from] AutotunePlanError),
    #[error("autotune runtime error: {0}")]
    AutotuneRuntime(#[from] AutotuneRuntimeError),
    #[error("daemon policy error: {0}")]
    DaemonPolicy(#[from] DaemonPolicyError),
    #[error("agent error: {0}")]
    Agent(#[from] AgentError),
    #[error("action error: {0}")]
    Action(#[from] crate::actions::ActionError),
    #[error("remote error: {0}")]
    Remote(#[from] RemoteError),
    #[error("command error: {0:#}")]
    Command(#[from] anyhow::Error),
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
    #[error("invalid config value for {field}: {message}")]
    InvalidValue { field: String, message: String },
}

impl From<stutter_config::ConfigError> for ConfigError {
    fn from(error: stutter_config::ConfigError) -> Self {
        match error {
            stutter_config::ConfigError::InvalidValue { field, reason, .. } => Self::InvalidValue {
                field,
                message: reason,
            },
            other => Self::InvalidValue {
                field: "static_config".to_owned(),
                message: other.to_string(),
            },
        }
    }
}

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("failed to read profile {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse profile {path}: {source:#}")]
    Parse {
        path: PathBuf,
        #[source]
        source: anyhow::Error,
    },
    #[error("profile validation failed: {message}")]
    Validation { message: String },
}

#[derive(Debug, Error)]
pub enum ProcfsError {
    #[error("failed to read procfs path {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse procfs field {field}: {message}")]
    Parse {
        field: &'static str,
        message: String,
    },
}

#[derive(Debug, Error)]
pub enum EbpfLoadError {
    #[error("failed to read BPF object {path}: {source}")]
    ReadObject {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("BPF object {path} is empty")]
    EmptyObject { path: PathBuf },
    #[error("eBPF load failed: {message}")]
    Load { message: String },
}

#[derive(Debug, Error)]
pub enum TargetError {
    #[error("invalid cgroup path {path}: {source:#}")]
    InvalidCgroupPath {
        path: PathBuf,
        #[source]
        source: anyhow::Error,
    },
    #[error(
        "native cgroup filtering is unavailable for {path} (resolved directory inode {cgroup_id}); use cgroup PID expansion until a runtime-verified cgroup-id resolver is implemented"
    )]
    NativeCgroupFilterUnsupported { path: PathBuf, cgroup_id: u64 },
    #[error(transparent)]
    Source(#[from] anyhow::Error),
}

#[derive(Debug, Error)]
pub enum EbpfError {
    #[error("failed to load eBPF object: {source:#}")]
    ObjectLoad {
        #[source]
        source: anyhow::Error,
    },
    #[error("failed to initialize eBPF map {map}: {source:#}")]
    MapInit {
        map: &'static str,
        #[source]
        source: anyhow::Error,
    },
    #[error("failed to attach eBPF program {program}: {source:#}")]
    Attach {
        program: &'static str,
        #[source]
        source: anyhow::Error,
    },
    #[error(
        "failed to attach eBPF program {program} to tracepoint {category}/{tracepoint}: {source:#}"
    )]
    TracepointAttach {
        program: &'static str,
        category: String,
        tracepoint: String,
        #[source]
        source: anyhow::Error,
    },
    #[error(transparent)]
    Source(#[from] anyhow::Error),
}

#[derive(Debug, Error)]
pub enum ProbeError {
    #[error("optional probe {probe} program {program} failed to attach: {source:#}")]
    Attach {
        probe: String,
        program: &'static str,
        #[source]
        source: anyhow::Error,
    },
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
    #[error("failed to load report input from {path}: {source:#}")]
    Load {
        path: PathBuf,
        #[source]
        source: anyhow::Error,
    },
}

#[derive(Debug, Error)]
pub enum AutotunePlanError {
    #[error("invalid candidate plan: {message}")]
    InvalidPlan { message: String },
    #[error("candidate denied for apply: {message}")]
    ApplyDenied { message: String },
}

#[derive(Debug, Error)]
pub enum AutotuneRuntimeError {
    #[error("invalid autotune runtime mode: {message}")]
    InvalidMode { message: String },
    #[error("autotune runtime failed: {source:#}")]
    Source {
        #[source]
        source: anyhow::Error,
    },
}

#[derive(Debug, Error)]
pub enum DaemonPolicyError {
    #[error("invalid daemon policy input: {message}")]
    InvalidInput { message: String },
    #[error("daemon policy rejected action: {message}")]
    Rejected { message: String },
}

#[derive(Debug, Error)]
pub enum AgentError {
    #[error("failed to read bearer token file {path}: {source}")]
    BearerTokenFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("bearer token is empty")]
    EmptyBearerToken,
    #[error("agent request is invalid: {message}")]
    InvalidRequest { message: String },
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
