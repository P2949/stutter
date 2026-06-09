use super::model::{BaselineTuneRecommendation, FixValidationReport};
use crate::tune::uncertainty_html::render_ab_uncertainty_section;
#[cfg(test)]
use crate::tune::{self, TuneSummary};

pub fn render_baseline_tune_recommendation_markdown(rec: &BaselineTuneRecommendation) -> String {
    let mut out = String::new();
    pushln(&mut out, "# stutter baseline/tune recommendation");
    pushln(&mut out, "");
    pushln(&mut out, format!("Verdict: {:?}", rec.verdict));
    pushln(
        &mut out,
        format!(
            "Best profile: {}",
            rec.best_profile.as_deref().unwrap_or("none")
        ),
    );
    pushln(&mut out, format!("Confidence: {:?}", rec.confidence));
    pushln(
        &mut out,
        format!(
            "Baseline runs: {} valid, {} invalid",
            rec.baseline_valid_runs, rec.baseline_invalid_runs
        ),
    );
    if let Some(metadata) = &rec.confidence_metadata {
        pushln(
            &mut out,
            format!(
                "Tune runs: {} configured over {}s epochs after {}s warmup",
                metadata.tune_runs, metadata.tune_epoch_seconds, metadata.tune_warmup_seconds
            ),
        );
        pushln(
            &mut out,
            format!(
                "Best profile runs: {} valid, {} invalid",
                metadata
                    .best_profile_valid_runs
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".to_owned()),
                metadata
                    .best_profile_invalid_runs
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".to_owned())
            ),
        );
        if !metadata.ranking_notes.is_empty() {
            pushln(&mut out, "Confidence notes:");
            for note in &metadata.ranking_notes {
                pushln(&mut out, format!("- {note}"));
            }
        }
    }
    pushln(&mut out, "");
    pushln(&mut out, "## Summary");
    pushln(&mut out, "");
    pushln(&mut out, &rec.summary);
    pushln(&mut out, "");
    pushln(&mut out, "## Scores");
    pushln(&mut out, "");
    pushln(
        &mut out,
        format!(
            "- Baseline score: {}",
            rec.diagnostic_baseline_raw_score_total
        ),
    );
    pushln(
        &mut out,
        format!(
            "- Best median score: {}",
            rec.best_median_diagnostic_raw_score_total
                .map(|score| score.to_string())
                .unwrap_or_else(|| "none".to_owned())
        ),
    );
    if let Some(delta) = rec.score_delta_abs {
        pushln(
            &mut out,
            format!(
                "- Score delta: {} ({})",
                delta,
                rec.score_delta_percent
                    .map(|pct| format!("{pct:.1}%"))
                    .unwrap_or_else(|| "n/a".to_owned())
            ),
        );
    }
    pushln(
        &mut out,
        format!(
            "- Score effect size: {}",
            format_optional_sigma(rec.score_effect_size)
        ),
    );
    pushln(
        &mut out,
        format!(
            "- Score noise ratio: {}",
            format_optional_ratio(rec.score_noise_ratio)
        ),
    );
    pushln(&mut out, "");
    pushln(&mut out, "## Latency details");
    pushln(&mut out, "");
    pushln(
        &mut out,
        format!("- Baseline over 5ms: {}", rec.baseline_over_5ms),
    );
    pushln(
        &mut out,
        format!(
            "- Best median over 5ms: {}",
            rec.best_median_over_5ms
                .map(|v| v.to_string())
                .unwrap_or_else(|| "n/a".to_owned())
        ),
    );
    pushln(
        &mut out,
        format!(
            "- Over 5ms delta: {}",
            rec.over_5ms_delta_abs
                .map(|v| v.to_string())
                .unwrap_or_else(|| "n/a".to_owned())
        ),
    );
    pushln(
        &mut out,
        format!(
            "- Over 5ms effect size: {}",
            format_optional_sigma(rec.over_5ms_effect_size)
        ),
    );
    pushln(
        &mut out,
        format!(
            "- Over 5ms noise ratio: {}",
            format_optional_ratio(rec.over_5ms_noise_ratio)
        ),
    );
    pushln(
        &mut out,
        format!(
            "- Baseline frame p99: {}",
            rec.baseline_frame_p99_ms
                .map(|v| format!("{v:.3}ms"))
                .unwrap_or_else(|| "n/a".to_owned())
        ),
    );
    pushln(
        &mut out,
        format!(
            "- Best median frame p99: {}",
            rec.best_median_frame_p99_ms
                .map(|v| format!("{v:.3}ms"))
                .unwrap_or_else(|| "n/a".to_owned())
        ),
    );
    pushln(
        &mut out,
        format!(
            "- Frame p99 delta: {}",
            rec.frame_p99_delta_ms
                .map(|v| format!("{v:+.3}ms"))
                .unwrap_or_else(|| "n/a".to_owned())
        ),
    );
    pushln(
        &mut out,
        format!(
            "- Frame p99 effect size: {}",
            format_optional_sigma(rec.frame_p99_effect_size)
        ),
    );
    pushln(
        &mut out,
        format!(
            "- Frame p99 noise ratio: {}",
            format_optional_ratio(rec.frame_p99_noise_ratio)
        ),
    );

    push_formal_metrics_markdown(&mut out, rec);
    push_warnings_and_next_steps(&mut out, rec);
    out
}

