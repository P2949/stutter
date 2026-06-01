use super::{
    TuneSummary,
    statistics::{self, FormalMetricComparison},
};

pub(super) fn formal_metrics_between_profiles(
    summary: &TuneSummary,
    best_profile: &str,
    other_profile: &str,
) -> Vec<FormalMetricComparison> {
    let best_runs = summary
        .candidates
        .iter()
        .filter(|candidate| candidate.profile == best_profile && candidate.valid)
        .collect::<Vec<_>>();
    let other_runs = summary
        .candidates
        .iter()
        .filter(|candidate| candidate.profile == other_profile && candidate.valid)
        .collect::<Vec<_>>();

    vec![
        statistics::compare_lower_is_better_metric(
            "diagnostic_raw_score_total",
            "score_points",
            &other_runs
                .iter()
                .map(|run| run.diagnostic_raw_score_total as f64)
                .collect::<Vec<_>>(),
            &best_runs
                .iter()
                .map(|run| run.diagnostic_raw_score_total as f64)
                .collect::<Vec<_>>(),
        ),
        statistics::compare_lower_is_better_metric(
            "over_5ms",
            "samples",
            &other_runs
                .iter()
                .map(|run| run.over_5ms as f64)
                .collect::<Vec<_>>(),
            &best_runs
                .iter()
                .map(|run| run.over_5ms as f64)
                .collect::<Vec<_>>(),
        ),
        statistics::compare_lower_is_better_metric(
            "frame_p99_ms",
            "ms",
            &other_runs
                .iter()
                .map(|run| run.frame_p99_ms)
                .filter(|value| *value > 0.0)
                .collect::<Vec<_>>(),
            &best_runs
                .iter()
                .map(|run| run.frame_p99_ms)
                .filter(|value| *value > 0.0)
                .collect::<Vec<_>>(),
        ),
        statistics::compare_lower_is_better_metric(
            "frame_over_16ms",
            "frames",
            &other_runs
                .iter()
                .map(|run| run.frame_over_16ms as f64)
                .collect::<Vec<_>>(),
            &best_runs
                .iter()
                .map(|run| run.frame_over_16ms as f64)
                .collect::<Vec<_>>(),
        ),
        statistics::compare_lower_is_better_metric(
            "frame_over_33ms",
            "frames",
            &other_runs
                .iter()
                .map(|run| run.frame_over_33ms as f64)
                .collect::<Vec<_>>(),
            &best_runs
                .iter()
                .map(|run| run.frame_over_33ms as f64)
                .collect::<Vec<_>>(),
        ),
    ]
}

pub(super) fn extend_formal_metric_warnings(
    warnings: &mut Vec<String>,
    metrics: &[FormalMetricComparison],
) {
    for metric in metrics {
        if let Some(reason) = &metric.not_enough_samples_reason {
            warnings.push(format!("{}: {reason}", metric.metric));
        } else if !metric.statistically_significant && metric.metric == "diagnostic_raw_score_total"
        {
            warnings.push(format!(
                "{} bootstrap 95% CI crosses zero; A/B improvement is not statistically significant",
                metric.metric
            ));
        }
    }
}

pub(super) fn extend_formal_metric_why(why: &mut Vec<String>, metrics: &[FormalMetricComparison]) {
    for metric in metrics {
        let ci = metric
            .bootstrap_ci95
            .as_ref()
            .map(|ci| format!("95% CI [{:.3}, {:.3}]", ci.lower, ci.upper))
            .unwrap_or_else(|| "95% CI n/a".to_owned());
        why.push(format!(
            "formal A/B {}: baseline_median={:.3} tuned_median={:.3} improvement={:.3} effect_size={} {} enough_samples={} significant={}",
            metric.metric,
            metric.baseline_median,
            metric.tuned_median,
            metric.improvement_delta,
            format_optional_float(metric.effect_size, 2),
            ci,
            metric.enough_samples,
            metric.statistically_significant
        ));
    }
}

fn format_optional_float(value: Option<f64>, digits: usize) -> String {
    value
        .map(|value| format!("{value:.digits$}"))
        .unwrap_or_else(|| "n/a".to_owned())
}
