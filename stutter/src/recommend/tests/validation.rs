use super::*;

#[test]
fn recommend_fix_plan_validates_when_required_ci_excludes_zero() {
    let baselines = write_repeated_baselines_with_frames("fix-valid-baseline", &[3, 3, 3, 3, 3]);
    let tune = temp_dir("fix-valid-tune");
    write_tune_with_candidate_scores(&tune, &[100, 100, 100, 100, 100], 1, 18.0);

    let rec = build_baseline_tune_recommendation_for_baselines(&baselines, &tune).unwrap();
    let report = validate_fix_plan_against_recommendation(&scheduler_fix_plan(), &rec);

    assert_eq!(report.status, AdvisorFixValidationStatus::Validated);
    assert!(
        report
            .passed_criteria
            .iter()
            .any(|criterion| criterion.contains("diagnostic_raw_score_total"))
    );
    let html = super::super::render::render_fix_validation_report_html(&report);
    assert!(html.contains("Fix validation"));
    assert!(html.contains("Validated"));
    assert!(html.contains("diagnostic_raw_score_total"));
    assert!(html.contains("Expected") || html.contains("expected"));
    assert!(html.contains("Actual") || html.contains("actual"));

    for baseline in baselines {
        fs::remove_dir_all(baseline).ok();
    }
    fs::remove_dir_all(tune).ok();
}

#[test]
fn recommend_fix_plan_rejects_when_metric_regresses() {
    let baselines = write_repeated_baselines_with_frames("fix-reject-baseline", &[2, 2, 2, 2, 2]);
    let tune = temp_dir("fix-reject-tune");
    write_tune_with_candidate_scores(&tune, &[300, 300, 300, 300, 300], 3, 18.0);

    let rec = build_baseline_tune_recommendation_for_baselines(&baselines, &tune).unwrap();
    let report = validate_fix_plan_against_recommendation(&scheduler_fix_plan(), &rec);

    assert_eq!(report.status, AdvisorFixValidationStatus::Rejected);
    assert!(
        report
            .failed_criteria
            .iter()
            .any(|criterion| criterion.contains("metric regressed")
                || criterion.contains("wrong direction"))
    );

    for baseline in baselines {
        fs::remove_dir_all(baseline).ok();
    }
    fs::remove_dir_all(tune).ok();
}

#[test]
fn recommend_fix_plan_marks_underpowered_when_ci_missing() {
    let baselines = write_repeated_baselines_with_frames("fix-underpowered-baseline", &[3, 3]);
    let tune = temp_dir("fix-underpowered-tune");
    write_tune_with_candidate_scores(&tune, &[100, 100], 1, 18.0);

    let rec = build_baseline_tune_recommendation_for_baselines(&baselines, &tune).unwrap();
    let report = validate_fix_plan_against_recommendation(&scheduler_fix_plan(), &rec);

    assert_eq!(report.status, AdvisorFixValidationStatus::Underpowered);
    assert!(
        report
            .failed_criteria
            .iter()
            .any(|criterion| criterion.contains("not enough samples")
                || criterion.contains("required CI is missing"))
    );

    for baseline in baselines {
        fs::remove_dir_all(baseline).ok();
    }
    fs::remove_dir_all(tune).ok();
}

#[test]
fn recommend_fix_plan_blocks_on_comparability_failure() {
    let baselines = write_repeated_baselines_with_frames("fix-block-baseline", &[3, 3, 3, 3, 3]);
    let tune = temp_dir("fix-block-tune");
    write_tune_with_candidate_scores(&tune, &[100, 100, 100, 100, 100], 1, 18.0);

    let mut rec = build_baseline_tune_recommendation_for_baselines(&baselines, &tune).unwrap();
    rec.warnings.push(
        "comparability Reject profile=best kind=drop-counters-nonzero: profile has dropped events"
            .to_owned(),
    );
    let report = validate_fix_plan_against_recommendation(&scheduler_fix_plan(), &rec);

    assert_eq!(report.status, AdvisorFixValidationStatus::InvalidExperiment);
    assert!(
        report
            .blockers
            .contains(&super::super::model::FixValidationBlocker::DropCountersNonzero)
    );

    for baseline in baselines {
        fs::remove_dir_all(baseline).ok();
    }
    fs::remove_dir_all(tune).ok();
}

