use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use super::*;
use crate::{
    advisor::{AdvisorFixValidationStatus, scheduler_profile_fix_plan},
    metrics::IntervalRecord,
    process_tree::TaskClass,
    recorder::{RecordedConfig, RecordedTime, SessionFile},
    tune::{
        RankingConfidence, TuneCandidateSummary, TuneCoverageMetrics, TuneIterationOrder,
        TuneProfileStats, TuneSummary, recommendation::TuneRecommendationVerdict,
    },
};

#[derive(Debug, serde::Deserialize)]
struct TuneAbFixture {
    name: String,
    scenario_name: String,
    expected_status: String,
    baseline_over_5ms: Vec<u64>,
    tuned_scores: Vec<u64>,
    tuned_over_5ms: u64,
    tuned_frame_p99_ms: f64,
}

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

fn write_baseline_with_scenario(
    dir: &Path,
    score_over_5ms: u64,
    samples: u64,
    scenario_name: &str,
) {
    let mut session = minimal_session(1);
    session.core.scenario_name = Some(scenario_name.to_owned());
    session.core.route_label = Some(scenario_name.to_owned());
    session.core.scenario_hash =
        crate::scenario::scenario_identity_hash(Some(scenario_name), None, Some(scenario_name));
    session.config.scenario_name = session.core.scenario_name.clone();
    session.config.route_label = session.core.route_label.clone();
    session.config.scenario_hash = session.core.scenario_hash.clone();

    fs::write(
        dir.join("session.json"),
        serde_json::to_vec_pretty(&session).unwrap(),
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
        profile_plan: None,
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

fn write_repeated_baselines_with_frames(name: &str, scores_over_5ms: &[u64]) -> Vec<PathBuf> {
    scores_over_5ms
        .iter()
        .enumerate()
        .map(|(idx, score)| {
            let dir = temp_dir(&format!("{name}-{idx}"));
            write_baseline_with_frames(&dir, *score, 100, 10_000_000, &[18.0, 19.0, 20.0]);
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
        scenario_name: None,
        scenario_hash: None,
        workload_label: None,
        route_label: None,
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
        order_strategy: "historical".to_owned(),
        order_balanced: true,
        order_balance_warning: None,
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

fn set_tune_scenario(dir: &Path, scenario_name: &str) {
    let path = dir.join("tuning_summary.json");
    let mut summary: TuneSummary = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    summary.scenario_name = Some(scenario_name.to_owned());
    summary.route_label = Some(scenario_name.to_owned());
    summary.scenario_hash =
        crate::scenario::scenario_identity_hash(Some(scenario_name), None, Some(scenario_name));
    fs::write(path, serde_json::to_vec_pretty(&summary).unwrap()).unwrap();
}

fn write_tune_with_candidate_scores(dir: &Path, scores: &[u64], over_5ms: u64, frame_p99_ms: f64) {
    write_tune_with_candidates(
        dir,
        RankingConfidence::High,
        scores.first().copied().unwrap_or_default(),
        scores
            .iter()
            .enumerate()
            .map(|(idx, score)| {
                candidate_with_metrics(
                    "best",
                    CandidateMetricFixture {
                        iteration: (idx + 1) as u32,
                        diagnostic_raw_score_total: *score,
                        over_5ms,
                        max_latency_ns: 1_000_000,
                        frame_p99_ms,
                        frame_over_16ms: 0,
                        frame_over_33ms: 0,
                        frame_over_50ms: 0,
                    },
                )
            })
            .collect(),
    );
}

fn scheduler_fix_plan() -> crate::advisor::AdvisorFixPlan {
    scheduler_profile_fix_plan(
        Path::new("/tmp/run"),
        crate::diagnosis::StutterCause::GameThreadSchedulerDelay,
        Some(42),
        Some(Path::new("profiles.toml")),
        Some("scheduler test evidence".to_owned()),
    )
}

fn load_tune_ab_fixture(name: &str) -> TuneAbFixture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tune_ab")
        .join(name)
        .join("fixture.toml");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("failed to read tune A/B fixture {}: {err}", path.display()));
    toml::from_str(&text)
        .unwrap_or_else(|err| panic!("failed to parse tune A/B fixture {}: {err}", path.display()))
}

fn validate_tune_ab_fixture(name: &str) -> crate::recommend::model::FixValidationReport {
    let fixture = load_tune_ab_fixture(name);
    assert_eq!(fixture.name, name);
    let baselines = write_repeated_baselines_with_frames(name, &fixture.baseline_over_5ms);
    let tune = temp_dir(&format!("{name}-tune"));
    write_tune_with_candidate_scores(
        &tune,
        &fixture.tuned_scores,
        fixture.tuned_over_5ms,
        fixture.tuned_frame_p99_ms,
    );

    let rec = build_baseline_tune_recommendation_for_baselines(&baselines, &tune).unwrap();
    let mut plan = scheduler_fix_plan();
    plan.validation.scenario_name = Some(fixture.scenario_name);
    let report = validate_fix_plan_against_recommendation(&plan, &rec);
    assert_eq!(
        format!("{:?}", report.status).to_ascii_lowercase(),
        fixture.expected_status
    );

    for baseline in baselines {
        fs::remove_dir_all(baseline).ok();
    }
    fs::remove_dir_all(tune).ok();
    report
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

#[path = "tests/validation.rs"]
mod validation;
