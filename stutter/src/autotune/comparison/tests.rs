use super::*;
use crate::scorer::StutterScore;

fn window(total: u64, over_5ms: u64, frame_p99_ms: f64) -> WindowScore {
    WindowScore {
        started_unix_nanos: 100,
        finished_unix_nanos: 200,
        interval_count: 10,
        scored_samples: 100,
        scored_task_count: 2,
        score: StutterScore {
            total,
            over_5ms,
            frame_p99_ms,
            frame_max_ms: frame_p99_ms,
            ..StutterScore::default()
        },
    }
}

fn window_with_samples(
    total: u64,
    over_5ms: u64,
    frame_p99_ms: f64,
    scored_samples: u64,
    interval_count: usize,
) -> WindowScore {
    let mut score = window(total, over_5ms, frame_p99_ms);
    score.scored_samples = scored_samples;
    score.interval_count = interval_count;
    score
}

fn input<'a>(
    baseline: &'a WindowScore,
    candidate: &'a WindowScore,
    data_quality: ExperimentDataQuality,
    target_disappeared: bool,
) -> ExperimentComparisonInput<'a> {
    ExperimentComparisonInput {
        baseline,
        candidate,
        data_quality,
        target_disappeared,
    }
}

fn high_quality_result(baseline: &WindowScore, candidate: &WindowScore) -> ExperimentResult {
    compare_experiment(input(
        baseline,
        candidate,
        ExperimentDataQuality::High,
        false,
    ))
}

#[test]
fn normalized_score_rates_drive_experiment_result() {
    assert!(matches!(
        high_quality_result(
            &window_with_samples(1_000, 10, 12.0, 100, 10),
            &window_with_samples(3_000, 30, 12.0, 300, 30)
        ),
        ExperimentResult::Inconclusive { .. }
    ));
    assert!(matches!(
        high_quality_result(
            &window_with_samples(1_000, 10, 12.0, 1_000, 100),
            &window_with_samples(900, 1, 12.0, 100, 10)
        ),
        ExperimentResult::Regressed { .. }
    ));
    assert!(matches!(
        high_quality_result(
            &window_with_samples(1_000, 10, 12.0, 100, 10),
            &window_with_samples(1_500, 30, 12.0, 300, 30)
        ),
        ExperimentResult::Improved { .. }
    ));

    match high_quality_result(
        &window_with_samples(1_000, 10, 12.0, 100, 10),
        &window_with_samples(875, 10, 13.0, 100, 10),
    ) {
        ExperimentResult::Improved {
            improvement_percent,
        } => assert!((improvement_percent - 12.5).abs() < 0.0001),
        other => panic!("expected Improved, got {other:?}"),
    }
}

#[test]
fn longer_candidate_window_with_same_over_5ms_rate_is_not_rejected() {
    let baseline = window_with_samples(1_000, 10, 12.0, 100, 10);
    let candidate = window_with_samples(2_400, 30, 12.0, 300, 30);

    assert!(matches!(
        high_quality_result(&baseline, &candidate),
        ExperimentResult::Improved { .. }
    ));
}

#[test]
fn normalized_comparison_is_invariant_across_equal_rate_window_scales() {
    let baseline = window_with_samples(1_000, 10, 12.0, 100, 10);

    for scale in [1_u64, 2, 3, 5, 8, 13] {
        let candidate = window_with_samples(
            1_000 * scale,
            10 * scale,
            12.0,
            100 * scale,
            10 * scale as usize,
        );

        assert!(
            matches!(
                high_quality_result(&baseline, &candidate),
                ExperimentResult::Inconclusive { .. }
            ),
            "same score and tail rates must stay inconclusive at scale {scale}"
        );
    }
}

#[test]
fn normalized_comparison_keeps_improvement_when_candidate_window_is_longer() {
    let baseline = window_with_samples(1_000, 10, 12.0, 100, 10);

    for scale in [1_u64, 2, 4, 10] {
        let candidate = window_with_samples(
            800 * scale,
            10 * scale,
            12.0,
            100 * scale,
            10 * scale as usize,
        );

        assert!(
            matches!(
                high_quality_result(&baseline, &candidate),
                ExperimentResult::Improved { .. }
            ),
            "20% per-sample score improvement must survive scale {scale}"
        );
    }
}

#[test]
fn normalized_tail_regression_blocks_raw_total_improvement() {
    let baseline = window_with_samples(1_000, 10, 12.0, 100, 10);
    let candidate = window_with_samples(2_400, 60, 12.0, 400, 40);

    assert!(matches!(
        high_quality_result(&baseline, &candidate),
        ExperimentResult::Regressed { .. }
    ));
}

