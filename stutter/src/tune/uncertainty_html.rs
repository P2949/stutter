//! Shared HTML rendering for formal A/B uncertainty.
//!
//! This deliberately uses inline SVG instead of external chart dependencies so
//! recommendation reports remain self-contained and safe to open offline.

use super::statistics::FormalMetricComparison;

pub(crate) fn render_ab_uncertainty_section(
    metrics: &[FormalMetricComparison],
    report_warnings: &[String],
) -> String {
    let mut out = String::new();

    pushln(&mut out, "<section id=\"ab-uncertainty-section\">");
    pushln(&mut out, "<h2>A/B uncertainty</h2>");
    pushln(
        &mut out,
        "<p class=\"muted\">Each chart shows baseline/tuned sample distributions and a separate bootstrap confidence interval band for median improvement. Positive improvement means the tuned side was better for lower-is-better metrics.</p>",
    );

    pushln(&mut out, uncertainty_style());

    let underpowered = metrics.iter().any(|metric| metric.underpowered)
        || report_warnings
            .iter()
            .any(|warning| warning.to_ascii_lowercase().contains("not enough samples"));

    if underpowered {
        pushln(
            &mut out,
            "<div class=\"ab-warning\"><strong>Warning:</strong> this comparison is underpowered or noisy. Treat the result as directional until repeated runs produce stable distributions and CIs that exclude zero.</div>",
        );
    }

    render_metric_table(&mut out, metrics);

    if metrics.is_empty() {
        pushln(&mut out, "<p>no formal A/B metrics available</p>");
    } else {
        for metric in metrics {
            render_metric_card(&mut out, metric);
        }
    }

    pushln(&mut out, "</section>");
    out
}

fn render_metric_table(out: &mut String, metrics: &[FormalMetricComparison]) {
    pushln(out, "<h3>Metric summary</h3>");
    pushln(out, "<table class=\"ab-table\">");
    pushln(
        out,
        "<thead><tr>\
         <th>Metric</th>\
         <th>Samples</th>\
         <th>Baseline median</th>\
         <th>Tuned median</th>\
         <th>Improvement</th>\
         <th>Effect size</th>\
         <th>Noise ratio</th>\
         <th>95% CI</th>\
         <th>Significant</th>\
         <th>Power</th>\
         </tr></thead>",
    );
    pushln(out, "<tbody>");

    for metric in metrics {
        let ci = metric
            .bootstrap_ci95
            .as_ref()
            .map(|ci| format!("[{:.3}, {:.3}]", ci.lower, ci.upper))
            .unwrap_or_else(|| "n/a".to_owned());

        let noise = format!(
            "baseline={} tuned={}",
            format_optional_ratio(metric.baseline_noise_ratio),
            format_optional_ratio(metric.tuned_noise_ratio)
        );

        let power = if let Some(power) = &metric.power_estimate {
            power
                .estimated_runs_per_side
                .map(|runs| format!("{runs} runs/side"))
                .unwrap_or_else(|| "unavailable".to_owned())
        } else if metric.underpowered {
            "underpowered".to_owned()
        } else {
            "ok".to_owned()
        };

        pushln(
            out,
            format!(
                "<tr>\
                 <td>{}</td>\
                 <td>{} / {}</td>\
                 <td>{:.3}{}</td>\
                 <td>{:.3}{}</td>\
                 <td>{:+.3}{}</td>\
                 <td>{}</td>\
                 <td>{}</td>\
                 <td>{}</td>\
                 <td>{}</td>\
                 <td>{}</td>\
                 </tr>",
                escape_html(&metric.metric),
                metric.baseline_samples,
                metric.tuned_samples,
                metric.baseline_median,
                escape_html(&metric.unit),
                metric.tuned_median,
                escape_html(&metric.unit),
                metric.improvement_delta,
                escape_html(&metric.unit),
                escape_html(&format_optional_sigma(metric.effect_size)),
                escape_html(&noise),
                escape_html(&ci),
                metric.statistically_significant,
                escape_html(&power),
            ),
        );
    }

    pushln(out, "</tbody></table>");
}

