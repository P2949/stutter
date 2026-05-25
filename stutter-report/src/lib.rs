#![forbid(unsafe_code)]

//! Report model, loading, analysis, diffing, and rendering scaffolding.
//!
//! This crate is intentionally independent from the main `stutter` runtime crate.
//! Existing report implementation remains in `stutter::report` until a future migration.

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
        model::ReportModel,
        render::{ReportRenderFormat, render_report_model},
    };

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
}
