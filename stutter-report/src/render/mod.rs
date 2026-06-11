use crate::model::ReportModel;

pub mod html;
pub mod text;

/// Supported report render targets for the migrated rendering boundary.
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

/// Render a report model to the requested format.
///
/// For `Text`, outputs a "stutter report" header followed by the full text
/// body produced by [`text::render_report`].
///
/// For `Html`, outputs a self-contained basic HTML report with the same core
/// sections as the migrated text renderer: identity, summary, header, data
/// quality, spike clusters, frame diagnosis, and correlations when those fields
/// are present in the model.
pub fn render_report_model(model: &ReportModel, format: ReportRenderFormat) -> String {
    let run_id = model
        .run_id()
        .map(|run_id| run_id.as_str())
        .unwrap_or("unknown");

    match format {
        ReportRenderFormat::Text => {
            let mut output = String::new();
            output.push_str("stutter report\n");
            output.push_str("==============\n");
            output.push_str(&format!("run_id: {run_id}\n"));
            let body = text::render_report(model);
            if !body.trim().is_empty() {
                output.push('\n');
                output.push_str(&body);
            }
            output
        }
        ReportRenderFormat::Html => html::render_report(model),
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
    fn text_renderer_outputs_report_identity_and_delegates_text_body() {
        let model = ReportModel::new().with_run_id(RunId::new("run-001"));

        let text = render_report_model(&model, ReportRenderFormat::Text);

        assert!(text.contains("stutter report"));
        assert!(text.contains("run-001"));
    }

    #[test]
    fn html_renderer_delegates_to_real_html_report() {
        let model = ReportModel::new().with_run_id(RunId::new("run-001"));

        let html = render_report_model(&model, ReportRenderFormat::Html);

        assert!(html.contains("<!doctype html>"));
        assert!(html.contains("<html lang=\"en\">"));
        assert!(html.contains("<section id=\"identity\">"));
        assert!(html.contains("run-001"));
    }
}
