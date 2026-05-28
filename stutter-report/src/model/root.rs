use serde::{Deserialize, Serialize};
use stutter_core::{ids::RunId, paths::LogicalPath, units::UnixNanoseconds};

use super::{
    DataQualitySummary, FrameDiagnosis, ReportHeaderSummary, SpikeCluster,
    TextReportCorrelationSections,
};

/// Report-domain model migrated far enough to support crate-local load/render tests.
///
/// Remaining fields still derived by the main crate are tracked in
/// `docs/REPORT_CRATE_MIGRATION.md`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ReportModel {
    pub run_id: Option<RunId>,
    pub source_path: Option<LogicalPath>,
    pub generated_at_unix_nanos: Option<UnixNanoseconds>,
    pub score: Option<f64>,
    pub p95_latency_ns: Option<u64>,
    pub p99_latency_ns: Option<u64>,
    pub top_culprit: Option<String>,
    pub header: Option<ReportHeaderSummary>,
    pub data_quality: Option<DataQualitySummary>,
    #[serde(default)]
    pub clusters: Vec<SpikeCluster>,
    #[serde(default)]
    pub frames: Vec<FrameDiagnosis>,
    pub correlations: Option<TextReportCorrelationSections>,
}

impl ReportModel {
    pub fn new() -> Self {
        Self {
            run_id: None,
            source_path: None,
            generated_at_unix_nanos: None,
            score: None,
            p95_latency_ns: None,
            p99_latency_ns: None,
            top_culprit: None,
            header: None,
            data_quality: None,
            clusters: Vec::new(),
            frames: Vec::new(),
            correlations: None,
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
