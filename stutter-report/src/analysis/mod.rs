use serde::{Deserialize, Serialize};

use crate::model::ReportModel;

/// Analysis summary derived from a [`ReportModel`].
///
/// All fields are pure derivations from the model with no external crate dependencies.
/// The main-crate analysis pipeline (spike clustering, display-path diagnosis, etc.) produces
/// richer analysis but requires `stutter` runtime types; those remain in the main crate.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReportAnalysis {
    /// The run identifier, if present.
    pub run_id: Option<String>,
    /// Whether the model has a source path (used to check that load captured it).
    pub has_source_path: bool,
    /// Overall stutter score, if computed.
    pub score: Option<f64>,
    /// p95 scheduling latency in nanoseconds, if available.
    pub p95_latency_ns: Option<u64>,
    /// p99 scheduling latency in nanoseconds, if available.
    pub p99_latency_ns: Option<u64>,
    /// Top culprit task comm, if identified.
    pub top_culprit: Option<String>,
    /// Data quality level as a string ("High", "Medium", "Low"), if known.
    pub data_quality_level: Option<String>,
    /// Whether the model has a populated header section.
    pub has_header: bool,
    /// Number of spike clusters in the model.
    pub cluster_count: usize,
    /// Number of frame diagnoses in the model.
    pub frame_count: usize,
    /// Whether the model has correlation sections.
    pub has_correlations: bool,
}

impl ReportAnalysis {
    pub fn from_model(model: &ReportModel) -> Self {
        Self {
            run_id: model.run_id().map(|run_id| run_id.as_str().to_owned()),
            has_source_path: model.source_path().is_some(),
            score: model.score,
            p95_latency_ns: model.p95_latency_ns,
            p99_latency_ns: model.p99_latency_ns,
            top_culprit: model.top_culprit.clone(),
            data_quality_level: model.data_quality.as_ref().map(|q| format!("{:?}", q.level)),
            has_header: model.header.is_some(),
            cluster_count: model.clusters.len(),
            frame_count: model.frames.len(),
            has_correlations: model
                .correlations
                .as_ref()
                .is_some_and(|c| !c.sections.is_empty()),
        }
    }
}

/// Build an analysis summary from the report model.
pub fn analyze_report_model(model: &ReportModel) -> ReportAnalysis {
    ReportAnalysis::from_model(model)
}

#[cfg(test)]
mod tests {
    use stutter_core::{ids::RunId, paths::LogicalPath};

    use super::analyze_report_model;
    use crate::model::{DataQualityLevel, DataQualitySummary, ReportModel, SpikeCluster, SpikePoint};

    fn minimal_data_quality(level: DataQualityLevel) -> DataQualitySummary {
        DataQualitySummary {
            level,
            reasons: Vec::new(),
            missing_optional_files: Vec::new(),
            validation_errors: Vec::new(),
            validation_warnings: Vec::new(),
            probe_activation_warnings: Vec::new(),
            schema_version: 22,
            expected_schema_version: 22,
            event_stream_write_errors: 0,
            spike_events_truncated: false,
            spike_events_retained_count: 0,
            spike_events_dropped_count: 0,
            interval_record_count: 0,
            active_target_pids_count: 0,
            drop_counters_nonzero: false,
            percentile_scope_counts: Default::default(),
            block_io_correlation_basis: "none".to_owned(),
            block_io_correlation_confidence: "high".to_owned(),
            block_io_correlation_warning: None,
            frame_timestamp_alignment: "none".to_owned(),
            cpu_perf_requested: false,
            cpu_perf_open_errors: 0,
            cpu_perf_read_errors: 0,
            cpu_perf_skipped_tasks: 0,
        }
    }

    fn minimal_cluster() -> SpikeCluster {
        SpikeCluster {
            points: vec![SpikePoint {
                task: 1,
                class: "game".to_owned(),
                process_pid: None,
                comm: "render".to_owned(),
                cpu: 0,
                wakeup_target_cpu: 0,
                latency_ns: 1_000_000,
                wakeup_ns: 1_000_000,
                switch_ns: 2_000_000,
                target_pending_wakeups: 0,
                observed_runnable_depth: 0,
                switch_prev_pid: 0,
                switch_prev_state: 1,
                switch_prev_state_label: "S".to_owned(),
                scx_ops: None,
                primary_cause: None,
                cause_tags: Vec::new(),
            }],
            distinct_tasks: 1,
            min_switch_ns: 2_000_000,
            max_switch_ns: 2_000_000,
            max_latency_ns: 1_000_000,
            diagnosis: None,
            wake_graph: Vec::new(),
        }
    }

    #[test]
    fn analysis_summarizes_available_model_identity() {
        let model = ReportModel::new()
            .with_run_id(RunId::new("run-001"))
            .with_source_path(LogicalPath::new("runs/run-001"));

        let analysis = analyze_report_model(&model);

        assert_eq!(analysis.run_id.as_deref(), Some("run-001"));
        assert!(analysis.has_source_path);
    }

    #[test]
    fn analysis_extracts_score_and_latencies() {
        let mut model = ReportModel::new().with_run_id(RunId::new("run-001"));
        model.score = Some(42.5);
        model.p95_latency_ns = Some(1_000_000);
        model.p99_latency_ns = Some(2_000_000);
        model.top_culprit = Some("render".to_owned());

        let analysis = analyze_report_model(&model);

        assert_eq!(analysis.score, Some(42.5));
        assert_eq!(analysis.p95_latency_ns, Some(1_000_000));
        assert_eq!(analysis.p99_latency_ns, Some(2_000_000));
        assert_eq!(analysis.top_culprit.as_deref(), Some("render"));
    }

    #[test]
    fn analysis_reports_data_quality_level() {
        let mut model = ReportModel::new();
        model.data_quality = Some(minimal_data_quality(DataQualityLevel::High));

        let analysis = analyze_report_model(&model);

        assert_eq!(analysis.data_quality_level.as_deref(), Some("High"));

        let mut low_model = ReportModel::new();
        low_model.data_quality = Some(minimal_data_quality(DataQualityLevel::Low));
        let low_analysis = analyze_report_model(&low_model);
        assert_eq!(low_analysis.data_quality_level.as_deref(), Some("Low"));
    }

    #[test]
    fn analysis_counts_clusters_and_frames() {
        let mut model = ReportModel::new();
        model.clusters = vec![minimal_cluster(), minimal_cluster()];
        model.frames = vec![];

        let analysis = analyze_report_model(&model);

        assert_eq!(analysis.cluster_count, 2);
        assert_eq!(analysis.frame_count, 0);
        assert!(!analysis.has_header);
        assert!(!analysis.has_correlations);
    }
}