pub fn render_fix_validation_report_markdown(report: &FixValidationReport) -> String {
    let mut out = String::new();
    pushln(&mut out, "# stutter fix validation");
    pushln(&mut out, "");
    pushln(&mut out, format!("Status: {:?}", report.status));
    pushln(
        &mut out,
        format!("Fix kind: {}", report.fix_plan.kind.as_str()),
    );
    pushln(&mut out, format!("Cause: {:?}", report.fix_plan.cause));
    pushln(
        &mut out,
        format!(
            "Best profile: {}",
            report
                .baseline_tune_recommendation
                .best_profile
                .as_deref()
                .unwrap_or("none")
        ),
    );
    pushln(&mut out, "");
    pushln(&mut out, "## Metric criteria");
    pushln(&mut out, "");
    if report.metric_results.is_empty() {
        pushln(&mut out, "- none");
    } else {
        for result in &report.metric_results {
            pushln(
                &mut out,
                format!(
                    "- {}: expected {}; actual {}; passed={}",
                    result.metric, result.expected, result.actual, result.passed
                ),
            );
        }
    }
    render_string_list(&mut out, "Passed criteria", &report.passed_criteria, "none");
    render_string_list(&mut out, "Failed criteria", &report.failed_criteria, "none");
    render_string_list(&mut out, "Warnings", &report.warnings, "none");
    render_string_list(&mut out, "Next steps", &report.next_steps, "none");
    out
}

