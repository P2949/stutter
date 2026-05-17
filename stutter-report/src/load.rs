use std::path::{Path, PathBuf};

use crate::{error::ReportError, model::ReportModel};

/// Request describing where a report model should be loaded from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportLoadRequest {
    path: PathBuf,
}

impl ReportLoadRequest {
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Placeholder load entry point for the future report migration.
pub fn load_report_model(request: &ReportLoadRequest) -> Result<ReportModel, ReportError> {
    Err(ReportError::unsupported_operation(
        "load_report_model",
        format!(
            "report loading for '{}' has not been migrated into stutter-report yet",
            request.path().display()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{ReportLoadRequest, load_report_model};
    use crate::error::ReportError;

    #[test]
    fn load_request_preserves_source_path() {
        let request = ReportLoadRequest::from_path("runs/run-001");

        assert_eq!(request.path(), Path::new("runs/run-001"));
    }

    #[test]
    fn load_report_model_is_explicitly_not_migrated_yet() {
        let request = ReportLoadRequest::from_path("runs/run-001");
        let error = match load_report_model(&request) {
            Ok(_) => panic!("loading should not be implemented in the skeleton crate"),
            Err(error) => error,
        };

        match error {
            ReportError::UnsupportedOperation { operation, reason } => {
                assert_eq!(operation, "load_report_model");
                assert!(reason.contains("runs/run-001"));
                assert!(reason.contains("not been migrated"));
            }
            other => panic!("expected unsupported operation error, got {other}"),
        }
    }
}
