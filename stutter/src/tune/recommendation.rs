use serde::{Deserialize, Serialize};

use super::{
    RankingConfidence, TuneProfileStats, TuneSummary,
    ranking::{noise_ratio, normalized_effect_size},
    recommendation_formal::{
        extend_formal_metric_warnings, extend_formal_metric_why, formal_metrics_between_profiles,
    },
    statistics::FormalMetricComparison,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TuneRecommendationVerdict {
    Recommended,
    NeedsRetest,
    NoRecommendation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuneRecommendation {
    pub schema_version: u32,
    pub verdict: TuneRecommendationVerdict,
    pub best_profile: Option<String>,
    pub baseline_profile: Option<String>,
    pub compared_against: Option<String>,
    pub confidence: RankingConfidence,
    pub summary: String,
    pub why: Vec<String>,
    pub warnings: Vec<String>,
    pub next_steps: Vec<String>,
    pub best_metrics: Option<TuneRecommendationMetrics>,
    pub comparison_metrics: Option<TuneRecommendationComparison>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuneRecommendationMetrics {
    #[serde(alias = "median_diagnostic_score_total")]
    pub median_diagnostic_raw_score_total: u64,
    #[serde(alias = "iqr_diagnostic_score_total")]
    pub iqr_diagnostic_raw_score_total: u64,
    #[serde(alias = "worst_diagnostic_score_total")]
    pub worst_diagnostic_raw_score_total: u64,
    pub median_over_5ms: u64,
    pub median_frame_p99_us: u64,
    pub valid_runs: usize,
    pub invalid_runs: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TuneRecommendationComparison {
    pub other_profile: String,

    pub score_delta_abs: i64,
    pub score_delta_percent: Option<f64>,
    pub score_effect_size: Option<f64>,
    pub score_noise_ratio: Option<f64>,

    pub over_5ms_delta_abs: i64,
    pub over_5ms_effect_size: Option<f64>,
    pub over_5ms_noise_ratio: Option<f64>,

    pub frame_p99_delta_us: i64,
    pub frame_p99_effect_size: Option<f64>,
    pub frame_p99_noise_ratio: Option<f64>,

    #[serde(default)]
    pub formal_metrics: Vec<FormalMetricComparison>,
}

pub fn build_tune_recommendation(
    summary: &TuneSummary,
    baseline_profile: Option<&str>,
) -> TuneRecommendation {
    let mut warnings = summary.ranking_notes.clone();
    warnings.extend(
        summary
            .comparability_warnings
            .iter()
            .map(|warning| match &warning.profile {
                Some(profile) => format!(
                    "comparability {:?} profile={} kind={}: {}",
                    warning.severity, profile, warning.kind, warning.message
                ),
                None => format!(
                    "comparability {:?} kind={}: {}",
                    warning.severity, warning.kind, warning.message
                ),
            }),
    );
    let mut why = Vec::new();
    let mut next_steps = Vec::new();
    let best_stat = if summary.best_profile.is_empty() {
        None
    } else {
        valid_stat(summary, &summary.best_profile)
    };

    if summary.ranking_confidence == RankingConfidence::Unstable {
        return TuneRecommendation {
            schema_version: 1,
            verdict: TuneRecommendationVerdict::NoRecommendation,
            best_profile: None,
            baseline_profile: baseline_profile.map(ToOwned::to_owned),
            compared_against: None,
            confidence: summary.ranking_confidence,
            summary: "No profile recommendation: ranking was unstable.".to_owned(),
            why,
            warnings,
            next_steps: vec![
                "Rerun with more runs and a comparable workload before applying any profile."
                    .to_owned(),
            ],
            best_metrics: None,
            comparison_metrics: None,
        };
    }

    let Some(best_stat) = best_stat else {
        if summary.best_profile.is_empty() {
            warnings.push("no best profile was selected".to_owned());
        } else {
            warnings.push(format!(
                "best profile '{}' has no valid stats",
                summary.best_profile
            ));
        }
        return TuneRecommendation {
            schema_version: 1,
            verdict: TuneRecommendationVerdict::NoRecommendation,
            best_profile: None,
            baseline_profile: baseline_profile.map(ToOwned::to_owned),
            compared_against: None,
            confidence: summary.ranking_confidence,
            summary: "No profile recommendation: no valid best profile was available.".to_owned(),
            why,
            warnings,
            next_steps: vec![
                "Collect another tuning run with at least two valid profiles.".to_owned(),
            ],
            best_metrics: None,
            comparison_metrics: None,
        };
    };

    if summary.runs < 3 {
        warnings.push(format!(
            "low run count: --runs {} is below the recommended minimum of 3",
            summary.runs
        ));
    }
    if best_stat.invalid_runs > 0 {
        warnings.push(format!(
            "best profile had {} invalid run(s)",
            best_stat.invalid_runs
        ));
    }
    if best_stat.iqr_diagnostic_raw_score_total > 0 {
        warnings.push(format!(
            "best profile score IQR is non-zero ({})",
            best_stat.iqr_diagnostic_raw_score_total
        ));
    }

    let (comparison_kind, comparison_stat) = choose_comparison_stat(
        summary,
        baseline_profile,
        &summary.best_profile,
        &mut warnings,
    );
    let comparison_metrics = comparison_stat.map(|other| {
        let mut metrics = comparison_between(best_stat, other);
        metrics.formal_metrics =
            formal_metrics_between_profiles(summary, &summary.best_profile, &other.profile);
        extend_formal_metric_warnings(&mut warnings, &metrics.formal_metrics);
        extend_formal_metric_why(&mut why, &metrics.formal_metrics);
        why.push(format!(
            "Median score delta versus {} '{}' is {} ({}, effect_size={}, noise_ratio={})",
            comparison_kind.as_deref().unwrap_or("comparison"),
            other.profile,
            metrics.score_delta_abs,
            format_percent(metrics.score_delta_percent),
            format_optional_float(metrics.score_effect_size, 2),
            format_optional_float(metrics.score_noise_ratio, 2),
        ));
        why.push(format!(
            "over_5ms delta versus '{}' is {} (effect_size={}, noise_ratio={})",
            other.profile,
            metrics.over_5ms_delta_abs,
            format_optional_float(metrics.over_5ms_effect_size, 2),
            format_optional_float(metrics.over_5ms_noise_ratio, 2),
        ));
        if best_stat.median_frame_p99_us > 0 || other.median_frame_p99_us > 0 {
            why.push(format!(
                "frame p99 delta versus '{}' is {}us (effect_size={}, noise_ratio={})",
                other.profile,
                metrics.frame_p99_delta_us,
                format_optional_float(metrics.frame_p99_effect_size, 2),
                format_optional_float(metrics.frame_p99_noise_ratio, 2),
            ));
        } else {
            warnings.push("missing frame data for best or comparison profile".to_owned());
        }
        let close_margin = other.median_diagnostic_raw_score_total / 20;
        if metrics.score_delta_abs.unsigned_abs() <= close_margin && close_margin > 0 {
            warnings.push(format!(
                "close score margin versus '{}': delta_abs={} threshold={}",
                other.profile,
                metrics.score_delta_abs.unsigned_abs(),
                close_margin
            ));
        }
        metrics
    });

    if comparison_metrics.is_none() {
        warnings.push("no valid comparison target was available".to_owned());
    }

    why.push(format!(
        "best profile '{}' had {} valid run(s) and {} invalid run(s)",
        best_stat.profile, best_stat.valid_runs, best_stat.invalid_runs
    ));
    why.push(format!(
        "best median score={} IQR={} worst={}",
        best_stat.median_diagnostic_raw_score_total,
        best_stat.iqr_diagnostic_raw_score_total,
        best_stat.worst_diagnostic_raw_score_total
    ));
    if best_stat.median_frame_p99_us == 0 {
        warnings.push("missing frame data for best profile".to_owned());
    }

    let verdict = match summary.ranking_confidence {
        RankingConfidence::High | RankingConfidence::Medium => {
            TuneRecommendationVerdict::Recommended
        }
        RankingConfidence::Low => TuneRecommendationVerdict::NeedsRetest,
        RankingConfidence::Unstable => TuneRecommendationVerdict::NoRecommendation,
    };

    let summary_text = match verdict {
        TuneRecommendationVerdict::Recommended => format!(
            "Profile '{}' is recommended for this workload sample.",
            summary.best_profile
        ),
        TuneRecommendationVerdict::NeedsRetest => format!(
            "Profile '{}' is currently best, but the result is not strong enough to trust without another run.",
            summary.best_profile
        ),
        TuneRecommendationVerdict::NoRecommendation => {
            "No profile recommendation is available.".to_owned()
        }
    };

    match verdict {
        TuneRecommendationVerdict::Recommended => {
            next_steps.push(
                "Apply manually only if this route and workload are representative.".to_owned(),
            );
            next_steps.push("Keep the restore path handy: stutter restore".to_owned());
        }
        TuneRecommendationVerdict::NeedsRetest => {
            next_steps.push("Rerun tune with the same workload and at least 5 runs.".to_owned());
        }
        TuneRecommendationVerdict::NoRecommendation => {
            next_steps.push("Collect another tuning run before applying a profile.".to_owned());
        }
    }

    TuneRecommendation {
        schema_version: 1,
        verdict,
        best_profile: Some(summary.best_profile.clone()),
        baseline_profile: baseline_profile.map(ToOwned::to_owned),
        compared_against: comparison_kind,
        confidence: summary.ranking_confidence,
        summary: summary_text,
        why,
        warnings,
        next_steps,
        best_metrics: Some(metrics_from_stat(best_stat)),
        comparison_metrics,
    }
}

pub fn render_tune_recommendation_markdown(rec: &TuneRecommendation) -> String {
    let mut out = String::new();
    pushln(&mut out, "# stutter tuning recommendation");
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
    pushln(&mut out, "");
    pushln(&mut out, "## Summary");
    pushln(&mut out, "");
    pushln(&mut out, &rec.summary);
    pushln(&mut out, "");
    pushln(&mut out, "## Why");
    pushln(&mut out, "");
    push_markdown_list(&mut out, &rec.why);
    pushln(&mut out, "");
    pushln(&mut out, "## Warnings");
    pushln(&mut out, "");
    if rec.warnings.is_empty() {
        pushln(&mut out, "- none");
    } else {
        push_markdown_list(&mut out, &rec.warnings);
    }
    pushln(&mut out, "");
    pushln(&mut out, "## Next steps");
    pushln(&mut out, "");
    push_markdown_list(&mut out, &rec.next_steps);
    out
}

fn valid_stat<'a>(summary: &'a TuneSummary, profile: &str) -> Option<&'a TuneProfileStats> {
    summary
        .profile_stats
        .iter()
        .find(|stat| stat.profile == profile && stat.valid_runs > 0)
}

fn choose_comparison_stat<'a>(
    summary: &'a TuneSummary,
    baseline_profile: Option<&str>,
    best_profile: &str,
    warnings: &mut Vec<String>,
) -> (Option<String>, Option<&'a TuneProfileStats>) {
    if let Some(baseline) = baseline_profile {
        if let Some(stat) = valid_stat(summary, baseline) {
            return (Some("baseline".to_owned()), Some(stat));
        }
        warnings.push(format!(
            "baseline profile '{baseline}' was not present with valid stats"
        ));
    } else {
        warnings.push("no baseline profile specified".to_owned());
    }

    let second = summary
        .profile_stats
        .iter()
        .filter(|stat| stat.profile != best_profile && stat.valid_runs > 0)
        .min_by_key(|stat| stat.median_diagnostic_raw_score_total);
    if second.is_some() {
        (Some("second-best".to_owned()), second)
    } else {
        (None, None)
    }
}

fn metrics_from_stat(stat: &TuneProfileStats) -> TuneRecommendationMetrics {
    TuneRecommendationMetrics {
        median_diagnostic_raw_score_total: stat.median_diagnostic_raw_score_total,
        iqr_diagnostic_raw_score_total: stat.iqr_diagnostic_raw_score_total,
        worst_diagnostic_raw_score_total: stat.worst_diagnostic_raw_score_total,
        median_over_5ms: stat.median_over_5ms,
        median_frame_p99_us: stat.median_frame_p99_us,
        valid_runs: stat.valid_runs,
        invalid_runs: stat.invalid_runs,
    }
}

fn comparison_between(
    best: &TuneProfileStats,
    other: &TuneProfileStats,
) -> TuneRecommendationComparison {
    let score_delta_abs = best.median_diagnostic_raw_score_total as i64
        - other.median_diagnostic_raw_score_total as i64;
    let score_pooled_stddev = pooled_comparison_stddev(
        best.stddev_diagnostic_raw_score_total,
        best.valid_runs,
        other.stddev_diagnostic_raw_score_total,
        other.valid_runs,
    );

    let over_5ms_delta_abs = best.median_over_5ms as i64 - other.median_over_5ms as i64;
    let over_5ms_pooled_stddev = pooled_comparison_stddev(
        best.stddev_over_5ms,
        best.valid_runs,
        other.stddev_over_5ms,
        other.valid_runs,
    );

    let frame_p99_delta_us = best.median_frame_p99_us as i64 - other.median_frame_p99_us as i64;
    let frame_p99_pooled_stddev = pooled_comparison_stddev(
        best.stddev_frame_p99_us,
        best.valid_runs,
        other.stddev_frame_p99_us,
        other.valid_runs,
    );

    TuneRecommendationComparison {
        other_profile: other.profile.clone(),

        score_delta_abs,
        score_delta_percent: if other.median_diagnostic_raw_score_total > 0 {
            Some(score_delta_abs as f64 / other.median_diagnostic_raw_score_total as f64 * 100.0)
        } else {
            None
        },
        score_effect_size: normalized_effect_size(score_delta_abs, score_pooled_stddev),
        score_noise_ratio: noise_ratio(
            best.iqr_diagnostic_raw_score_total,
            best.median_diagnostic_raw_score_total,
        ),

        over_5ms_delta_abs,
        over_5ms_effect_size: normalized_effect_size(over_5ms_delta_abs, over_5ms_pooled_stddev),
        over_5ms_noise_ratio: noise_ratio(best.iqr_over_5ms, best.median_over_5ms),

        frame_p99_delta_us,
        frame_p99_effect_size: normalized_effect_size(frame_p99_delta_us, frame_p99_pooled_stddev),
        frame_p99_noise_ratio: noise_ratio(best.iqr_frame_p99_us, best.median_frame_p99_us),
        formal_metrics: Vec::new(),
    }
}

fn pooled_comparison_stddev(
    best_stddev: f64,
    best_n: usize,
    other_stddev: f64,
    other_n: usize,
) -> f64 {
    if best_n < 2 || other_n < 2 {
        return 0.0;
    }

    let numerator = ((best_n - 1) as f64 * best_stddev * best_stddev)
        + ((other_n - 1) as f64 * other_stddev * other_stddev);
    let denominator = (best_n + other_n - 2) as f64;

    if denominator <= 0.0 {
        0.0
    } else {
        (numerator / denominator).sqrt()
    }
}

fn format_percent(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.1}%"))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn format_optional_float(value: Option<f64>, precision: usize) -> String {
    match value {
        Some(value) => format!("{value:.precision$}"),
        None => "n/a".to_owned(),
    }
}

fn push_markdown_list(out: &mut String, items: &[String]) {
    if items.is_empty() {
        pushln(out, "- none");
        return;
    }
    for item in items {
        pushln(out, format!("- {item}"));
    }
}

fn pushln(out: &mut String, line: impl AsRef<str>) {
    out.push_str(line.as_ref());
    out.push('\n');
}

#[cfg(test)]
mod tests;
