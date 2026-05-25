use crate::model::ReportModel;

pub mod text;

/// Supported report render targets for the future rendering layer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportRenderFormat {
    Text,
    Html,
}

impl ReportRenderFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Html => "html",
        }
    }
}

/// Render a minimal skeleton report model.
pub fn render_report_model(model: &ReportModel, format: ReportRenderFormat) -> String {
    let run_id = model
        .run_id()
        .map(|run_id| run_id.as_str())
        .unwrap_or("unknown");

    match format {
        ReportRenderFormat::Text => format!("stutter report\nrun_id: {run_id}\n"),
        ReportRenderFormat::Html => {
            format!(
                "<!doctype html>\n<title>stutter report</title>\n<h1>stutter report</h1>\n<p>run_id: {run_id}</p>\n"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use stutter_core::ids::RunId;

    use super::{ReportRenderFormat, render_report_model};
    use crate::model::ReportModel;

    #[test]
    fn render_format_names_are_stable() {
        assert_eq!(ReportRenderFormat::Text.as_str(), "text");
        assert_eq!(ReportRenderFormat::Html.as_str(), "html");
    }

    #[test]
    fn renderer_outputs_minimal_report_identity() {
        let model = ReportModel::new().with_run_id(RunId::new("run-001"));

        let text = render_report_model(&model, ReportRenderFormat::Text);
        assert!(text.contains("stutter report"));
        assert!(text.contains("run-001"));

        let html = render_report_model(&model, ReportRenderFormat::Html);
        assert!(html.contains("<h1>stutter report</h1>"));
        assert!(html.contains("run-001"));
    }
}