#[test]
fn recommend_fix_plan_blocks_on_scenario_mismatch_unless_allowed() {
    let baseline = temp_dir("fix-scenario-baseline");
    write_baseline_with_scenario(&baseline, 3, 100, "city-route-a");
    let tune = temp_dir("fix-scenario-tune");
    write_tune_with_candidate_scores(&tune, &[100, 100, 100, 100, 100], 1, 18.0);
    set_tune_scenario(&tune, "city-route-b");

    let rec =
        build_baseline_tune_recommendation_for_baselines(std::slice::from_ref(&baseline), &tune)
            .unwrap();
    assert!(
        rec.warnings
            .iter()
            .any(|warning| warning.contains("scenario-mismatch"))
    );
    let report = validate_fix_plan_against_recommendation(&scheduler_fix_plan(), &rec);
    assert_eq!(report.status, AdvisorFixValidationStatus::InvalidExperiment);
    assert!(
        report
            .blockers
            .contains(&super::super::model::FixValidationBlocker::ScenarioMismatch)
    );

    let allowed = build_baseline_tune_recommendation_for_baselines_with_options(
        std::slice::from_ref(&baseline),
        &tune,
        super::super::model::BaselineTuneRecommendationOptions {
            allow_scenario_mismatch: true,
        },
    )
    .unwrap();
    assert!(
        !allowed
            .warnings
            .iter()
            .any(|warning| warning.contains("scenario-mismatch"))
    );

    fs::remove_dir_all(baseline).ok();
    fs::remove_dir_all(tune).ok();
}

#[test]
fn real_ab_fixture_validates_scheduler_fix_when_ci_excludes_zero() {
    let report = validate_tune_ab_fixture("scheduler_affinity_validated");

    assert_eq!(report.status, AdvisorFixValidationStatus::Validated);
    assert!(report.blockers.is_empty());
    assert!(
        report
            .metric_results
            .iter()
            .any(|metric| metric.metric == "diagnostic_raw_score_total" && metric.passed)
    );
}

#[test]
fn real_ab_fixture_rejects_cpu_affinity_for_gpu_bound_case() {
    let report = validate_tune_ab_fixture("gpu_bound_cpu_affinity_rejected");

    assert_eq!(report.status, AdvisorFixValidationStatus::Rejected);
    assert!(
        report
            .failed_criteria
            .iter()
            .any(|criterion| criterion.contains("metric regressed")
                || criterion.contains("wrong direction"))
    );
}

