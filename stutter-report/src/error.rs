use std::{io, path::PathBuf};

use thiserror::Error;

/// Error type for report loading, analysis, diffing, and rendering.
#[derive(Debug, Error)]
pub enum ReportError {
    #[error("failed to load report input '{}'", path.display())]
    Load {
        path: PathBuf,
        #[source]
        error: io::Error,
    },

    #[error("invalid report model: {message}")]
    InvalidModel { message: String },
}

impl ReportError {
    pub fn load(path: impl Into<PathBuf>, error: io::Error) -> Self {
        Self::Load {
            path: path.into(),
            error,
        }
    }

    pub fn invalid_model(message: impl Into<String>) -> Self {
        Self::InvalidModel {
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, io};

    use super::ReportError;

    #[test]
    fn load_error_wraps_io_source() {
        let error = ReportError::load(
            "runs/run-001/session.json",
            io::Error::new(io::ErrorKind::NotFound, "missing file"),
        );

        assert_eq!(
            error.to_string(),
            "failed to load report input 'runs/run-001/session.json'"
        );

        let source = match error.source() {
            Some(source) => source,
            None => panic!("expected I/O source error"),
        };

        assert_eq!(source.to_string(), "missing file");
    }

    #[test]
    fn invalid_model_error_formats_message() {
        let error = ReportError::invalid_model("missing run id");

        assert_eq!(error.to_string(), "invalid report model: missing run id");
    }
}