fn render_metric_card(out: &mut String, metric: &FormalMetricComparison) {
    pushln(out, "<article class=\"ab-card\">");
    pushln(out, format!("<h3>{}</h3>", escape_html(&metric.metric)));

    if !metric.uncertainty_warnings.is_empty() {
        pushln(out, "<ul class=\"ab-warning-list\">");
        for warning in &metric.uncertainty_warnings {
            pushln(out, format!("<li>{}</li>", escape_html(warning)));
        }
        pushln(out, "</ul>");
    }
    if let Some(power) = &metric.power_estimate {
        let estimate = power
            .estimated_runs_per_side
            .map(|runs| format!("{runs} runs per side"))
            .unwrap_or_else(|| "unavailable".to_owned());
        pushln(
            out,
            format!(
                "<p class=\"muted\">Sample-size guidance for {:.0}% target: {}. {}</p>",
                power.target_relative_improvement_percent,
                escape_html(&estimate),
                escape_html(&power.reason)
            ),
        );
    }

    pushln(out, "<div class=\"ab-chart-grid\">");
    pushln(out, render_distribution_svg(metric));
    pushln(out, render_ci_svg(metric));
    pushln(out, "</div>");
    pushln(out, "</article>");
}

fn render_distribution_svg(metric: &FormalMetricComparison) -> String {
    let values = metric
        .baseline_values
        .iter()
        .chain(metric.tuned_values.iter())
        .copied()
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();

    if values.is_empty() {
        return "<p class=\"muted\">No finite samples for distribution chart.</p>".to_owned();
    }

    let (min, max) = padded_range(&values);
    let mut svg = String::new();

    pushln(
        &mut svg,
        "<svg class=\"ab-svg\" viewBox=\"0 0 760 220\" role=\"img\">",
    );
    pushln(
        &mut svg,
        format!(
            "<title>{} sample distribution</title>",
            escape_html(&metric.metric)
        ),
    );
    pushln(
        &mut svg,
        "<text x=\"20\" y=\"24\">Sample distribution</text>",
    );
    pushln(
        &mut svg,
        "<line x1=\"80\" y1=\"170\" x2=\"720\" y2=\"170\" class=\"axis\"/>",
    );
    pushln(&mut svg, "<text x=\"20\" y=\"84\">baseline</text>");
    pushln(&mut svg, "<text x=\"20\" y=\"134\">tuned</text>");

    pushln(
        &mut svg,
        format!(
            "<text x=\"80\" y=\"195\">{:.3}{}</text>",
            min,
            escape_html(&metric.unit)
        ),
    );
    pushln(
        &mut svg,
        format!(
            "<text x=\"660\" y=\"195\">{:.3}{}</text>",
            max,
            escape_html(&metric.unit)
        ),
    );

    for (idx, value) in metric.baseline_values.iter().enumerate() {
        let x = scale_x(*value, min, max);
        let y = 76.0 + (idx % 5) as f64 * 4.0;
        pushln(
            &mut svg,
            format!("<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"4\" class=\"baseline-point\"/>"),
        );
    }

    for (idx, value) in metric.tuned_values.iter().enumerate() {
        let x = scale_x(*value, min, max);
        let y = 126.0 + (idx % 5) as f64 * 4.0;
        pushln(
            &mut svg,
            format!("<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"4\" class=\"tuned-point\"/>"),
        );
    }

    let baseline_median_x = scale_x(metric.baseline_median, min, max);
    let tuned_median_x = scale_x(metric.tuned_median, min, max);

    pushln(
        &mut svg,
        format!(
            "<line x1=\"{baseline_median_x:.1}\" y1=\"58\" x2=\"{baseline_median_x:.1}\" y2=\"106\" class=\"median-line\"/>"
        ),
    );
    pushln(
        &mut svg,
        format!(
            "<line x1=\"{tuned_median_x:.1}\" y1=\"112\" x2=\"{tuned_median_x:.1}\" y2=\"160\" class=\"median-line\"/>"
        ),
    );

    pushln(&mut svg, "</svg>");
    svg
}

