use super::experiment::WindowScore;

#[derive(Clone, Debug, PartialEq)]
pub enum ExperimentResult {
    Improved { improvement_percent: f64 },
    Regressed { regression_percent: f64 },
    Inconclusive { reason: String },
    Invalid { reason: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExperimentDataQuality {
    High,
    Medium,
    Low,
}

impl ExperimentDataQuality {
    pub fn is_acceptable_for_improvement(self) -> bool {
        matches!(self, Self::High | Self::Medium)
    }

    pub fn is_low(self) -> bool {
        matches!(self, Self::Low)
    }
}

#[derive(Clone, Debug)]
pub struct ExperimentComparisonInput<'a> {
    pub baseline: &'a WindowScore,
    pub candidate: &'a WindowScore,
    pub data_quality: ExperimentDataQuality,
    pub target_disappeared: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScoreComparisonConfig {
    pub min_improvement_percent: f64,
    pub max_regression_percent: f64,
    pub max_frame_p99_regression_ms: f64,
    pub max_over_5ms_regression: u64,
}

pub(crate) const DEFAULT_SCORE_COMPARISON_CONFIG: ScoreComparisonConfig = ScoreComparisonConfig {
    min_improvement_percent: 12.5,
    max_regression_percent: 7.5,
    max_frame_p99_regression_ms: 2.0,
    max_over_5ms_regression: 0,
};

#[derive(Clone, Debug, PartialEq)]
pub struct ThresholdTier {
    pub min_scored_samples: u64,
    pub min_improvement_percent: f64,
    pub max_regression_percent: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThresholdPolicy {
    pub tiers: Vec<ThresholdTier>,
}

impl ThresholdPolicy {
    pub fn default_tiers() -> Self {
        Self {
            tiers: vec![
                ThresholdTier {
                    min_scored_samples: 0,
                    min_improvement_percent: 15.0,
                    max_regression_percent: 5.0,
                },
                ThresholdTier {
                    min_scored_samples: 200,
                    min_improvement_percent: 12.5,
                    max_regression_percent: 7.5,
                },
                ThresholdTier {
                    min_scored_samples: 1_000,
                    min_improvement_percent: 10.0,
                    max_regression_percent: 8.5,
                },
            ],
        }
    }

    pub fn config_for_samples(&self, scored_samples: u64) -> ScoreComparisonConfig {
        let tier = self
            .tiers
            .iter()
            .filter(|tier| tier.min_scored_samples <= scored_samples)
            .max_by_key(|tier| tier.min_scored_samples);

        match tier {
            Some(tier) => ScoreComparisonConfig {
                min_improvement_percent: tier.min_improvement_percent,
                max_regression_percent: tier.max_regression_percent,
                ..DEFAULT_SCORE_COMPARISON_CONFIG
            },
            None => DEFAULT_SCORE_COMPARISON_CONFIG,
        }
    }
}

impl Default for ScoreComparisonConfig {
    fn default() -> Self {
        DEFAULT_SCORE_COMPARISON_CONFIG
    }
}

#[derive(Clone, Debug)]
pub struct ScoreComparisonInput<'a> {
    pub baseline: &'a WindowScore,
    pub candidate: &'a WindowScore,
    pub data_quality: ExperimentDataQuality,
    pub target_disappeared: bool,
}

#[derive(Clone, Debug)]
pub struct ExperimentComparisonPolicy {
    pub min_improvement_ratio: f64,
    pub max_regression_ratio: f64,
    pub improved_frame_p99_slack_ms: f64,
    pub regressed_frame_p99_slack_ms: f64,
}

impl Default for ExperimentComparisonPolicy {
    fn default() -> Self {
        Self {
            min_improvement_ratio: 1.0
                - (DEFAULT_SCORE_COMPARISON_CONFIG.min_improvement_percent / 100.0),
            max_regression_ratio: 1.0
                + (DEFAULT_SCORE_COMPARISON_CONFIG.max_regression_percent / 100.0),
            improved_frame_p99_slack_ms: 1.0,
            regressed_frame_p99_slack_ms: DEFAULT_SCORE_COMPARISON_CONFIG
                .max_frame_p99_regression_ms,
        }
    }
}

impl ExperimentComparisonPolicy {
    pub fn as_score_comparison_config(&self) -> ScoreComparisonConfig {
        ScoreComparisonConfig {
            min_improvement_percent: ratio_to_improvement_percent(self.min_improvement_ratio),
            max_regression_percent: ratio_to_regression_percent(self.max_regression_ratio),
            max_frame_p99_regression_ms: self.regressed_frame_p99_slack_ms,
            max_over_5ms_regression: DEFAULT_SCORE_COMPARISON_CONFIG.max_over_5ms_regression,
        }
    }
}

pub fn compare_experiment(input: ExperimentComparisonInput<'_>) -> ExperimentResult {
    compare_experiment_with_policy(input, &ExperimentComparisonPolicy::default())
}

pub fn compare_experiment_with_policy(
    input: ExperimentComparisonInput<'_>,
    policy: &ExperimentComparisonPolicy,
) -> ExperimentResult {
    compare_scores_with_config(
        ScoreComparisonInput {
            baseline: input.baseline,
            candidate: input.candidate,
            data_quality: input.data_quality,
            target_disappeared: input.target_disappeared,
        },
        &policy.as_score_comparison_config(),
        None,
    )
}

pub fn compare_scores(input: ScoreComparisonInput<'_>) -> ExperimentResult {
    compare_scores_with_config(input, &ScoreComparisonConfig::default(), None)
}

pub fn compare_scores_with_config(
    input: ScoreComparisonInput<'_>,
    config: &ScoreComparisonConfig,
    threshold_policy: Option<&ThresholdPolicy>,
) -> ExperimentResult {
    let effective_config = threshold_policy
        .map(|policy| policy.config_for_samples(input.baseline.scored_samples))
        .unwrap_or(*config);
    let config = &effective_config;

    if let Err(reason) = validate_score_comparison_config(config) {
        return ExperimentResult::Invalid { reason };
    }

    if let Err(reason) = validate_window_score("baseline", input.baseline) {
        return ExperimentResult::Invalid { reason };
    }

    if let Err(reason) = validate_window_score("candidate", input.candidate) {
        return ExperimentResult::Invalid { reason };
    }

    let regression_percent =
        regression_percent(input.baseline.score.total, input.candidate.score.total);

    if input.target_disappeared {
        return ExperimentResult::Regressed { regression_percent };
    }

    if input.data_quality.is_low() {
        return ExperimentResult::Regressed { regression_percent };
    }

    if regression_percent > config.max_regression_percent {
        return ExperimentResult::Regressed { regression_percent };
    }

    if input.candidate.score.over_5ms
        > input
            .baseline
            .score
            .over_5ms
            .saturating_add(config.max_over_5ms_regression)
    {
        return ExperimentResult::Regressed { regression_percent };
    }

    if input.candidate.score.frame_p99_ms
        > input.baseline.score.frame_p99_ms + config.max_frame_p99_regression_ms
    {
        return ExperimentResult::Regressed { regression_percent };
    }

    let improvement_percent =
        improvement_percent(input.baseline.score.total, input.candidate.score.total);

    if input.data_quality.is_acceptable_for_improvement()
        && improvement_percent >= config.min_improvement_percent
        && input.candidate.score.over_5ms <= input.baseline.score.over_5ms
        && input.candidate.score.frame_p99_ms
            <= input.baseline.score.frame_p99_ms + config.max_frame_p99_regression_ms
    {
        return ExperimentResult::Improved {
            improvement_percent,
        };
    }

    ExperimentResult::Inconclusive {
        reason: format!(
            "candidate did not meet conservative thresholds: improvement_percent={:.2} min_improvement_percent={:.2} regression_percent={:.2} max_regression_percent={:.2}",
            improvement_percent,
            config.min_improvement_percent,
            regression_percent,
            config.max_regression_percent
        ),
    }
}

fn validate_score_comparison_config(config: &ScoreComparisonConfig) -> Result<(), String> {
    if !config.min_improvement_percent.is_finite() || config.min_improvement_percent < 0.0 {
        return Err(format!(
            "min_improvement_percent must be finite and non-negative: {}",
            config.min_improvement_percent
        ));
    }

    if !config.max_regression_percent.is_finite() || config.max_regression_percent < 0.0 {
        return Err(format!(
            "max_regression_percent must be finite and non-negative: {}",
            config.max_regression_percent
        ));
    }

    if !config.max_frame_p99_regression_ms.is_finite() || config.max_frame_p99_regression_ms < 0.0 {
        return Err(format!(
            "max_frame_p99_regression_ms must be finite and non-negative: {}",
            config.max_frame_p99_regression_ms
        ));
    }

    Ok(())
}

fn validate_window_score(label: &str, score: &WindowScore) -> Result<(), String> {
    if score.interval_count == 0 {
        return Err(format!("{label} window has zero intervals"));
    }

    if score.scored_samples == 0 {
        return Err(format!("{label} window has zero scored samples"));
    }

    if score.scored_task_count == 0 {
        return Err(format!("{label} window has zero scored tasks"));
    }

    if !score.score.frame_p99_ms.is_finite() {
        return Err(format!("{label} frame_p99_ms is not finite"));
    }

    if !score.score.frame_max_ms.is_finite() {
        return Err(format!("{label} frame_max_ms is not finite"));
    }

    Ok(())
}

fn ratio_to_improvement_percent(ratio: f64) -> f64 {
    if !ratio.is_finite() {
        0.0
    } else {
        ((1.0 - ratio) * 100.0).max(0.0)
    }
}

fn ratio_to_regression_percent(ratio: f64) -> f64 {
    if !ratio.is_finite() {
        0.0
    } else {
        ((ratio - 1.0) * 100.0).max(0.0)
    }
}

fn improvement_percent(baseline_total: u64, candidate_total: u64) -> f64 {
    if baseline_total == 0 || candidate_total >= baseline_total {
        0.0
    } else {
        ((baseline_total - candidate_total) as f64 / baseline_total as f64) * 100.0
    }
}

pub(crate) fn regression_percent(baseline_total: u64, candidate_total: u64) -> f64 {
    if candidate_total <= baseline_total {
        0.0
    } else if baseline_total == 0 {
        100.0
    } else {
        ((candidate_total - baseline_total) as f64 / baseline_total as f64) * 100.0
    }
}

#[cfg(test)]
mod tests {
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
        assert_eq!(config.max_over_5ms_regression, 0);

        assert!(
            (config.min_improvement_percent - policy_config.min_improvement_percent).abs() < 1e-9
        );
        assert!(
            (config.max_regression_percent - policy_config.max_regression_percent).abs() < 1e-9
        );
        assert_eq!(
            config.max_frame_p99_regression_ms,
            policy_config.max_frame_p99_regression_ms
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
                max_over_5ms_regression: 0,
            },
            None,
        );

        match result {
            ExperimentResult::Inconclusive { reason } => {
                assert!(reason.contains("candidate did not meet conservative thresholds"));
                assert!(reason.contains("improvement_percent=10.00"));
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
        let candidate = window(890, 10, 12.0);

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
                max_over_5ms_regression: 0,
            },
            Some(&ThresholdPolicy::default_tiers()),
        );

        assert!(matches!(result, ExperimentResult::Improved { .. }));
    }
}