#[test]
fn real_ab_fixture_marks_underpowered_when_baselines_are_missing() {
    let report = validate_tune_ab_fixture("scheduler_affinity_underpowered");

    assert_eq!(report.status, AdvisorFixValidationStatus::Underpowered);
    assert!(
        report
            .failed_criteria
            .iter()
            .any(|criterion| criterion.contains("not enough samples")
                || criterion.contains("required CI is missing"))
    );
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
    assert!(rec.score_effect_size.is_some());
    assert!(rec.score_noise_ratio.is_some());
    assert!(rec.over_5ms_effect_size.is_some());
    assert!(rec.over_5ms_noise_ratio.is_some());
    assert!(rec.frame_p99_effect_size.is_some());
    assert!(rec.frame_p99_noise_ratio.is_some());
    let metadata = rec.confidence_metadata.as_ref().unwrap();
    assert_eq!(metadata.ranking_confidence, RankingConfidence::High);
    assert_eq!(metadata.tune_runs, 3);
    assert_eq!(metadata.baseline_valid_runs, 3);

    let markdown = super::super::render::render_baseline_tune_recommendation_markdown(&rec);
    assert!(markdown.contains("Score effect size:"));
    assert!(markdown.contains("Score noise ratio:"));
    assert!(markdown.contains("Over 5ms effect size:"));
    assert!(markdown.contains("Frame p99 effect size:"));
    assert!(markdown.contains("Tune runs: 3 configured"));

    let html = super::super::render::render_baseline_tune_recommendation_html(&rec);
    assert!(html.contains("Unified comparison metrics"));
    assert!(html.contains("diagnostic_raw_score_total"));
    assert!(html.contains("Noise ratio"));
    assert!(html.contains("Tune runs"));
    assert!(html.contains("A/B uncertainty"));
    assert!(html.contains("Sample distribution"));
    assert!(html.contains("Bootstrap median-improvement CI"));
    assert!(html.contains("Effect size"));
    assert!(html.contains("Samples"));
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
fn baseline_tune_html_exposes_distribution_charts_ci_bands_and_underpowered_warnings() {
    let tune = temp_dir("html-ab-uncertainty-tune");
    write_tune_with_candidates(
        &tune,
        RankingConfidence::Low,
        100,
        vec![
            candidate_with_iteration("best", 1, 100),
            candidate_with_iteration("best", 2, 98),
        ],
    );

    let baselines = write_repeated_baselines("html-ab-uncertainty-baseline", &[120, 118]);

    let rec = build_baseline_tune_recommendation_for_baselines(&baselines, &tune).unwrap();
    let html = super::super::render::render_baseline_tune_recommendation_html(&rec);

    assert!(html.contains("A/B uncertainty"));
    assert!(html.contains("Sample distribution"));
    assert!(html.contains("Bootstrap median-improvement CI") || html.contains("No CI band"));
    assert!(html.contains("Effect size"));
    assert!(html.contains("Noise ratio"));
    assert!(html.contains("underpowered"));
    assert!(html.contains("not enough samples"));

    for baseline in baselines {
        fs::remove_dir_all(baseline).ok();
    }
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

    assert_eq!(json["schema_version"], 5);
    assert_eq!(json["best_profile"], "best");
    assert_eq!(json["diagnostic_baseline_raw_score_total"], 200);
    assert_eq!(json["baseline_over_5ms"], 2);
    assert!(json["best_median_over_5ms"].is_number());
    assert!(json["over_5ms_delta_abs"].is_number());
    assert!(json["formal_metrics"].is_array());
    assert!(json["confidence_metadata"].is_object());
    assert!(json["score_effect_size"].is_null() || json["score_effect_size"].is_number());
    assert!(json["score_noise_ratio"].is_null() || json["score_noise_ratio"].is_number());
    assert!(json["over_5ms_effect_size"].is_null() || json["over_5ms_effect_size"].is_number());
    assert!(json["frame_p99_effect_size"].is_null() || json["frame_p99_effect_size"].is_number());
    fs::remove_dir_all(baseline).ok();
    fs::remove_dir_all(tune).ok();
}

#[test]
fn zero_baseline_score_never_recommends() {
    let baseline = temp_dir("zero-baseline-score");
    write_baseline(&baseline, 0, 100);
    let tune = temp_dir("zero-tune-score");
    write_tune(&tune, RankingConfidence::High, 0);

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
    write_tune(&tune, RankingConfidence::High, 10);

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
        scenario_name: None,
        scenario_hash: None,
        workload_label: None,
        route_label: None,
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
    let tune_only_markdown = super::super::render::render_tune_recommendation_for_summary(&summary);

    assert!(tune_only_markdown.contains("# stutter tuning recommendation"));
    assert!(tune_only_markdown.contains("Best profile: best"));
    assert_eq!(rec.baseline_frame_p99_ms, Some(17.0));
    assert_eq!(rec.best_median_frame_p99_ms, Some(15.0));
    assert_eq!(rec.frame_p99_delta_ms, Some(-2.0));

    fs::remove_dir_all(baseline).ok();
    fs::remove_dir_all(tune).ok();
}
