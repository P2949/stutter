use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::de::DeserializeOwned;

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

pub(crate) fn load_json_file<T: DeserializeOwned>(path: &Path) -> Result<T, ReportError> {
    let file = fs::File::open(path).map_err(|source| ReportError::Load {
        path: path.to_path_buf(),
        error: source,
    })?;
    serde_json::from_reader(file).map_err(|e| ReportError::invalid_model(e.to_string()))
}

/// Load a report model from the requested path.
pub fn load_report_model(request: &ReportLoadRequest) -> Result<ReportModel, ReportError> {
    let model: ReportModel = load_json_file(request.path())?;
    model
        .validate_identity_strings()
        .map_err(|err| ReportError::invalid_model(err.to_string()))?;
    Ok(model)
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::tempdir;

    use super::{ReportLoadRequest, load_report_model};
    use crate::model::ReportModel;

    #[test]
    fn load_request_preserves_source_path() {
        let request = ReportLoadRequest::from_path("runs/run-001");

        assert_eq!(request.path(), Path::new("runs/run-001"));
    }

    #[test]
    fn load_report_model_reads_json_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("report.json");
        let model = ReportModel::new();
        fs::write(&path, serde_json::to_string(&model).unwrap()).unwrap();

        let request = ReportLoadRequest::from_path(&path);
        let loaded = load_report_model(&request).unwrap();
        assert_eq!(loaded.run_id, model.run_id);
    }

    #[test]
    fn load_report_model_rejects_empty_run_id() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("report.json");

        fs::write(
            &path,
            r#"{
                "run_id": "",
                "source_path": null,
                "generated_at_unix_nanos": null,
                "score": null,
                "p95_latency_ns": null,
                "p99_latency_ns": null,
                "top_culprit": null,
                "header": null,
                "data_quality": null,
                "clusters": [],
                "frames": [],
                "correlations": null
            }"#,
        )
        .unwrap();

        let request = ReportLoadRequest::from_path(&path);
        let err = load_report_model(&request).expect_err("empty run_id should be rejected");

        assert!(
            err.to_string().contains("RunId cannot be empty"),
            "unexpected error: {err}"
        );
    }
}
