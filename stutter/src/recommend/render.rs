use super::model::BaselineTuneRecommendation;
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

    pushln(&mut out, "");
    pushln(&mut out, "## Formal A/B statistics");
    pushln(&mut out, "");
    if rec.formal_metrics.is_empty() {
        pushln(&mut out, "- none");
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
                &mut out,
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
                pushln(&mut out, format!("  - {reason}"));
            }
        }
    }
    pushln(&mut out, "");
    pushln(&mut out, "## Warnings");
    pushln(&mut out, "");
    if rec.warnings.is_empty() {
        pushln(&mut out, "- none");
    } else {
        for warning in &rec.warnings {
            pushln(&mut out, format!("- {warning}"));
        }
    }
    pushln(&mut out, "");
    pushln(&mut out, "## Next steps");
    pushln(&mut out, "");
    for step in &rec.next_steps {
        pushln(&mut out, format!("- {step}"));
    }
    out
}

#[cfg(test)]
pub(crate) fn render_tune_recommendation_for_summary(summary: &TuneSummary) -> String {
    tune::recommendation::render_tune_recommendation_markdown(
        &tune::recommendation::build_tune_recommendation(summary, None),
    )
}

fn pushln(out: &mut String, line: impl AsRef<str>) {
    out.push_str(line.as_ref());
    out.push('\n');
}