pub fn render_baseline_tune_recommendation_html(rec: &BaselineTuneRecommendation) -> String {
    let mut out = String::new();
    pushln(&mut out, "<!doctype html>");
    pushln(&mut out, "<html lang=\"en\">");
    pushln(&mut out, "<head>");
    pushln(&mut out, "<meta charset=\"utf-8\">");
    pushln(
        &mut out,
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
    );
    pushln(
        &mut out,
        "<title>stutter baseline/tune recommendation</title>",
    );
    pushln(
        &mut out,
        "<style>body{font-family:system-ui,sans-serif;line-height:1.45;max-width:1100px;margin:2rem auto;padding:0 1rem}table{border-collapse:collapse;width:100%;margin:1rem 0}th,td{border:1px solid #ccc;padding:.4rem;text-align:left}code{white-space:pre-wrap}</style>",
    );
    pushln(&mut out, "</head>");
    pushln(&mut out, "<body>");
    pushln(&mut out, "<h1>stutter baseline/tune recommendation</h1>");
    pushln(&mut out, "<dl>");
    html_dl_row(&mut out, "Verdict", &format!("{:?}", rec.verdict));
    html_dl_row(
        &mut out,
        "Best profile",
        rec.best_profile.as_deref().unwrap_or("none"),
    );
    html_dl_row(&mut out, "Confidence", &format!("{:?}", rec.confidence));
    html_dl_row(
        &mut out,
        "Baseline runs",
        &format!(
            "{} valid, {} invalid",
            rec.baseline_valid_runs, rec.baseline_invalid_runs
        ),
    );
    if let Some(metadata) = &rec.confidence_metadata {
        html_dl_row(
            &mut out,
            "Tune runs",
            &format!(
                "{} configured over {}s epochs after {}s warmup",
                metadata.tune_runs, metadata.tune_epoch_seconds, metadata.tune_warmup_seconds
            ),
        );
        html_dl_row(
            &mut out,
            "Best profile runs",
            &format!(
                "{} valid, {} invalid",
                metadata
                    .best_profile_valid_runs
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".to_owned()),
                metadata
                    .best_profile_invalid_runs
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "n/a".to_owned())
            ),
        );
    }
    pushln(&mut out, "</dl>");
    pushln(&mut out, "<h2>Summary</h2>");
    pushln(&mut out, format!("<p>{}</p>", escape_html(&rec.summary)));

    pushln(&mut out, "<h2>Unified comparison metrics</h2>");
    pushln(&mut out, "<table>");
    pushln(
        &mut out,
        "<thead><tr><th>Metric</th><th>Baseline</th><th>Best tuned</th><th>Delta</th><th>Effect size</th><th>Noise ratio</th></tr></thead>",
    );
    pushln(&mut out, "<tbody>");
    html_metric_row(
        &mut out,
        "diagnostic_raw_score_total",
        rec.diagnostic_baseline_raw_score_total.to_string(),
        rec.best_median_diagnostic_raw_score_total
            .map(|value| value.to_string())
            .unwrap_or_else(|| "n/a".to_owned()),
        rec.score_delta_abs
            .map(|value| value.to_string())
            .unwrap_or_else(|| "n/a".to_owned()),
        format_optional_sigma(rec.score_effect_size),
        format_optional_ratio(rec.score_noise_ratio),
    );
    html_metric_row(
        &mut out,
        "over_5ms",
        rec.baseline_over_5ms.to_string(),
        rec.best_median_over_5ms
            .map(|value| value.to_string())
            .unwrap_or_else(|| "n/a".to_owned()),
        rec.over_5ms_delta_abs
            .map(|value| value.to_string())
            .unwrap_or_else(|| "n/a".to_owned()),
        format_optional_sigma(rec.over_5ms_effect_size),
        format_optional_ratio(rec.over_5ms_noise_ratio),
    );
    html_metric_row(
        &mut out,
        "frame_p99_ms",
        rec.baseline_frame_p99_ms
            .map(|value| format!("{value:.3}"))
            .unwrap_or_else(|| "n/a".to_owned()),
        rec.best_median_frame_p99_ms
            .map(|value| format!("{value:.3}"))
            .unwrap_or_else(|| "n/a".to_owned()),
        rec.frame_p99_delta_ms
            .map(|value| format!("{value:+.3}"))
            .unwrap_or_else(|| "n/a".to_owned()),
        format_optional_sigma(rec.frame_p99_effect_size),
        format_optional_ratio(rec.frame_p99_noise_ratio),
    );
    pushln(&mut out, "</tbody></table>");

    if let Some(metadata) = &rec.confidence_metadata
        && !metadata.ranking_notes.is_empty()
    {
        pushln(&mut out, "<h2>Confidence notes</h2>");
        pushln(&mut out, "<ul>");
        for note in &metadata.ranking_notes {
            pushln(&mut out, format!("<li>{}</li>", escape_html(note)));
        }
        pushln(&mut out, "</ul>");
    }

    out.push_str(&render_ab_uncertainty_section(
        &rec.formal_metrics,
        &rec.warnings,
    ));

    html_list(&mut out, "Warnings", &rec.warnings, "none");
    html_list(&mut out, "Next steps", &rec.next_steps, "none");
    pushln(&mut out, "</body></html>");
    out
}

pub fn render_fix_validation_report_html(report: &FixValidationReport) -> String {
    let mut out = String::new();
    pushln(&mut out, "<!doctype html>");
    pushln(&mut out, "<html lang=\"en\">");
    pushln(&mut out, "<head>");
    pushln(&mut out, "<meta charset=\"utf-8\">");
    pushln(
        &mut out,
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
    );
    pushln(&mut out, "<title>stutter fix validation</title>");
    pushln(
        &mut out,
        "<style>body{font-family:system-ui,sans-serif;line-height:1.45;max-width:1100px;margin:2rem auto;padding:0 1rem}table{border-collapse:collapse;width:100%;margin:1rem 0}th,td{border:1px solid #ccc;padding:.4rem;text-align:left}code{white-space:pre-wrap}.pass{color:#126b39}.fail{color:#a32929}</style>",
    );
    pushln(&mut out, "</head>");
    pushln(&mut out, "<body>");
    pushln(&mut out, "<h1>stutter fix validation</h1>");
    pushln(&mut out, "<section id=\"fix-validation\">");
    pushln(&mut out, "<h2>Fix validation</h2>");
    pushln(&mut out, "<dl>");
    html_dl_row(&mut out, "Status", &format!("{:?}", report.status));
    html_dl_row(&mut out, "Hypothesis", &report.fix_plan.rationale);
    html_dl_row(&mut out, "Fix kind", report.fix_plan.kind.as_str());
    html_dl_row(&mut out, "Cause", &format!("{:?}", report.fix_plan.cause));
    html_dl_row(
        &mut out,
        "Best profile",
        report
            .baseline_tune_recommendation
            .best_profile
            .as_deref()
            .unwrap_or("none"),
    );
    pushln(&mut out, "</dl>");
    pushln(&mut out, "<table>");
    pushln(
        &mut out,
        "<thead><tr><th>Metric</th><th>Expected</th><th>Actual</th><th>Result</th></tr></thead>",
    );
    pushln(&mut out, "<tbody>");
    for result in &report.metric_results {
        let class = if result.passed { "pass" } else { "fail" };
        let label = if result.passed { "passed" } else { "failed" };
        pushln(
            &mut out,
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td class=\"{}\">{}</td></tr>",
                escape_html(&result.metric),
                escape_html(&result.expected),
                escape_html(&result.actual),
                class,
                label
            ),
        );
    }
    pushln(&mut out, "</tbody></table>");
    html_list(&mut out, "Passed criteria", &report.passed_criteria, "none");
    html_list(&mut out, "Failed criteria", &report.failed_criteria, "none");
    html_list(&mut out, "Warnings", &report.warnings, "none");
    html_list(&mut out, "Next steps", &report.next_steps, "none");
    pushln(&mut out, "</section>");
    pushln(&mut out, "</body></html>");
    out
}

