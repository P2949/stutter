use super::{recommendation::TuneRecommendation, uncertainty_html::render_ab_uncertainty_section};

pub fn render_tune_recommendation_html(rec: &TuneRecommendation) -> String {
    let mut out = String::new();

    pushln(&mut out, "<!doctype html>");
    pushln(&mut out, "<html lang=\"en\">");
    pushln(&mut out, "<head>");
    pushln(&mut out, "<meta charset=\"utf-8\">");
    pushln(
        &mut out,
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
    );
    pushln(&mut out, "<title>stutter tuning recommendation</title>");
    pushln(
        &mut out,
        "<style>body{font-family:system-ui,sans-serif;line-height:1.45;max-width:1200px;margin:2rem auto;padding:0 1rem}table{border-collapse:collapse;width:100%;margin:1rem 0}th,td{border:1px solid #ccc;padding:.4rem;text-align:left}.muted{color:#666}</style>",
    );
    pushln(&mut out, "</head>");
    pushln(&mut out, "<body>");
    pushln(&mut out, "<h1>stutter tuning recommendation</h1>");

    pushln(&mut out, "<dl>");
    html_dl_row(&mut out, "Verdict", &format!("{:?}", rec.verdict));
    html_dl_row(
        &mut out,
        "Best profile",
        rec.best_profile.as_deref().unwrap_or("none"),
    );
    html_dl_row(
        &mut out,
        "Compared against",
        rec.compared_against.as_deref().unwrap_or("none"),
    );
    html_dl_row(&mut out, "Confidence", &format!("{:?}", rec.confidence));
    pushln(&mut out, "</dl>");

    pushln(&mut out, "<h2>Summary</h2>");
    pushln(&mut out, format!("<p>{}</p>", escape_html(&rec.summary)));

    if let Some(metrics) = &rec.best_metrics {
        pushln(&mut out, "<h2>Best profile metrics</h2>");
        pushln(&mut out, "<dl>");
        html_dl_row(&mut out, "Valid runs", &metrics.valid_runs.to_string());
        html_dl_row(&mut out, "Invalid runs", &metrics.invalid_runs.to_string());
        html_dl_row(
            &mut out,
            "Median diagnostic raw score",
            &metrics.median_diagnostic_raw_score_total.to_string(),
        );
        html_dl_row(
            &mut out,
            "IQR diagnostic raw score",
            &metrics.iqr_diagnostic_raw_score_total.to_string(),
        );
        html_dl_row(
            &mut out,
            "Median over 5ms",
            &metrics.median_over_5ms.to_string(),
        );
        html_dl_row(
            &mut out,
            "Median frame p99 us",
            &metrics.median_frame_p99_us.to_string(),
        );
        pushln(&mut out, "</dl>");
    }

    if let Some(comparison) = &rec.comparison_metrics {
        pushln(&mut out, "<h2>Comparison summary</h2>");
        pushln(&mut out, "<dl>");
        html_dl_row(&mut out, "Other profile", &comparison.other_profile);
        html_dl_row(
            &mut out,
            "Score delta",
            &comparison.score_delta_abs.to_string(),
        );
        html_dl_row(
            &mut out,
            "Score effect size",
            &format_optional_sigma(comparison.score_effect_size),
        );
        html_dl_row(
            &mut out,
            "Score noise ratio",
            &format_optional_ratio(comparison.score_noise_ratio),
        );
        html_dl_row(
            &mut out,
            "Over 5ms effect size",
            &format_optional_sigma(comparison.over_5ms_effect_size),
        );
        html_dl_row(
            &mut out,
            "Frame p99 effect size",
            &format_optional_sigma(comparison.frame_p99_effect_size),
        );
        pushln(&mut out, "</dl>");

        out.push_str(&render_ab_uncertainty_section(
            &comparison.formal_metrics,
            &rec.warnings,
        ));
    } else {
        pushln(
            &mut out,
            "<p class=\"muted\">No formal A/B comparison target was available.</p>",
        );
    }

    html_list(&mut out, "Why", &rec.why, "none");
    html_list(&mut out, "Warnings", &rec.warnings, "none");
    html_list(&mut out, "Next steps", &rec.next_steps, "none");

    pushln(&mut out, "</body></html>");
    out
}

fn html_dl_row(out: &mut String, label: &str, value: &str) {
    pushln(
        out,
        format!(
            "<dt>{}</dt><dd>{}</dd>",
            escape_html(label),
            escape_html(value)
        ),
    );
}

fn html_list(out: &mut String, title: &str, items: &[String], empty: &str) {
    pushln(out, format!("<h2>{}</h2>", escape_html(title)));
    if items.is_empty() {
        pushln(out, format!("<p>{}</p>", escape_html(empty)));
        return;
    }
    pushln(out, "<ul>");
    for item in items {
        pushln(out, format!("<li>{}</li>", escape_html(item)));
    }
    pushln(out, "</ul>");
}

fn format_optional_sigma(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.2}σ"))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn format_optional_ratio(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.2}"))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn pushln(out: &mut String, line: impl AsRef<str>) {
    out.push_str(line.as_ref());
    out.push('\n');
}
