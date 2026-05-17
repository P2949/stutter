use std::io;

use thiserror::Error;

/// Error type for shared configuration parsing and resolution.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to parse config source '{source}': {message}", source = source_name)]
    Parse {
        source_name: String,
        message: String,
    },

    #[error("invalid value for config field '{field}': {value} ({reason})")]
    InvalidValue {
        field: String,
        value: String,
        reason: String,
    },

    #[error("missing required config field '{field}'")]
    MissingRequiredField { field: String },

    #[error("conflicting config layers for field '{field}': '{first}' conflicts with '{second}'")]
    ConflictingLayers {
        field: String,
        first: String,
        second: String,
    },

    #[error("unsupported config setting '{setting}' from source '{source}'", source = source_name)]
    UnsupportedSetting {
        source_name: String,
        setting: String,
    },

    #[error("I/O error while reading config source '{source}'")]
    IoSource {
        source: String,
        #[source]
        error: io::Error,
    },
}

impl ConfigError {
    pub fn parse(source: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Parse {
            source_name: source.into(),
            message: message.into(),
        }
    }

    pub fn invalid_value(
        field: impl Into<String>,
        value: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self::InvalidValue {
            field: field.into(),
            value: value.into(),
            reason: reason.into(),
        }
    }

    pub fn missing_required_field(field: impl Into<String>) -> Self {
        Self::MissingRequiredField {
            field: field.into(),
        }
    }

    pub fn conflicting_layers(
        field: impl Into<String>,
        first: impl Into<String>,
        second: impl Into<String>,
    ) -> Self {
        Self::ConflictingLayers {
            field: field.into(),
            first: first.into(),
            second: second.into(),
        }
    }

    pub fn unsupported_setting(source: impl Into<String>, setting: impl Into<String>) -> Self {
        Self::UnsupportedSetting {
            source_name: source.into(),
            setting: setting.into(),
        }
    }

    pub fn io_source(source: impl Into<String>, error: io::Error) -> Self {
        Self::IoSource {
            source: source.into(),
            error,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, io};

    use super::ConfigError;

    #[test]
    fn parse_error_formats_source_and_message() {
        let error = ConfigError::parse("stutter.toml", "expected table");

        assert_eq!(
            error.to_string(),
            "failed to parse config source 'stutter.toml': expected table"
        );

        assert!(
            error.source().is_none(),
            "parse errors with string messages must not expose a fake source error"
        );
    }

    #[test]
    fn invalid_value_error_formats_field_value_and_reason() {
        let error = ConfigError::invalid_value(
            "autotune.mode",
            "unsafe-live",
            "mode is not supported by this build",
        );

        assert_eq!(
            error.to_string(),
            "invalid value for config field 'autotune.mode': unsafe-live (mode is not supported by this build)"
        );
    }

    #[test]
    fn missing_required_field_error_formats_field() {
        let error = ConfigError::missing_required_field("paths.runs_dir");

        assert_eq!(
            error.to_string(),
            "missing required config field 'paths.runs_dir'"
        );
    }

    #[test]
    fn conflicting_layers_error_formats_sources_and_field() {
        let error = ConfigError::conflicting_layers(
            "paths.agent_socket",
            "config-file",
            "daemon-policy-override",
        );

        assert_eq!(
            error.to_string(),
            "conflicting config layers for field 'paths.agent_socket': 'config-file' conflicts with 'daemon-policy-override'"
        );
    }

    #[test]
    fn unsupported_setting_error_formats_source_and_setting() {
        let error = ConfigError::unsupported_setting("api-override", "kernel.raw_sysctl");

        assert_eq!(
            error.to_string(),
            "unsupported config setting 'kernel.raw_sysctl' from source 'api-override'"
        );

        assert!(
            error.source().is_none(),
            "unsupported setting errors with string labels must not expose a fake source error"
        );
    }

    #[test]
    fn io_source_error_wraps_io_error_as_source() {
        let error = ConfigError::io_source(
            "/etc/stutter/config.toml",
            io::Error::new(io::ErrorKind::NotFound, "missing file"),
        );

        assert_eq!(
            error.to_string(),
            "I/O error while reading config source '/etc/stutter/config.toml'"
        );

        let source = match error.source() {
            Some(source) => source,
            None => panic!("expected I/O source error"),
        };

        assert_eq!(source.to_string(), "missing file");
    }
}