fn render_ci_svg(metric: &FormalMetricComparison) -> String {
    let Some(ci) = &metric.bootstrap_ci95 else {
        return "<p class=\"muted\">No CI band: not enough samples.</p>".to_owned();
    };

    let values = vec![ci.lower, ci.upper, metric.improvement_delta, 0.0];
    let (min, max) = padded_range(&values);

    let zero_x = scale_x(0.0, min, max);
    let lower_x = scale_x(ci.lower, min, max);
    let upper_x = scale_x(ci.upper, min, max);
    let improvement_x = scale_x(metric.improvement_delta, min, max);
    let band_x = lower_x.min(upper_x);
    let band_w = (upper_x - lower_x).abs().max(1.0);

    let mut svg = String::new();
    pushln(
        &mut svg,
        "<svg class=\"ab-svg\" viewBox=\"0 0 760 180\" role=\"img\">",
    );
    pushln(
        &mut svg,
        format!(
            "<title>{} bootstrap confidence interval</title>",
            escape_html(&metric.metric)
        ),
    );
    pushln(
        &mut svg,
        "<text x=\"20\" y=\"24\">Bootstrap median-improvement CI</text>",
    );
    pushln(
        &mut svg,
        "<line x1=\"80\" y1=\"90\" x2=\"720\" y2=\"90\" class=\"axis\"/>",
    );
    pushln(
        &mut svg,
        format!(
            "<line x1=\"{zero_x:.1}\" y1=\"50\" x2=\"{zero_x:.1}\" y2=\"130\" class=\"zero-line\"/>"
        ),
    );
    pushln(
        &mut svg,
        format!(
            "<rect x=\"{band_x:.1}\" y=\"70\" width=\"{band_w:.1}\" height=\"40\" class=\"ci-band\"/>"
        ),
    );
    pushln(
        &mut svg,
        format!(
            "<line x1=\"{improvement_x:.1}\" y1=\"60\" x2=\"{improvement_x:.1}\" y2=\"120\" class=\"improvement-line\"/>"
        ),
    );
    pushln(
        &mut svg,
        format!(
            "<text x=\"80\" y=\"150\">CI [{:.3}, {:.3}] {}; improvement {:+.3}{}</text>",
            ci.lower,
            ci.upper,
            escape_html(&metric.unit),
            metric.improvement_delta,
            escape_html(&metric.unit)
        ),
    );
    pushln(&mut svg, "</svg>");
    svg
}

fn uncertainty_style() -> &'static str {
    r#"
<style>
#ab-uncertainty-section .ab-warning {
  margin: 1rem 0;
  padding: .75rem 1rem;
  border-left: 4px solid #b45309;
  background: #fffbeb;
}
#ab-uncertainty-section .ab-card {
  margin: 1.25rem 0;
  padding: 1rem;
  border: 1px solid #ddd;
  border-radius: .5rem;
}
#ab-uncertainty-section .ab-chart-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
  gap: 1rem;
}
#ab-uncertainty-section .ab-svg {
  width: 100%;
  max-width: 760px;
  background: #fff;
  border: 1px solid #e5e7eb;
  border-radius: .5rem;
}
#ab-uncertainty-section .axis { stroke: #6b7280; stroke-width: 1; }
#ab-uncertainty-section .zero-line { stroke: #111827; stroke-width: 1; stroke-dasharray: 4 3; }
#ab-uncertainty-section .baseline-point { fill: #2563eb; opacity: .75; }
#ab-uncertainty-section .tuned-point { fill: #16a34a; opacity: .75; }
#ab-uncertainty-section .median-line { stroke: #111827; stroke-width: 2; }
#ab-uncertainty-section .ci-band { fill: #fbbf24; opacity: .45; }
#ab-uncertainty-section .improvement-line { stroke: #dc2626; stroke-width: 2; }
#ab-uncertainty-section .ab-warning-list { color: #92400e; }
</style>
"#
}

fn padded_range(values: &[f64]) -> (f64, f64) {
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);

    if !min.is_finite() || !max.is_finite() {
        return (0.0, 1.0);
    }

    if (max - min).abs() <= f64::EPSILON {
        let pad = min.abs().max(1.0) * 0.10;
        return (min - pad, max + pad);
    }

    let pad = (max - min) * 0.10;
    (min - pad, max + pad)
}

fn scale_x(value: f64, min: f64, max: f64) -> f64 {
    let width = 640.0;
    let left = 80.0;
    if (max - min).abs() <= f64::EPSILON {
        return left + width / 2.0;
    }
    left + ((value - min) / (max - min)).clamp(0.0, 1.0) * width
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