#[test]
fn improved_when_total_drops_by_at_least_twelve_point_five_percent_and_spikes_do_not_worsen() {
    let baseline = window(1_000, 10, 12.0);
    let candidate = window(875, 10, 13.0);

    let result = compare_experiment(input(
        &baseline,
        &candidate,
        ExperimentDataQuality::High,
        false,
    ));

    match result {
        ExperimentResult::Improved {
            improvement_percent,
        } => {
            assert!((improvement_percent - 12.5).abs() < 0.0001);
        }
        other => panic!("expected Improved, got {other:?}"),
    }
}

#[test]
fn medium_data_quality_can_still_improve() {
    let baseline = window(1_000, 10, 12.0);
    let candidate = window(800, 9, 12.5);

    let result = compare_experiment(input(
        &baseline,
        &candidate,
        ExperimentDataQuality::Medium,
        false,
    ));

    assert!(matches!(result, ExperimentResult::Improved { .. }));
}

#[test]
fn candidate_total_regression_is_regressed() {
    let baseline = window(1_000, 10, 12.0);
    let candidate = window(1_075, 10, 12.0);

    let result = compare_experiment(input(
        &baseline,
        &candidate,
        ExperimentDataQuality::High,
        false,
    ));

    match result {
        ExperimentResult::Regressed { regression_percent } => {
            assert!((regression_percent - 7.5).abs() < 0.0001);
        }
        other => panic!("expected Regressed, got {other:?}"),
    }
}

#[test]
fn over_5ms_increase_is_regressed_even_when_total_improves() {
    let baseline = window(1_000, 10, 12.0);
    let candidate = window(700, 11, 12.0);

    let result = compare_experiment(input(
        &baseline,
        &candidate,
        ExperimentDataQuality::High,
        false,
    ));

    assert!(matches!(result, ExperimentResult::Regressed { .. }));
}

#[test]
fn frame_p99_more_than_two_ms_worse_is_regressed() {
    let baseline = window(1_000, 10, 12.0);
    let candidate = window(700, 10, 14.01);

    let result = compare_experiment(input(
        &baseline,
        &candidate,
        ExperimentDataQuality::High,
        false,
    ));

    assert!(matches!(result, ExperimentResult::Regressed { .. }));
}

#[test]
fn target_disappeared_is_regressed() {
    let baseline = window(1_000, 10, 12.0);
    let candidate = window(700, 10, 12.0);

    let result = compare_experiment(input(
        &baseline,
        &candidate,
        ExperimentDataQuality::High,
        true,
    ));

    assert!(matches!(result, ExperimentResult::Regressed { .. }));
}

#[test]
fn low_data_quality_is_regressed() {
    let baseline = window(1_000, 10, 12.0);
    let candidate = window(700, 10, 12.0);

    let result = compare_experiment(input(
        &baseline,
        &candidate,
        ExperimentDataQuality::Low,
        false,
    ));

    assert!(matches!(result, ExperimentResult::Regressed { .. }));
}

#[test]
fn candidate_that_does_not_clear_improvement_threshold_is_inconclusive() {
    let baseline = window(1_000, 10, 12.0);
    let candidate = window(900, 10, 12.5);

    let result = compare_experiment(input(
        &baseline,
        &candidate,
        ExperimentDataQuality::High,
        false,
    ));

    match result {
        ExperimentResult::Inconclusive { reason } => {
            assert!(reason.contains("did not meet conservative"));
        }
        other => panic!("expected Inconclusive, got {other:?}"),
    }
}

#[test]
fn improved_total_with_frame_p99_more_than_two_ms_worse_is_regressed() {
    let baseline = window(1_000, 10, 12.0);
    let candidate = window(800, 10, 14.5);

    let result = compare_experiment(input(
        &baseline,
        &candidate,
        ExperimentDataQuality::High,
        false,
    ));

    assert!(matches!(result, ExperimentResult::Regressed { .. }));
}

