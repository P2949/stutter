use anyhow::Context;

use super::super::*;

pub(crate) fn render_html_report(model: &HtmlReportModel) -> anyhow::Result<String> {
    let model_json = escape_json_for_script_tag(
        &serde_json::to_string(model).context("failed to serialize HTML report model")?,
    );
    let session_json = escape_json_for_script_tag(
        &serde_json::to_string(&model.session).context("failed to serialize HTML session data")?,
    );
    let spike_events_json = escape_json_for_script_tag(
        &serde_json::to_string(&model.spike_events)
            .context("failed to serialize HTML spike event data")?,
    );
    let spike_density_json = escape_json_for_script_tag(
        &serde_json::to_string(&model.spike_density)
            .context("failed to serialize HTML spike density data")?,
    );
    let artifacts_json = escape_json_for_script_tag(
        &serde_json::to_string(&model.chart_artifacts)
            .context("failed to serialize HTML chart artifact data")?,
    );
    let cluster_analysis_json = escape_json_for_script_tag(
        &serde_json::to_string(&model.cluster_analysis)
            .context("failed to serialize HTML cluster data")?,
    );

    let template = include_str!("../../report_template.html");

    Ok(template
        .replace("{html_report_model_json}", &model_json)
        .replace("{session_json}", &session_json)
        .replace("{spike_events_json}", &spike_events_json)
        .replace("{spike_density_json}", &spike_density_json)
        .replace("{artifacts_json}", &artifacts_json)
        .replace("{cluster_analysis_json}", &cluster_analysis_json)
        .replace("{top}", &model.top_limit.to_string()))
}

fn escape_json_for_script_tag(s: &str) -> String {
    s.replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace("</", "<\\/")
}

#[cfg(test)]
mod tests {
    use super::escape_json_for_script_tag;

    #[test]
    fn html_json_escaping_remains_script_tag_safe() {
        assert_eq!(
            escape_json_for_script_tag(r#"<script>&</script>"#),
            r#"\u003cscript\u003e\u0026\u003c/script\u003e"#
        );
    }
}
