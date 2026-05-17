use serde::{Deserialize, Serialize};
use stutter_core::{ids::RunId, paths::LogicalPath, units::UnixNanoseconds};

/// Minimal report domain model placeholder for future report migration.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReportModel {
    pub run_id: Option<RunId>,
    pub source_path: Option<LogicalPath>,
    pub generated_at_unix_nanos: Option<UnixNanoseconds>,
}

impl ReportModel {
    pub const fn new() -> Self {
        Self {
            run_id: None,
            source_path: None,
            generated_at_unix_nanos: None,
        }
    }

    pub fn with_run_id(mut self, run_id: RunId) -> Self {
        self.run_id = Some(run_id);
        self
    }

    pub fn with_source_path(mut self, source_path: LogicalPath) -> Self {
        self.source_path = Some(source_path);
        self
    }

    pub fn with_generated_at_unix_nanos(
        mut self,
        generated_at_unix_nanos: UnixNanoseconds,
    ) -> Self {
        self.generated_at_unix_nanos = Some(generated_at_unix_nanos);
        self
    }

    pub fn run_id(&self) -> Option<&RunId> {
        self.run_id.as_ref()
    }

    pub fn source_path(&self) -> Option<&LogicalPath> {
        self.source_path.as_ref()
    }

    pub fn generated_at_unix_nanos(&self) -> Option<UnixNanoseconds> {
        self.generated_at_unix_nanos
    }
}

#[cfg(test)]
mod tests {
    use stutter_core::{ids::RunId, paths::LogicalPath, units::UnixNanoseconds};

    use super::ReportModel;

    #[test]
    fn report_model_tracks_minimal_identity_source_and_generation_time() {
        let model = ReportModel::new()
            .with_run_id(RunId::new("run-001"))
            .with_source_path(LogicalPath::new("runs/run-001"))
            .with_generated_at_unix_nanos(UnixNanoseconds::new(123));

        assert_eq!(
            model.run_id().map(|run_id| run_id.as_str()),
            Some("run-001")
        );
        assert_eq!(
            model.source_path().map(|path| path.as_str()),
            Some("runs/run-001")
        );
        assert_eq!(
            model
                .generated_at_unix_nanos()
                .map(UnixNanoseconds::as_u128),
            Some(123)
        );
    }
}
