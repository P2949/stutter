use std::{collections::BTreeMap, path::PathBuf};

use super::*;
use crate::tune::{
    TuneCandidateSummary, TuneCoverageMetrics, TuneIterationOrder, TuneProfilePlanSummary,
    TuneProfileRulePlanSummary,
};

fn stat(profile: &str, median: u64) -> TuneProfileStats {
    TuneProfileStats {
        profile: profile.to_owned(),
        valid_runs: 3,
        invalid_runs: 0,
        median_diagnostic_raw_score_total: median,
        iqr_diagnostic_raw_score_total: 0,
        worst_diagnostic_raw_score_total: median,
        mean_diagnostic_raw_score_total: median as f64,
        stddev_diagnostic_raw_score_total: 0.0,
        median_over_5ms: median / 100,
        iqr_over_5ms: 0,
        mean_over_5ms: (median / 100) as f64,
        stddev_over_5ms: 0.0,
        median_frame_p99_us: 16_000,
        iqr_frame_p99_us: 0,
        mean_frame_p99_us: 16_000.0,
        stddev_frame_p99_us: 0.0,
    }
}

#[derive(Clone, Copy)]
struct CandidateMetricFixture {
    iteration: u32,
    diagnostic_raw_score_total: u64,
    over_5ms: u64,
    frame_p99_ms: f64,
    frame_over_16ms: u64,
    frame_over_33ms: u64,
    frame_over_50ms: u64,
    max_latency_ns: u64,
}

fn candidate(profile: &str, diagnostic_raw_score_total: u64) -> TuneCandidateSummary {
    candidate_with_metrics(
        profile,
        CandidateMetricFixture {
            iteration: 1,
            diagnostic_raw_score_total,
            over_5ms: diagnostic_raw_score_total / 100,
            frame_p99_ms: 16.0,
            frame_over_16ms: 0,
            frame_over_33ms: 0,
            frame_over_50ms: 0,
            max_latency_ns: 0,
        },
    )
}

fn candidate_with_metrics(profile: &str, metrics: CandidateMetricFixture) -> TuneCandidateSummary {
    TuneCandidateSummary {
        profile: profile.to_owned(),
        iteration: metrics.iteration,
        run_dir: PathBuf::from(format!("/tmp/{profile}-{}", metrics.iteration)),
        applied_tasks: 1,
        profile_plan: None,
        warmup_seconds: 1,
        measure_seconds: 1,
        interval_count: 2,
        samples: 100,
        scored_samples: 100,
        diagnostic_raw_score_total: metrics.diagnostic_raw_score_total,
        over_1ms: 0,
        over_2ms: 0,
        over_5ms: metrics.over_5ms,
        max_latency_ns: metrics.max_latency_ns,
        frame_count: 120,
        frame_max_ms: metrics.frame_p99_ms + 3.0,
        frame_p99_ms: metrics.frame_p99_ms,
        frame_over_16ms: metrics.frame_over_16ms,
        frame_over_33ms: metrics.frame_over_33ms,
        frame_over_50ms: metrics.frame_over_50ms,
        coverage: TuneCoverageMetrics::default(),
        valid: true,
    }
}

fn formal_metric_names(metrics: &[FormalMetricComparison]) -> Vec<&str> {
    metrics
        .iter()
        .map(|metric| metric.metric.as_str())
        .collect()
}

const EXPECTED_FORMAL_METRICS: &[&str] = &[
    "diagnostic_raw_score_total",
    "over_5ms",
    "frame_p99_ms",
    "frame_over_16ms",
    "frame_over_33ms",
    "frame_over_50ms",
    "max_latency_ns",
];

