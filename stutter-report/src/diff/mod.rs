use serde::{Deserialize, Serialize};

use crate::model::ReportModel;

/// Minimal diff summary placeholder for future report diff migration.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportDiff {
    pub changed: bool,
}

impl ReportDiff {
    pub const fn unchanged() -> Self {
        Self { changed: false }
    }

    pub const fn with_changes() -> Self {
        Self { changed: true }
    }

    pub const fn has_changes(&self) -> bool {
        self.changed
    }
}

/// Compare two skeleton report models.
pub fn diff_report_models(baseline: &ReportModel, current: &ReportModel) -> ReportDiff {
    if baseline == current {
        ReportDiff::unchanged()
    } else {
        ReportDiff::with_changes()
    }
}

#[cfg(test)]
mod tests {
    use stutter_core::ids::RunId;

    use super::diff_report_models;
    use crate::model::ReportModel;

    #[test]
    fn diff_reports_no_change_for_equal_models() {
        let model = ReportModel::new().with_run_id(RunId::new("run-001"));

        let diff = diff_report_models(&model, &model);

        assert!(!diff.has_changes());
    }

    #[test]
    fn diff_reports_change_for_different_models() {
        let baseline = ReportModel::new().with_run_id(RunId::new("run-001"));
        let current = ReportModel::new().with_run_id(RunId::new("run-002"));

        let diff = diff_report_models(&baseline, &current);

        assert!(diff.has_changes());
    }
}
