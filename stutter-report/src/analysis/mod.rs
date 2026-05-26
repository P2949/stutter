use serde::{Deserialize, Serialize};

use crate::model::ReportModel;

/// Migration-boundary analysis summary for crate-local report tests.
///
/// Full report analysis still lives in the main crate until the migration
/// checklist moves each analyzer behind this crate.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportAnalysis {
    pub run_id: Option<String>,
    pub has_source_path: bool,
}

impl ReportAnalysis {
    pub fn from_model(model: &ReportModel) -> Self {
        Self {
            run_id: model.run_id().map(|run_id| run_id.as_str().to_owned()),
            has_source_path: model.source_path().is_some(),
        }
    }
}

/// Build the currently migrated analysis summary from the report model.
pub fn analyze_report_model(model: &ReportModel) -> ReportAnalysis {
    ReportAnalysis::from_model(model)
}

#[cfg(test)]
mod tests {
    use stutter_core::{ids::RunId, paths::LogicalPath};

    use super::analyze_report_model;
    use crate::model::ReportModel;

    #[test]
    fn analysis_summarizes_available_model_identity() {
        let model = ReportModel::new()
            .with_run_id(RunId::new("run-001"))
            .with_source_path(LogicalPath::new("runs/run-001"));

        let analysis = analyze_report_model(&model);

        assert_eq!(analysis.run_id.as_deref(), Some("run-001"));
        assert!(analysis.has_source_path);
    }
}
