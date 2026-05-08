#![allow(dead_code)]

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
            min_improvement_ratio: 0.875,
            max_regression_ratio: 1.075,
            improved_frame_p99_slack_ms: 1.0,
            regressed_frame_p99_slack_ms: 2.0,
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
    if let Err(reason) = validate_window_score("baseline", input.baseline) {
        return ExperimentResult::Invalid { reason };
    }

    if let Err(reason) = validate_window_score("candidate", input.candidate) {
        return ExperimentResult::Invalid { reason };
    }

    if input.target_disappeared {
        return ExperimentResult::Regressed {
            regression_percent: regression_percent(
                input.baseline.score.total,
                input.candidate.score.total,
            ),
        };
    }

    if input.data_quality.is_low() {
        return ExperimentResult::Regressed {
            regression_percent: regression_percent(
                input.baseline.score.total,
                input.candidate.score.total,
            ),
        };
    }

    if input.candidate.score.total as f64
        >= input.baseline.score.total as f64 * policy.max_regression_ratio
    {
        return ExperimentResult::Regressed {
            regression_percent: regression_percent(
                input.baseline.score.total,
                input.candidate.score.total,
            ),
        };
    }

    if input.candidate.score.over_5ms > input.baseline.score.over_5ms {
        return ExperimentResult::Regressed {
            regression_percent: regression_percent(
                input.baseline.score.total,
                input.candidate.score.total,
            ),
        };
    }

    if input.candidate.score.frame_p99_ms
        > input.baseline.score.frame_p99_ms + policy.regressed_frame_p99_slack_ms
    {
        return ExperimentResult::Regressed {
            regression_percent: regression_percent(
                input.baseline.score.total,
                input.candidate.score.total,
            ),
        };
    }

    if input.data_quality.is_acceptable_for_improvement()
        && input.candidate.score.total as f64
            <= input.baseline.score.total as f64 * policy.min_improvement_ratio
        && input.candidate.score.over_5ms <= input.baseline.score.over_5ms
        && input.candidate.score.frame_p99_ms
            <= input.baseline.score.frame_p99_ms + policy.improved_frame_p99_slack_ms
    {
        return ExperimentResult::Improved {
            improvement_percent: improvement_percent(
                input.baseline.score.total,
                input.candidate.score.total,
            ),
        };
    }

    ExperimentResult::Inconclusive {
        reason: "candidate did not meet conservative improve or regress thresholds".to_owned(),
    }
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

fn improvement_percent(baseline_total: u64, candidate_total: u64) -> f64 {
    if baseline_total == 0 || candidate_total >= baseline_total {
        0.0
    } else {
        ((baseline_total - candidate_total) as f64 / baseline_total as f64) * 100.0
    }
}

fn regression_percent(baseline_total: u64, candidate_total: u64) -> f64 {
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
    fn improved_total_with_frame_p99_more_than_one_ms_worse_is_inconclusive() {
        let baseline = window(1_000, 10, 12.0);
        let candidate = window(800, 10, 13.5);

        let result = compare_experiment(input(
            &baseline,
            &candidate,
            ExperimentDataQuality::High,
            false,
        ));

        assert!(matches!(result, ExperimentResult::Inconclusive { .. }));
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
}