fn push_formal_metrics_markdown(out: &mut String, rec: &BaselineTuneRecommendation) {
    pushln(out, "");
    pushln(out, "## Formal A/B statistics");
    pushln(out, "");
    if rec.formal_metrics.is_empty() {
        pushln(out, "- none");
    } else {
        for metric in &rec.formal_metrics {
            let ci = metric
                .bootstrap_ci95
                .as_ref()
                .map(|ci| format!("95% CI [{:.3}, {:.3}]", ci.lower, ci.upper))
                .unwrap_or_else(|| "95% CI n/a".to_owned());
            let effect = metric
                .effect_size
                .map(|value| format!("{value:.2}σ"))
                .unwrap_or_else(|| "n/a".to_owned());
            pushln(
                out,
                format!(
                    "- {}: baseline_median={:.3}{} tuned_median={:.3}{} improvement={:.3}{} effect_size={} {} enough_samples={} significant={}",
                    metric.metric,
                    metric.baseline_median,
                    metric.unit,
                    metric.tuned_median,
                    metric.unit,
                    metric.improvement_delta,
                    metric.unit,
                    effect,
                    ci,
                    metric.enough_samples,
                    metric.statistically_significant
                ),
            );
            if let Some(reason) = &metric.not_enough_samples_reason {
                pushln(out, format!("  - {reason}"));
            }
            if let Some(power) = &metric.power_estimate {
                let estimate = power
                    .estimated_runs_per_side
                    .map(|runs| format!("{runs} runs per side"))
                    .unwrap_or_else(|| "unavailable".to_owned());
                pushln(
                    out,
                    format!(
                        "  - power estimate for {:.0}% target: {estimate} ({})",
                        power.target_relative_improvement_percent, power.reason
                    ),
                );
            }
        }
    }
}

fn push_warnings_and_next_steps(out: &mut String, rec: &BaselineTuneRecommendation) {
    pushln(out, "");
    pushln(out, "## Warnings");
    pushln(out, "");
    if rec.warnings.is_empty() {
        pushln(out, "- none");
    } else {
        for warning in &rec.warnings {
            pushln(out, format!("- {warning}"));
        }
    }
    pushln(out, "");
    pushln(out, "## Next steps");
    pushln(out, "");
    for step in &rec.next_steps {
        pushln(out, format!("- {step}"));
    }
}

fn render_string_list(out: &mut String, title: &str, items: &[String], empty: &str) {
    pushln(out, "");
    pushln(out, format!("## {title}"));
    pushln(out, "");
    if items.is_empty() {
        pushln(out, format!("- {empty}"));
    } else {
        for item in items {
            pushln(out, format!("- {item}"));
        }
    }
}

#[cfg(test)]
pub(crate) fn render_tune_recommendation_for_summary(summary: &TuneSummary) -> String {
    tune::recommendation::render_tune_recommendation_markdown(
        &tune::recommendation::build_tune_recommendation(summary, None),
    )
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

fn html_metric_row(
    out: &mut String,
    metric: &str,
    baseline: String,
    tuned: String,
    delta: String,
    effect_size: String,
    noise_ratio: String,
) {
    pushln(
        out,
        format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape_html(metric),
            escape_html(&baseline),
            escape_html(&tuned),
            escape_html(&delta),
            escape_html(&effect_size),
            escape_html(&noise_ratio)
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
