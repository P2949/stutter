use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;
use crate::{
    metrics::IntervalRecord,
    process_tree::TaskClass,
    recorder::{RecordedConfig, RecordedTime, SessionFile},
    tune::{
        RankingConfidence, TuneCandidateSummary, TuneCoverageMetrics, TuneIterationOrder,
        TuneProfileStats, TuneSummary, recommendation::TuneRecommendationVerdict,
    },
};

fn temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-recommend-test-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn minimal_session(interval_count: u64) -> SessionFile {
    SessionFile {
        core: crate::recorder::SessionMetadataCore {
            schema_version: crate::recorder::SESSION_SCHEMA_VERSION,
            run_name: Some("baseline".to_owned()),
            started_at: RecordedTime::default(),
            ended_at: RecordedTime::default(),
            duration_ms: 1000,
            interval_record_count: interval_count,
            active_target_pids_count: 1,
            ..Default::default()
        },
        stop_reason: "test".to_owned(),
        config: RecordedConfig {
            summary_period_ms: 1000,
            spike_threshold_ns: 1_000_000,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn interval(over_5ms: u64, samples: u64) -> IntervalRecord {
    interval_with_max_latency(over_5ms, samples, over_5ms.saturating_mul(1_000_000))
}

fn interval_with_max_latency(over_5ms: u64, samples: u64, max_ns: u64) -> IntervalRecord {
    IntervalRecord {
        elapsed_ms: 1000,
        task: 1,
        active: true,
        class: TaskClass::Game,
        comm: "game".to_owned(),
        process_pid: Some(1),
        process_comm: "game".into(),
        samples,
        stored_samples: samples,
        over_5ms,
        max_ns,
        percentile_scope: "all".to_owned(),
        ..Default::default()
    }
}

fn write_baseline(dir: &Path, score_over_5ms: u64, samples: u64) {
    fs::write(
        dir.join("session.json"),
        serde_json::to_vec_pretty(&minimal_session(1)).unwrap(),
    )
    .unwrap();
    fs::write(
        dir.join("interval.json"),
        format!(
            "{}\n",
            serde_json::to_string(&interval(score_over_5ms, samples)).unwrap()
        ),
    )
    .unwrap();
}

fn write_baseline_with_frames(
    dir: &Path,
    score_over_5ms: u64,
    samples: u64,
    max_ns: u64,
    frametimes_ms: &[f64],
) {
    fs::write(
        dir.join("session.json"),
        serde_json::to_vec_pretty(&minimal_session(1)).unwrap(),
    )
    .unwrap();
    fs::write(
        dir.join("interval.json"),
        format!(
            "{}\n",
            serde_json::to_string(&interval_with_max_latency(score_over_5ms, samples, max_ns))
                .unwrap()
        ),
    )
    .unwrap();
    let frames = frametimes_ms
        .iter()
        .enumerate()
        .map(|(idx, frametime_ms)| {
            format!(
                r#"{{"elapsed_ms":{},"frametime_ms":{}}}"#,
                (idx + 1) * 100,
                frametime_ms
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(dir.join("frame_correlation.json"), format!("{frames}\n")).unwrap();
}

fn stat(profile: &str, score: u64, confidence_runs: usize) -> TuneProfileStats {
    TuneProfileStats {
        profile: profile.to_owned(),
        valid_runs: confidence_runs,
        invalid_runs: 0,
        median_diagnostic_raw_score_total: score,
        iqr_diagnostic_raw_score_total: 0,
        worst_diagnostic_raw_score_total: score,
        mean_diagnostic_raw_score_total: score as f64,
        stddev_diagnostic_raw_score_total: 0.0,
        median_over_5ms: score / 100,
        iqr_over_5ms: 0,
        mean_over_5ms: (score / 100) as f64,
        stddev_over_5ms: 0.0,
        median_frame_p99_us: 0,
        iqr_frame_p99_us: 0,
        mean_frame_p99_us: 0.0,
        stddev_frame_p99_us: 0.0,
    }
}

#[derive(Clone, Copy)]
struct CandidateMetricFixture {
    iteration: u32,
    diagnostic_raw_score_total: u64,
    over_5ms: u64,
    max_latency_ns: u64,
    frame_p99_ms: f64,
    frame_over_16ms: u64,
    frame_over_33ms: u64,
    frame_over_50ms: u64,
}

fn candidate(profile: &str, score: u64) -> TuneCandidateSummary {
    candidate_with_iteration(profile, 1, score)
}

fn candidate_with_iteration(profile: &str, iteration: u32, score: u64) -> TuneCandidateSummary {
    candidate_with_metrics(
        profile,
        CandidateMetricFixture {
            iteration,
            diagnostic_raw_score_total: score,
            over_5ms: score / 100,
            max_latency_ns: 0,
            frame_p99_ms: 0.0,
            frame_over_16ms: 0,
            frame_over_33ms: 0,
            frame_over_50ms: 0,
        },
    )
}

fn candidate_with_metrics(profile: &str, metrics: CandidateMetricFixture) -> TuneCandidateSummary {
    TuneCandidateSummary {
        profile: profile.to_owned(),
        iteration: metrics.iteration,
        run_dir: PathBuf::from(format!("/tmp/{profile}-{}", metrics.iteration)),
        applied_tasks: 1,
        warmup_seconds: 0,
        measure_seconds: 1,
        interval_count: 1,
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

fn formal_metric_names(metrics: &[crate::tune::statistics::FormalMetricComparison]) -> Vec<&str> {
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

fn write_repeated_baselines(name: &str, scores_over_5ms: &[u64]) -> Vec<PathBuf> {
    scores_over_5ms
        .iter()
        .enumerate()
        .map(|(idx, score)| {
            let dir = temp_dir(&format!("{name}-{idx}"));
            write_baseline(&dir, *score, 100);
            dir
        })
        .collect()
}

fn write_tune(dir: &Path, confidence: RankingConfidence, best_score: u64) {
    write_tune_with_candidates(
        dir,
        confidence,
        best_score,
        vec![
            candidate_with_iteration("best", 1, best_score),
            candidate_with_iteration("best", 2, best_score),
            candidate_with_iteration("best", 3, best_score),
        ],
    );
}

fn write_tune_with_candidates(
    dir: &Path,
    confidence: RankingConfidence,
    best_score: u64,
    candidates: Vec<TuneCandidateSummary>,
) {
    fs::create_dir_all(dir).unwrap();
    let summary = TuneSummary {
        schema_version: 1,
        tree_pid: 42,
        profiles_path: PathBuf::from("profiles.toml"),
        runs: 3,
        epoch_seconds: 60,
        warmup_seconds: 10,
        restore_policy: "restore-after-each".to_owned(),
        best_profile: if confidence == RankingConfidence::Unstable {
            String::new()
        } else {
            "best".to_owned()
        },
        candidate_order: vec![TuneIterationOrder {
            iteration: 1,
            profiles: vec!["best".to_owned(), "other".to_owned()],
        }],
        profile_stats: vec![
            stat("best", best_score, 3),
            stat("other", best_score + 100, 3),
        ],
        ranking_confidence: confidence,
        ranking_notes: Vec::new(),
        comparability_warnings: Vec::new(),
        candidates,
    };
    fs::write(
        dir.join("tuning_summary.json"),
        serde_json::to_vec_pretty(&summary).unwrap(),
    )
    .unwrap();
}

#[test]
fn missing_tuning_summary_fails_clearly() {
    let baseline = temp_dir("missing-summary-baseline");
    write_baseline(&baseline, 2, 100);
    let tune = temp_dir("missing-summary-tune");

    let err = build_baseline_tune_recommendation(&baseline, &tune).unwrap_err();

    assert!(err.to_string().contains("missing tuning summary"));
    fs::remove_dir_all(baseline).ok();
    fs::remove_dir_all(tune).ok();
}

#[test]
fn unstable_tune_summary_gives_no_recommendation() {
    let baseline = temp_dir("unstable-baseline");
    write_baseline(&baseline, 2, 100);
    let tune = temp_dir("unstable-tune");
    write_tune(&tune, RankingConfidence::Unstable, 100);

    let rec = build_baseline_tune_recommendation(&baseline, &tune).unwrap();

    assert_eq!(rec.verdict, TuneRecommendationVerdict::NoRecommendation);
    fs::remove_dir_all(baseline).ok();
    fs::remove_dir_all(tune).ok();
}

#[test]
fn best_worse_than_baseline_gives_no_recommendation() {
    let baseline = temp_dir("worse-baseline");
    write_baseline(&baseline, 1, 100);
    let tune = temp_dir("worse-tune");
    write_tune(&tune, RankingConfidence::High, 200);

    let rec = build_baseline_tune_recommendation(&baseline, &tune).unwrap();

    assert_eq!(rec.verdict, TuneRecommendationVerdict::NoRecommendation);
    fs::remove_dir_all(baseline).ok();
    fs::remove_dir_all(tune).ok();
}

#[test]
fn best_more_than_five_percent_better_gives_recommended_with_repeated_baselines() {
    let baselines = write_repeated_baselines("better-baseline", &[2, 2, 2]);
    let tune = temp_dir("better-tune");
    write_tune(&tune, RankingConfidence::High, 100);

    let rec = build_baseline_tune_recommendation_for_baselines(&baselines, &tune).unwrap();

    assert_eq!(rec.verdict, TuneRecommendationVerdict::Recommended);
    assert_eq!(rec.score_delta_abs, Some(100));
    assert_eq!(rec.baseline_valid_runs, 3);
    assert!(
        rec.formal_metrics
            .iter()
            .any(|metric| metric.metric == "diagnostic_raw_score_total"
                && metric.statistically_significant)
    );
    for baseline in baselines {
        fs::remove_dir_all(baseline).ok();
    }
    fs::remove_dir_all(tune).ok();
}

#[test]
fn baseline_recommendation_formal_metrics_cover_all_key_metrics() {
    let baseline_inputs = [
        (4, 8_000_000, vec![17.0, 34.0, 51.0]),
        (5, 9_000_000, vec![18.0, 52.0, 53.0]),
        (6, 10_000_000, vec![55.0, 56.0, 57.0]),
    ];
    let baselines = baseline_inputs
        .iter()
        .enumerate()
        .map(|(idx, (score_over_5ms, max_ns, frames))| {
            let dir = temp_dir(&format!("formal-baseline-{idx}"));
            write_baseline_with_frames(&dir, *score_over_5ms, 100, *max_ns, frames);
            dir
        })
        .collect::<Vec<_>>();
    let tune = temp_dir("formal-tune");
    write_tune_with_candidates(
        &tune,
        RankingConfidence::High,
        100,
        vec![
            candidate_with_metrics(
                "best",
                CandidateMetricFixture {
                    iteration: 1,
                    diagnostic_raw_score_total: 100,
                    over_5ms: 1,
                    max_latency_ns: 1_000_000,
                    frame_p99_ms: 12.0,
                    frame_over_16ms: 0,
                    frame_over_33ms: 0,
                    frame_over_50ms: 0,
                },
            ),
            candidate_with_metrics(
                "best",
                CandidateMetricFixture {
                    iteration: 2,
                    diagnostic_raw_score_total: 110,
                    over_5ms: 2,
                    max_latency_ns: 2_000_000,
                    frame_p99_ms: 13.0,
                    frame_over_16ms: 1,
                    frame_over_33ms: 0,
                    frame_over_50ms: 0,
                },
            ),
            candidate_with_metrics(
                "best",
                CandidateMetricFixture {
                    iteration: 3,
                    diagnostic_raw_score_total: 120,
                    over_5ms: 3,
                    max_latency_ns: 3_000_000,
                    frame_p99_ms: 14.0,
                    frame_over_16ms: 2,
                    frame_over_33ms: 1,
                    frame_over_50ms: 0,
                },
            ),
        ],
    );

    let rec = build_baseline_tune_recommendation_for_baselines(&baselines, &tune).unwrap();

    assert_eq!(
        formal_metric_names(&rec.formal_metrics),
        EXPECTED_FORMAL_METRICS
    );
    for metric in &rec.formal_metrics {
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
    for baseline in baselines {
        fs::remove_dir_all(baseline).ok();
    }
    fs::remove_dir_all(tune).ok();
}

#[test]
fn single_baseline_run_needs_retest_with_explicit_sample_warning() {
    let baseline = temp_dir("single-baseline");
    write_baseline(&baseline, 2, 100);
    let tune = temp_dir("single-baseline-tune");
    write_tune(&tune, RankingConfidence::High, 100);

    let rec = build_baseline_tune_recommendation(&baseline, &tune).unwrap();

    assert_eq!(rec.verdict, TuneRecommendationVerdict::NeedsRetest);
    assert!(
        rec.formal_metrics
            .iter()
            .any(|metric| !metric.enough_samples)
    );
    assert!(
        rec.warnings
            .iter()
            .any(|warning| warning.contains("not enough samples for formal A/B comparison"))
    );
    fs::remove_dir_all(baseline).ok();
    fs::remove_dir_all(tune).ok();
}

#[test]
fn low_confidence_gives_needs_retest() {
    let baseline = temp_dir("low-baseline");
    write_baseline(&baseline, 2, 100);
    let tune = temp_dir("low-tune");
    write_tune(&tune, RankingConfidence::Low, 100);

    let rec = build_baseline_tune_recommendation(&baseline, &tune).unwrap();

    assert_eq!(rec.verdict, TuneRecommendationVerdict::NeedsRetest);
    fs::remove_dir_all(baseline).ok();
    fs::remove_dir_all(tune).ok();
}

#[test]
fn json_output_serializes_expected_fields() {
    let baseline = temp_dir("json-baseline");
    write_baseline(&baseline, 2, 100);
    let tune = temp_dir("json-tune");
    write_tune(&tune, RankingConfidence::High, 100);

    let rec = build_baseline_tune_recommendation(&baseline, &tune).unwrap();
    let json = serde_json::to_value(&rec).unwrap();

    assert_eq!(json["schema_version"], 3);
    assert_eq!(json["best_profile"], "best");
    assert_eq!(json["diagnostic_baseline_raw_score_total"], 200);
    assert_eq!(json["baseline_over_5ms"], 2);
    assert!(json["best_median_over_5ms"].is_number());
    assert!(json["over_5ms_delta_abs"].is_number());
    assert!(json["formal_metrics"].is_array());
    fs::remove_dir_all(baseline).ok();
    fs::remove_dir_all(tune).ok();
}

#[test]
fn zero_baseline_score_never_recommends() {
    let baseline = temp_dir("zero-baseline-score");
    write_baseline(&baseline, 0, 100); // score 0
    let tune = temp_dir("zero-tune-score");
    write_tune(&tune, RankingConfidence::High, 0); // score 0

    let rec = build_baseline_tune_recommendation(&baseline, &tune).unwrap();

    assert_eq!(rec.verdict, TuneRecommendationVerdict::NeedsRetest);
    assert!(
        rec.warnings
            .iter()
            .any(|w| w.contains("baseline score is zero"))
    );
    fs::remove_dir_all(baseline).ok();
    fs::remove_dir_all(tune).ok();
}

#[test]
fn very_low_baseline_score_needs_retest() {
    let baseline = temp_dir("low-baseline-score");
    write_baseline(&baseline, 0, 100);
    fs::write(
        baseline.join("interval.json"),
        format!(
            "{}\n",
            serde_json::to_string(&IntervalRecord {
                elapsed_ms: 1000,
                task: 1,
                active: true,
                class: TaskClass::Game,
                comm: "game".to_owned(),
                samples: 100,
                stored_samples: 100,
                over_1ms: 50,
                ..Default::default()
            })
            .unwrap()
        ),
    )
    .unwrap();

    let tune = temp_dir("low-tune-score");
    write_tune(&tune, RankingConfidence::High, 10); // score 10

    let rec = build_baseline_tune_recommendation(&baseline, &tune).unwrap();

    assert_eq!(rec.diagnostic_baseline_raw_score_total, 50);
    assert_eq!(rec.verdict, TuneRecommendationVerdict::NeedsRetest);
    assert!(
        rec.warnings
            .iter()
            .any(|w| w.contains("baseline score is very low (50)"))
    );
    fs::remove_dir_all(baseline).ok();
    fs::remove_dir_all(tune).ok();
}

#[test]
fn frame_p99_metrics_are_captured_structured() {
    let baseline = temp_dir("frame-baseline");
    write_baseline(&baseline, 2, 100);
    fs::write(
        baseline.join("frame_correlation.json"),
        r#"{"elapsed_ms":100,"frametime_ms":16.0}
{"elapsed_ms":200,"frametime_ms":17.0}
"#,
    )
    .unwrap();

    let tune = temp_dir("frame-tune");
    let summary = TuneSummary {
        schema_version: 1,
        tree_pid: 42,
        profiles_path: PathBuf::from("profiles.toml"),
        runs: 3,
        epoch_seconds: 60,
        warmup_seconds: 10,
        restore_policy: "restore-after-each".to_owned(),
        best_profile: "best".to_owned(),
        candidate_order: vec![],
        profile_stats: vec![TuneProfileStats {
            profile: "best".to_owned(),
            valid_runs: 3,
            invalid_runs: 0,
            median_diagnostic_raw_score_total: 100,
            iqr_diagnostic_raw_score_total: 0,
            worst_diagnostic_raw_score_total: 100,
            mean_diagnostic_raw_score_total: 100.0,
            stddev_diagnostic_raw_score_total: 0.0,
            median_over_5ms: 1,
            iqr_over_5ms: 0,
            mean_over_5ms: 1.0,
            stddev_over_5ms: 0.0,
            median_frame_p99_us: 15000,
            iqr_frame_p99_us: 0,
            mean_frame_p99_us: 15000.0,
            stddev_frame_p99_us: 0.0,
        }],
        ranking_confidence: RankingConfidence::High,
        ranking_notes: Vec::new(),
        comparability_warnings: Vec::new(),
        candidates: vec![candidate("best", 100)],
    };
    fs::write(
        tune.join("tuning_summary.json"),
        serde_json::to_vec_pretty(&summary).unwrap(),
    )
    .unwrap();

    let rec = build_baseline_tune_recommendation(&baseline, &tune).unwrap();
    let tune_only_markdown = super::render::render_tune_recommendation_for_summary(&summary);

    assert!(tune_only_markdown.contains("# stutter tuning recommendation"));
    assert!(tune_only_markdown.contains("Best profile: best"));
    assert_eq!(rec.baseline_frame_p99_ms, Some(17.0));
    assert_eq!(rec.best_median_frame_p99_ms, Some(15.0));
    assert_eq!(rec.frame_p99_delta_ms, Some(-2.0));

    fs::remove_dir_all(baseline).ok();
    fs::remove_dir_all(tune).ok();
}
