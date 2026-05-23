use serde::{Deserialize, Serialize};

use super::{RankingConfidence, TuneProfileStats, TuneSummary};

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
    pub over_5ms_delta_abs: i64,
    pub frame_p99_delta_us: i64,
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
        let metrics = comparison_between(best_stat, other);
        why.push(format!(
            "Median score delta versus {} '{}' is {} ({})",
            comparison_kind.as_deref().unwrap_or("comparison"),
            other.profile,
            metrics.score_delta_abs,
            format_percent(metrics.score_delta_percent)
        ));
        why.push(format!(
            "over_5ms delta versus '{}' is {}",
            other.profile, metrics.over_5ms_delta_abs
        ));
        if best_stat.median_frame_p99_us > 0 || other.median_frame_p99_us > 0 {
            why.push(format!(
                "frame p99 delta versus '{}' is {}us",
                other.profile, metrics.frame_p99_delta_us
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
    TuneRecommendationComparison {
        other_profile: other.profile.clone(),
        score_delta_abs,
        score_delta_percent: if other.median_diagnostic_raw_score_total > 0 {
            Some(score_delta_abs as f64 / other.median_diagnostic_raw_score_total as f64 * 100.0)
        } else {
            None
        },
        over_5ms_delta_abs: best.median_over_5ms as i64 - other.median_over_5ms as i64,
        frame_p99_delta_us: best.median_frame_p99_us as i64 - other.median_frame_p99_us as i64,
    }
}

fn format_percent(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.1}%"))
        .unwrap_or_else(|| "n/a".to_owned())
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
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::tune::{TuneCandidateSummary, TuneCoverageMetrics, TuneIterationOrder};

    fn stat(profile: &str, median: u64) -> TuneProfileStats {
        TuneProfileStats {
            profile: profile.to_owned(),
            valid_runs: 3,
            invalid_runs: 0,
            median_diagnostic_raw_score_total: median,
            iqr_diagnostic_raw_score_total: 0,
            worst_diagnostic_raw_score_total: median,
            median_over_5ms: median / 100,
            iqr_over_5ms: 0,
            median_frame_p99_us: 16_000,
            iqr_frame_p99_us: 0,
        }
    }

    fn candidate(profile: &str, diagnostic_score_total: u64) -> TuneCandidateSummary {
        TuneCandidateSummary {
            profile: profile.to_owned(),
            iteration: 1,
            run_dir: PathBuf::from(format!("/tmp/{profile}")),
            applied_tasks: 1,
            warmup_seconds: 1,
            measure_seconds: 1,
            interval_count: 2,
            samples: 100,
            scored_samples: 100,
            diagnostic_score_total,
            over_1ms: 0,
            over_2ms: 0,
            over_5ms: diagnostic_score_total / 100,
            max_latency_ns: 0,
            frame_count: 1,
            frame_max_ms: 16.0,
            frame_p99_ms: 16.0,
            frame_over_16ms: 0,
            frame_over_33ms: 0,
            frame_over_50ms: 0,
            coverage: TuneCoverageMetrics::default(),
            valid: true,
        }
    }

    fn summary(confidence: RankingConfidence) -> TuneSummary {
        TuneSummary {
            schema_version: 1,
            tree_pid: 42,
            profiles_path: PathBuf::from("profiles.toml"),
            runs: 3,
            epoch_seconds: 120,
            warmup_seconds: 30,
            restore_policy: "restore-after-each".to_owned(),
            best_profile: "best".to_owned(),
            candidate_order: vec![TuneIterationOrder {
                iteration: 1,
                profiles: vec![
                    "best".to_owned(),
                    "baseline".to_owned(),
                    "second".to_owned(),
                ],
            }],
            profile_stats: vec![stat("best", 80), stat("baseline", 120), stat("second", 100)],
            ranking_confidence: confidence,
            ranking_notes: Vec::new(),
            comparability_warnings: Vec::new(),
            candidates: vec![candidate("best", 80), candidate("baseline", 120)],
        }
    }

    #[test]
    fn unstable_ranking_produces_no_recommendation() {
        let mut summary = summary(RankingConfidence::Unstable);
        summary.ranking_notes.push("too close".to_owned());

        let rec = build_tune_recommendation(&summary, None);

        assert_eq!(rec.verdict, TuneRecommendationVerdict::NoRecommendation);
        assert_eq!(rec.best_profile, None);
        assert!(rec.warnings.iter().any(|warning| warning == "too close"));
    }

    #[test]
    fn medium_confidence_produces_recommended_verdict() {
        let rec = build_tune_recommendation(&summary(RankingConfidence::Medium), None);

        assert_eq!(rec.verdict, TuneRecommendationVerdict::Recommended);
        assert_eq!(rec.best_profile.as_deref(), Some("best"));
    }

    #[test]
    fn low_confidence_produces_needs_retest() {
        let rec = build_tune_recommendation(&summary(RankingConfidence::Low), None);

        assert_eq!(rec.verdict, TuneRecommendationVerdict::NeedsRetest);
        assert_eq!(rec.best_profile.as_deref(), Some("best"));
    }

    #[test]
    fn baseline_profile_is_preferred_for_comparison() {
        let rec = build_tune_recommendation(&summary(RankingConfidence::High), Some("baseline"));

        assert_eq!(rec.compared_against.as_deref(), Some("baseline"));
        assert_eq!(
            rec.comparison_metrics.as_ref().unwrap().other_profile,
            "baseline"
        );
    }

    #[test]
    fn missing_baseline_profile_adds_warning() {
        let rec = build_tune_recommendation(&summary(RankingConfidence::High), Some("missing"));

        assert!(
            rec.warnings
                .iter()
                .any(|warning| warning.contains("baseline profile 'missing'"))
        );
        assert_eq!(rec.compared_against.as_deref(), Some("second-best"));
    }

    #[test]
    fn markdown_contains_verdict_best_profile_confidence_and_warnings() {
        let mut summary = summary(RankingConfidence::Medium);
        summary.ranking_notes.push("note".to_owned());
        summary
            .comparability_warnings
            .push(crate::tune::comparability::TuneComparabilityWarning {
                profile: Some("best".to_owned()),
                kind: "scored-sample-count-mismatch".to_owned(),
                message: "sample mismatch".to_owned(),
                severity: crate::tune::comparability::TuneComparabilitySeverity::Warning,
            });
        let rec = build_tune_recommendation(&summary, None);
        let markdown = render_tune_recommendation_markdown(&rec);

        assert!(markdown.contains("Verdict: Recommended"));
        assert!(markdown.contains("Best profile: best"));
        assert!(markdown.contains("Confidence: Medium"));
        assert!(markdown.contains("## Warnings"));
        assert!(markdown.contains("- note"));
        assert!(markdown.contains("sample mismatch"));
    }
}
