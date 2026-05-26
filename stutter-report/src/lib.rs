#![forbid(unsafe_code)]

//! Report model, loading, analysis, diffing, and rendering migration boundary.
//!
//! This crate is intentionally independent from the main `stutter` runtime crate.
//! The remaining main-crate report logic is tracked in
//! `docs/REPORT_CRATE_MIGRATION.md`.

pub mod analysis;
pub mod diff;
pub mod error;
pub mod load;
pub mod model;
pub mod render;

pub use error::ReportError;

#[cfg(test)]
mod tests {
    use std::path::Path;

    use stutter_core::{ids::RunId, paths::LogicalPath, units::UnixNanoseconds};

    use super::{
        analysis::analyze_report_model,
        diff::diff_report_models,
        error::ReportError,
        load::{ReportLoadRequest, load_report_model},
        model::{DataQualityLevel, DataQualitySummary, ReportModel},
        render::{ReportRenderFormat, render_report_model},
    };

    fn minimal_data_quality() -> DataQualitySummary {
        DataQualitySummary {
            level: DataQualityLevel::High,
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

    #[test]
    fn report_crate_exposes_requested_skeleton_modules() {
        let model = ReportModel::new()
            .with_run_id(RunId::new("run-001"))
            .with_source_path(LogicalPath::new("runs/run-001"))
            .with_generated_at_unix_nanos(UnixNanoseconds::new(123));

        let analysis = analyze_report_model(&model);
        assert_eq!(analysis.run_id.as_deref(), Some("run-001"));
        assert!(analysis.has_source_path);

        let unchanged = diff_report_models(&model, &model);
        assert!(!unchanged.has_changes());

        let changed = diff_report_models(&ReportModel::new(), &model);
        assert!(changed.has_changes());

        let text = render_report_model(&model, ReportRenderFormat::Text);
        assert!(text.contains("stutter report"));
        assert!(text.contains("run-001"));
        assert_eq!(ReportRenderFormat::Html.as_str(), "html");

        let request = ReportLoadRequest::from_path("runs/run-001");
        assert_eq!(request.path(), Path::new("runs/run-001"));

        let error = match load_report_model(&request) {
            Ok(_) => panic!("loading a missing file should return an error"),
            Err(error) => error,
        };

        match error {
            ReportError::Load { path, .. } => {
                assert_eq!(path, Path::new("runs/run-001"));
            }
            other => panic!("expected load error, got {other}"),
        }
    }

    #[test]
    fn render_report_model_text_includes_full_body_when_model_has_content() {
        let mut model = ReportModel::new().with_run_id(RunId::new("run-002"));
        model.data_quality = Some(minimal_data_quality());

        let text = render_report_model(&model, ReportRenderFormat::Text);

        assert!(text.contains("stutter report"));
        assert!(text.contains("run-002"));
        // The full renderer should include the data quality section
        assert!(text.contains("data quality"));
        assert!(text.contains("level: High"));
    }

    #[test]
    fn render_report_model_html_stub_contains_run_id() {
        let model = ReportModel::new().with_run_id(RunId::new("run-003"));

        let html = render_report_model(&model, ReportRenderFormat::Html);

        assert!(html.contains("<h1>stutter report</h1>"));
        assert!(html.contains("run-003"));
        assert!(html.starts_with("<!doctype html>"));
    }

    #[test]
    fn analyze_report_model_extracts_all_available_fields() {
        let mut model = ReportModel::new()
            .with_run_id(RunId::new("run-004"))
            .with_source_path(LogicalPath::new("runs/run-004"));
        model.score = Some(75.0);
        model.p95_latency_ns = Some(500_000);
        model.p99_latency_ns = Some(1_500_000);
        model.top_culprit = Some("audio".to_owned());
        model.data_quality = Some(minimal_data_quality());

        let analysis = analyze_report_model(&model);

        assert_eq!(analysis.run_id.as_deref(), Some("run-004"));
        assert!(analysis.has_source_path);
        assert_eq!(analysis.score, Some(75.0));
        assert_eq!(analysis.p95_latency_ns, Some(500_000));
        assert_eq!(analysis.p99_latency_ns, Some(1_500_000));
        assert_eq!(analysis.top_culprit.as_deref(), Some("audio"));
        assert_eq!(analysis.data_quality_level.as_deref(), Some("High"));
        assert!(!analysis.has_header);
        assert_eq!(analysis.cluster_count, 0);
        assert_eq!(analysis.frame_count, 0);
        assert!(!analysis.has_correlations);
    }

    #[test]
    fn full_pipeline_load_analyze_diff_render() {
        // Verify the full pipeline can be exercised end-to-end with only stutter-report types.
        // Load fixture, analyze, diff against empty, render.
        let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/minimal/input.json");

        let model = load_report_model(&ReportLoadRequest::from_path(&fixture_path))
            .expect("minimal fixture should load successfully");

        // Analysis
        let analysis = analyze_report_model(&model);
        assert!(analysis.has_header);
        assert_eq!(analysis.data_quality_level.as_deref(), Some("High"));
        assert_eq!(analysis.cluster_count, 0);

        // Diff against empty model
        let diff = diff_report_models(&ReportModel::new(), &model);
        assert!(diff.has_changes());

        // Diff against itself — no changes
        let no_diff = diff_report_models(&model, &model);
        assert!(!no_diff.has_changes());

        // Render
        let text = render_report_model(&model, ReportRenderFormat::Text);
        assert!(text.contains("stutter report"));
        assert!(text.contains("data quality"));
        assert!(text.contains("High"));
    }
}