fn summary(confidence: RankingConfidence) -> TuneSummary {
    TuneSummary {
        schema_version: 1,
        tree_pid: 42,
        scenario_name: None,
        scenario_hash: None,
        workload_label: None,
        route_label: None,
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
        candidates: vec![
            candidate_with_metrics(
                "best",
                CandidateMetricFixture {
                    iteration: 1,
                    diagnostic_raw_score_total: 80,
                    over_5ms: 1,
                    frame_p99_ms: 16.0,
                    frame_over_16ms: 0,
                    frame_over_33ms: 0,
                    frame_over_50ms: 0,
                    max_latency_ns: 1_000_000,
                },
            ),
            candidate_with_metrics(
                "best",
                CandidateMetricFixture {
                    iteration: 2,
                    diagnostic_raw_score_total: 80,
                    over_5ms: 1,
                    frame_p99_ms: 16.0,
                    frame_over_16ms: 0,
                    frame_over_33ms: 0,
                    frame_over_50ms: 0,
                    max_latency_ns: 1_000_000,
                },
            ),
            candidate_with_metrics(
                "best",
                CandidateMetricFixture {
                    iteration: 3,
                    diagnostic_raw_score_total: 80,
                    over_5ms: 1,
                    frame_p99_ms: 16.0,
                    frame_over_16ms: 0,
                    frame_over_33ms: 0,
                    frame_over_50ms: 0,
                    max_latency_ns: 1_000_000,
                },
            ),
            candidate_with_metrics(
                "baseline",
                CandidateMetricFixture {
                    iteration: 1,
                    diagnostic_raw_score_total: 120,
                    over_5ms: 2,
                    frame_p99_ms: 17.0,
                    frame_over_16ms: 1,
                    frame_over_33ms: 0,
                    frame_over_50ms: 0,
                    max_latency_ns: 2_000_000,
                },
            ),
            candidate_with_metrics(
                "baseline",
                CandidateMetricFixture {
                    iteration: 2,
                    diagnostic_raw_score_total: 120,
                    over_5ms: 2,
                    frame_p99_ms: 17.0,
                    frame_over_16ms: 1,
                    frame_over_33ms: 0,
                    frame_over_50ms: 0,
                    max_latency_ns: 2_000_000,
                },
            ),
            candidate_with_metrics(
                "baseline",
                CandidateMetricFixture {
                    iteration: 3,
                    diagnostic_raw_score_total: 120,
                    over_5ms: 2,
                    frame_p99_ms: 17.0,
                    frame_over_16ms: 1,
                    frame_over_33ms: 0,
                    frame_over_50ms: 0,
                    max_latency_ns: 2_000_000,
                },
            ),
            candidate_with_metrics(
                "second",
                CandidateMetricFixture {
                    iteration: 1,
                    diagnostic_raw_score_total: 100,
                    over_5ms: 2,
                    frame_p99_ms: 16.5,
                    frame_over_16ms: 1,
                    frame_over_33ms: 0,
                    frame_over_50ms: 0,
                    max_latency_ns: 1_500_000,
                },
            ),
            candidate_with_metrics(
                "second",
                CandidateMetricFixture {
                    iteration: 2,
                    diagnostic_raw_score_total: 100,
                    over_5ms: 2,
                    frame_p99_ms: 16.5,
                    frame_over_16ms: 1,
                    frame_over_33ms: 0,
                    frame_over_50ms: 0,
                    max_latency_ns: 1_500_000,
                },
            ),
            candidate_with_metrics(
                "second",
                CandidateMetricFixture {
                    iteration: 3,
                    diagnostic_raw_score_total: 100,
                    over_5ms: 2,
                    frame_p99_ms: 16.5,
                    frame_over_16ms: 1,
                    frame_over_33ms: 0,
                    frame_over_50ms: 0,
                    max_latency_ns: 1_500_000,
                },
            ),
        ],
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
fn formal_diagnostic_score_blocks_recommended_verdict_when_underpowered() {
    let mut summary = summary(RankingConfidence::High);
    summary.candidates = vec![candidate("best", 80), candidate("baseline", 120)];

    let rec = build_tune_recommendation(&summary, None);

    assert_eq!(rec.verdict, TuneRecommendationVerdict::NeedsRetest);
    assert!(
        rec.warnings
            .iter()
            .any(|warning| warning.contains("formal diagnostic score comparison"))
    );
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

#[test]
fn recommendation_includes_profile_plan_coverage_section() {
    let mut summary = summary(RankingConfidence::Medium);
    summary.candidates[0].profile_plan = Some(TuneProfilePlanSummary {
        snapshot_tasks: 184,
        matched_tasks: 117,
        pending_unique_tasks: 117,
        pending_affinity: 117,
        rules: vec![TuneProfileRulePlanSummary {
            rule_index: 0,
            matched_tasks: 104,
            pending_affinity: 104,
            top_classes: BTreeMap::from([("Helper".to_owned(), 92)]),
            top_thread_comms: BTreeMap::from([("RenderThread".to_owned(), 1)]),
            process_comm_captures: 103,
        }],
    });

    let rec = build_tune_recommendation(&summary, None);
    let markdown = render_tune_recommendation_markdown(&rec);
    let html = crate::tune::recommendation_html::render_tune_recommendation_html(&rec);

    assert_eq!(rec.profile_plan.as_ref().unwrap().matched_tasks, 117);
    assert!(
        rec.why
            .iter()
            .any(|why| why.contains("Profile coverage matched 117 of 184"))
    );
    assert!(markdown.contains("## Profile coverage"));
    assert!(markdown.contains("Top rule 0 matched 104 task(s)"));
    assert!(html.contains("<h2>Profile coverage</h2>"));
    assert!(html.contains("RenderThread"));
}

#[test]
fn formal_ab_metrics_cover_all_key_tune_candidate_metrics() {
    let mut summary = summary(RankingConfidence::High);
    summary.candidates = vec![
        candidate_with_metrics(
            "best",
            CandidateMetricFixture {
                iteration: 1,
                diagnostic_raw_score_total: 180,
                over_5ms: 3,
                frame_p99_ms: 17.0,
                frame_over_16ms: 2,
                frame_over_33ms: 1,
                frame_over_50ms: 0,
                max_latency_ns: 2_000_000,
            },
        ),
        candidate_with_metrics(
            "best",
            CandidateMetricFixture {
                iteration: 2,
                diagnostic_raw_score_total: 190,
                over_5ms: 4,
                frame_p99_ms: 18.0,
                frame_over_16ms: 3,
                frame_over_33ms: 2,
                frame_over_50ms: 1,
                max_latency_ns: 3_000_000,
            },
        ),
        candidate_with_metrics(
            "best",
            CandidateMetricFixture {
                iteration: 3,
                diagnostic_raw_score_total: 200,
                over_5ms: 5,
                frame_p99_ms: 19.0,
                frame_over_16ms: 4,
                frame_over_33ms: 3,
                frame_over_50ms: 2,
                max_latency_ns: 4_000_000,
            },
        ),
        candidate_with_metrics(
            "baseline",
            CandidateMetricFixture {
                iteration: 1,
                diagnostic_raw_score_total: 280,
                over_5ms: 8,
                frame_p99_ms: 25.0,
                frame_over_16ms: 7,
                frame_over_33ms: 4,
                frame_over_50ms: 3,
                max_latency_ns: 8_000_000,
            },
        ),
        candidate_with_metrics(
            "baseline",
            CandidateMetricFixture {
                iteration: 2,
                diagnostic_raw_score_total: 300,
                over_5ms: 9,
                frame_p99_ms: 26.0,
                frame_over_16ms: 8,
                frame_over_33ms: 5,
                frame_over_50ms: 4,
                max_latency_ns: 9_000_000,
            },
        ),
        candidate_with_metrics(
            "baseline",
            CandidateMetricFixture {
                iteration: 3,
                diagnostic_raw_score_total: 320,
                over_5ms: 10,
                frame_p99_ms: 27.0,
                frame_over_16ms: 9,
                frame_over_33ms: 6,
                frame_over_50ms: 5,
                max_latency_ns: 10_000_000,
            },
        ),
    ];

    let rec = build_tune_recommendation(&summary, Some("baseline"));
    let metrics = &rec.comparison_metrics.as_ref().unwrap().formal_metrics;

    assert_eq!(formal_metric_names(metrics), EXPECTED_FORMAL_METRICS);
    for metric in metrics {
        assert!(
            metric.enough_samples,
            "{} should have enough samples",
            metric.metric
        );
        assert!(
            metric.bootstrap_ci95.is_some(),
            "{} should report a bootstrap CI",
            metric.metric
        );
        assert!(
            metric.effect_size.is_some(),
            "{} should report an effect size when both sides vary",
            metric.metric
        );
    }
    assert!(
        rec.why
            .iter()
            .any(|line| line.contains("formal A/B max_latency_ns"))
    );
}

#[test]
fn recommendation_comparison_includes_effect_size_and_noise_ratio() {
    let best = TuneProfileStats {
        profile: "best".to_owned(),
        valid_runs: 3,
        invalid_runs: 0,
        median_diagnostic_raw_score_total: 100,
        iqr_diagnostic_raw_score_total: 10,
        worst_diagnostic_raw_score_total: 110,
        mean_diagnostic_raw_score_total: 100.0,
        stddev_diagnostic_raw_score_total: 10.0,
        median_over_5ms: 2,
        iqr_over_5ms: 1,
        mean_over_5ms: 2.0,
        stddev_over_5ms: 1.0,
        median_frame_p99_us: 8000,
        iqr_frame_p99_us: 500,
        mean_frame_p99_us: 8000.0,
        stddev_frame_p99_us: 500.0,
    };

    let other = TuneProfileStats {
        profile: "other".to_owned(),
        median_diagnostic_raw_score_total: 130,
        mean_diagnostic_raw_score_total: 130.0,
        ..best.clone()
    };

    let comparison = comparison_between(&best, &other);

    assert!(comparison.score_effect_size.is_some());
    assert_eq!(comparison.score_noise_ratio, Some(0.10));
}

#[test]
fn tune_recommendation_html_exposes_ab_uncertainty_charts_and_metrics() {
    let mut summary = summary(RankingConfidence::High);
    summary.candidates = vec![
        candidate_with_metrics(
            "best",
            CandidateMetricFixture {
                iteration: 1,
                diagnostic_raw_score_total: 180,
                over_5ms: 3,
                frame_p99_ms: 17.0,
                frame_over_16ms: 2,
                frame_over_33ms: 1,
                frame_over_50ms: 0,
                max_latency_ns: 2_000_000,
            },
        ),
        candidate_with_metrics(
            "best",
            CandidateMetricFixture {
                iteration: 2,
                diagnostic_raw_score_total: 190,
                over_5ms: 4,
                frame_p99_ms: 18.0,
                frame_over_16ms: 3,
                frame_over_33ms: 2,
                frame_over_50ms: 1,
                max_latency_ns: 3_000_000,
            },
        ),
        candidate_with_metrics(
            "best",
            CandidateMetricFixture {
                iteration: 3,
                diagnostic_raw_score_total: 200,
                over_5ms: 5,
                frame_p99_ms: 19.0,
                frame_over_16ms: 4,
                frame_over_33ms: 3,
                frame_over_50ms: 2,
                max_latency_ns: 4_000_000,
            },
        ),
        candidate_with_metrics(
            "baseline",
            CandidateMetricFixture {
                iteration: 1,
                diagnostic_raw_score_total: 280,
                over_5ms: 8,
                frame_p99_ms: 25.0,
                frame_over_16ms: 7,
                frame_over_33ms: 4,
                frame_over_50ms: 3,
                max_latency_ns: 8_000_000,
            },
        ),
        candidate_with_metrics(
            "baseline",
            CandidateMetricFixture {
                iteration: 2,
                diagnostic_raw_score_total: 300,
                over_5ms: 9,
                frame_p99_ms: 26.0,
                frame_over_16ms: 8,
                frame_over_33ms: 5,
                frame_over_50ms: 4,
                max_latency_ns: 9_000_000,
            },
        ),
        candidate_with_metrics(
            "baseline",
            CandidateMetricFixture {
                iteration: 3,
                diagnostic_raw_score_total: 320,
                over_5ms: 10,
                frame_p99_ms: 27.0,
                frame_over_16ms: 9,
                frame_over_33ms: 6,
                frame_over_50ms: 5,
                max_latency_ns: 10_000_000,
            },
        ),
    ];

    let rec = build_tune_recommendation(&summary, Some("baseline"));
    let html = crate::tune::recommendation_html::render_tune_recommendation_html(&rec);

    assert!(html.contains("A/B uncertainty"));
    assert!(html.contains("Sample distribution"));
    assert!(html.contains("Bootstrap median-improvement CI"));
    assert!(html.contains("Effect size"));
    assert!(html.contains("Noise ratio"));
    assert!(html.contains("Samples"));
    assert!(html.contains("diagnostic_raw_score_total"));
}
