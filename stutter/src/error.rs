use std::{error::Error, fmt};

#[derive(Debug)]
pub enum StutterError {
    Config(ConfigError),
    Target(TargetError),
    Ebpf(EbpfError),
    Probe(ProbeError),
    Recording(RecordingError),
    Artifact(ArtifactError),
    Report(ReportError),
    Action(crate::actions::ActionError),
    Remote(RemoteError),
}

impl fmt::Display for StutterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => write!(f, "config error: {error}"),
            Self::Target(error) => write!(f, "target error: {error}"),
            Self::Ebpf(error) => write!(f, "eBPF error: {error}"),
            Self::Probe(error) => write!(f, "probe error: {error}"),
            Self::Recording(error) => write!(f, "recording error: {error}"),
            Self::Artifact(error) => write!(f, "artifact error: {error}"),
            Self::Report(error) => write!(f, "report error: {error}"),
            Self::Action(error) => write!(f, "action error: {error}"),
            Self::Remote(error) => write!(f, "remote error: {error}"),
        }
    }
}

impl Error for StutterError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Action(error) => Some(error),
            Self::Config(_)
            | Self::Target(_)
            | Self::Ebpf(_)
            | Self::Probe(_)
            | Self::Recording(_)
            | Self::Artifact(_)
            | Self::Report(_)
            | Self::Remote(_) => None,
        }
    }
}

impl From<crate::actions::ActionError> for StutterError {
    fn from(error: crate::actions::ActionError) -> Self {
        Self::Action(error)
    }
}

macro_rules! simple_error_type {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct $name {
            pub message: String,
        }

        impl $name {
            pub fn new(message: impl Into<String>) -> Self {
                Self {
                    message: message.into(),
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.message)
            }
        }

        impl Error for $name {}
    };
}

simple_error_type!(ConfigError);
simple_error_type!(TargetError);
simple_error_type!(EbpfError);
simple_error_type!(ProbeError);
simple_error_type!(RecordingError);
simple_error_type!(ArtifactError);
simple_error_type!(ReportError);
simple_error_type!(RemoteError);

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