#[test]
fn invalid_baseline_window_is_invalid() {
    let mut baseline = window(1_000, 10, 12.0);
    baseline.interval_count = 0;
    let candidate = window(800, 10, 12.0);

    let result = compare_experiment(input(
        &baseline,
        &candidate,
        ExperimentDataQuality::High,
        false,
    ));

    match result {
        ExperimentResult::Invalid { reason } => {
            assert_eq!(reason, "baseline window has zero intervals");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn invalid_candidate_window_is_invalid() {
    let baseline = window(1_000, 10, 12.0);
    let mut candidate = window(800, 10, 12.0);
    candidate.scored_samples = 0;

    let result = compare_experiment(input(
        &baseline,
        &candidate,
        ExperimentDataQuality::High,
        false,
    ));

    match result {
        ExperimentResult::Invalid { reason } => {
            assert_eq!(reason, "candidate window has zero scored samples");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn score_comparison_config_default_matches_existing_conservative_policy() {
    let config = ScoreComparisonConfig::default();
    let policy_config = ExperimentComparisonPolicy::default().as_score_comparison_config();

    assert_eq!(config, DEFAULT_SCORE_COMPARISON_CONFIG);
    assert_eq!(config.min_improvement_percent, 12.5);
    assert_eq!(config.max_regression_percent, 7.5);
    assert_eq!(config.max_frame_p99_regression_ms, 2.0);
    assert_eq!(config.max_over_5ms_regression_per_1k_samples, 0.0);
    assert_eq!(config.max_over_5ms_regression, 0);

    assert!((config.min_improvement_percent - policy_config.min_improvement_percent).abs() < 1e-9);
    assert!((config.max_regression_percent - policy_config.max_regression_percent).abs() < 1e-9);
    assert_eq!(
        config.max_frame_p99_regression_ms,
        policy_config.max_frame_p99_regression_ms
    );
    assert_eq!(
        config.max_over_5ms_regression_per_1k_samples,
        policy_config.max_over_5ms_regression_per_1k_samples
    );
    assert_eq!(
        config.max_over_5ms_regression,
        policy_config.max_over_5ms_regression
    );
}

#[test]
fn compare_scores_improves_when_min_percent_threshold_is_met() {
    let baseline = window(1_000, 10, 12.0);
    let candidate = window(800, 10, 13.0);

    let result = compare_scores_with_config(
        ScoreComparisonInput {
            baseline: &baseline,
            candidate: &candidate,
            data_quality: ExperimentDataQuality::High,
            target_disappeared: false,
        },
        &ScoreComparisonConfig {
            min_improvement_percent: 20.0,
            max_regression_percent: 7.5,
            max_frame_p99_regression_ms: 2.0,
            max_over_5ms_regression_per_1k_samples: 0.0,
            max_over_5ms_regression: 0,
        },
        None,
    );

    match result {
        ExperimentResult::Improved {
            improvement_percent,
        } => assert!((improvement_percent - 20.0).abs() < 0.0001),
        other => panic!("expected Improved, got {other:?}"),
    }
}

#[test]
fn compare_scores_is_inconclusive_when_improvement_percent_is_too_small() {
    let baseline = window(1_000, 10, 12.0);
    let candidate = window(900, 10, 12.0);

    let result = compare_scores_with_config(
        ScoreComparisonInput {
            baseline: &baseline,
            candidate: &candidate,
            data_quality: ExperimentDataQuality::High,
            target_disappeared: false,
        },
        &ScoreComparisonConfig {
            min_improvement_percent: 12.5,
            max_regression_percent: 7.5,
            max_frame_p99_regression_ms: 2.0,
            max_over_5ms_regression_per_1k_samples: 0.0,
            max_over_5ms_regression: 0,
        },
        None,
    );

    match result {
        ExperimentResult::Inconclusive { reason } => {
            assert!(reason.contains("candidate did not meet conservative thresholds"));
            assert!(reason.contains("improvement_percent=10.00"));
            assert!(reason.contains("baseline_score_per_sample=10.000000"));
            assert!(reason.contains("candidate_score_per_sample=9.000000"));
            assert!(reason.contains("baseline_raw_total=1000"));
            assert!(reason.contains("candidate_raw_total=900"));
            assert!(reason.contains("baseline_scored_samples=100"));
            assert!(reason.contains("candidate_scored_samples=100"));
        }
        other => panic!("expected Inconclusive, got {other:?}"),
    }
}

#[test]
fn compare_scores_regresses_when_max_regression_percent_is_exceeded() {
    let baseline = window(1_000, 10, 12.0);
    let candidate = window(1_081, 10, 12.0);

    let result = compare_scores_with_config(
        ScoreComparisonInput {
            baseline: &baseline,
            candidate: &candidate,
            data_quality: ExperimentDataQuality::High,
            target_disappeared: false,
        },
        &ScoreComparisonConfig {
            min_improvement_percent: 12.5,
            max_regression_percent: 7.5,
            max_frame_p99_regression_ms: 2.0,
            max_over_5ms_regression_per_1k_samples: 0.0,
            max_over_5ms_regression: 0,
        },
        None,
    );

    match result {
        ExperimentResult::Regressed { regression_percent } => {
            assert!((regression_percent - 8.1).abs() < 0.0001);
        }
        other => panic!("expected Regressed, got {other:?}"),
    }
}

#[test]
fn compare_scores_regresses_when_frame_p99_regression_exceeds_config() {
    let baseline = window(1_000, 10, 12.0);
    let candidate = window(800, 10, 14.1);

    let result = compare_scores_with_config(
        ScoreComparisonInput {
            baseline: &baseline,
            candidate: &candidate,
            data_quality: ExperimentDataQuality::High,
            target_disappeared: false,
        },
        &ScoreComparisonConfig {
            min_improvement_percent: 12.5,
            max_regression_percent: 7.5,
            max_frame_p99_regression_ms: 2.0,
            max_over_5ms_regression_per_1k_samples: 0.0,
            max_over_5ms_regression: 0,
        },
        None,
    );

    assert!(matches!(result, ExperimentResult::Regressed { .. }));
}

#[test]
fn compare_scores_regresses_when_over_5ms_regression_exceeds_config() {
    let baseline = window(1_000, 10, 12.0);
    let candidate = window(800, 12, 12.0);

    let result = compare_scores_with_config(
        ScoreComparisonInput {
            baseline: &baseline,
            candidate: &candidate,
            data_quality: ExperimentDataQuality::High,
            target_disappeared: false,
        },
        &ScoreComparisonConfig {
            min_improvement_percent: 12.5,
            max_regression_percent: 7.5,
            max_frame_p99_regression_ms: 2.0,
            max_over_5ms_regression_per_1k_samples: 10.0,
            max_over_5ms_regression: 1,
        },
        None,
    );

    assert!(matches!(result, ExperimentResult::Regressed { .. }));
}

#[test]
fn compare_scores_improvement_requires_no_over_5ms_increase_even_when_regression_guard_allows_one()
{
    let baseline = window(1_000, 10, 12.0);
    let candidate = window(800, 11, 12.0);

    let result = compare_scores_with_config(
        ScoreComparisonInput {
            baseline: &baseline,
            candidate: &candidate,
            data_quality: ExperimentDataQuality::High,
            target_disappeared: false,
        },
        &ScoreComparisonConfig {
            min_improvement_percent: 12.5,
            max_regression_percent: 7.5,
            max_frame_p99_regression_ms: 2.0,
            max_over_5ms_regression_per_1k_samples: 10.0,
            max_over_5ms_regression: 1,
        },
        None,
    );

    assert!(matches!(result, ExperimentResult::Inconclusive { .. }));
}

#[test]
fn compare_scores_rejects_invalid_config() {
    let baseline = window(1_000, 10, 12.0);
    let candidate = window(800, 10, 12.0);

    let result = compare_scores_with_config(
        ScoreComparisonInput {
            baseline: &baseline,
            candidate: &candidate,
            data_quality: ExperimentDataQuality::High,
            target_disappeared: false,
        },
        &ScoreComparisonConfig {
            min_improvement_percent: f64::NAN,
            max_regression_percent: 7.5,
            max_frame_p99_regression_ms: 2.0,
            max_over_5ms_regression_per_1k_samples: 0.0,
            max_over_5ms_regression: 0,
        },
        None,
    );

    match result {
        ExperimentResult::Invalid { reason } => {
            assert!(reason.contains("min_improvement_percent must be finite"));
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn non_finite_frame_p99_is_invalid() {
    let baseline = window(1_000, 10, 12.0);
    let candidate = window(800, 10, f64::NAN);

    let result = compare_experiment(input(
        &baseline,
        &candidate,
        ExperimentDataQuality::High,
        false,
    ));

    match result {
        ExperimentResult::Invalid { reason } => {
            assert_eq!(reason, "candidate frame_p99_ms is not finite");
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn threshold_policy_selects_sample_count_tiers() {
    let policy = ThresholdPolicy::default_tiers();

    let low = policy.config_for_samples(199);
    assert_eq!(low.min_improvement_percent, 15.0);
    assert_eq!(low.max_regression_percent, 5.0);

    let medium = policy.config_for_samples(200);
    assert_eq!(medium.min_improvement_percent, 12.5);
    assert_eq!(medium.max_regression_percent, 7.5);

    let high = policy.config_for_samples(1_000);
    assert_eq!(high.min_improvement_percent, 10.0);
    assert_eq!(high.max_regression_percent, 8.5);
}

#[test]
fn compare_scores_can_use_threshold_policy() {
    let mut baseline = window(1_000, 10, 12.0);
    baseline.scored_samples = 1_000;
    let mut candidate = window(890, 10, 12.0);
    candidate.scored_samples = 1_000;

    let result = compare_scores_with_config(
        ScoreComparisonInput {
            baseline: &baseline,
            candidate: &candidate,
            data_quality: ExperimentDataQuality::High,
            target_disappeared: false,
        },
        &ScoreComparisonConfig {
            min_improvement_percent: 12.5,
            max_regression_percent: 7.5,
            max_frame_p99_regression_ms: 2.0,
            max_over_5ms_regression_per_1k_samples: 0.0,
            max_over_5ms_regression: 0,
        },
        Some(&ThresholdPolicy::default_tiers()),
    );

    assert!(matches!(result, ExperimentResult::Improved { .. }));
}
