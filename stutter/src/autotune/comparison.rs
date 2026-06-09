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
    pub max_over_5ms_regression_per_1k_samples: f64,
}

pub(crate) const DEFAULT_SCORE_COMPARISON_CONFIG: ScoreComparisonConfig = ScoreComparisonConfig {
    min_improvement_percent: 12.5,
    max_regression_percent: 7.5,
    max_frame_p99_regression_ms: 2.0,
    max_over_5ms_regression_per_1k_samples: 0.0,
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
    #[cfg(test)]
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

#[derive(Clone, Copy, Debug, PartialEq)]
struct ComparableScore {
    score_per_sample: f64,
    over_5ms_per_sample: f64,
    raw_total: u64,
    raw_over_5ms: u64,
}

#[derive(Clone, Debug)]
pub struct ExperimentComparisonPolicy {
    pub min_improvement_ratio: f64,
    pub max_regression_ratio: f64,
    pub regressed_frame_p99_slack_ms: f64,
}

impl Default for ExperimentComparisonPolicy {
    fn default() -> Self {
        Self {
            min_improvement_ratio: 1.0
                - (DEFAULT_SCORE_COMPARISON_CONFIG.min_improvement_percent / 100.0),
            max_regression_ratio: 1.0
                + (DEFAULT_SCORE_COMPARISON_CONFIG.max_regression_percent / 100.0),
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
            max_over_5ms_regression_per_1k_samples: DEFAULT_SCORE_COMPARISON_CONFIG
                .max_over_5ms_regression_per_1k_samples,
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

    let baseline = match comparable_score("baseline", input.baseline) {
        Ok(score) => score,
        Err(reason) => return ExperimentResult::Invalid { reason },
    };
    let candidate = match comparable_score("candidate", input.candidate) {
        Ok(score) => score,
        Err(reason) => return ExperimentResult::Invalid { reason },
    };

    let regression_percent =
        regression_percent_f64(baseline.score_per_sample, candidate.score_per_sample);

    if input.target_disappeared {
        return ExperimentResult::Regressed { regression_percent };
    }

    if input.data_quality.is_low() {
        return ExperimentResult::Regressed { regression_percent };
    }

    if regression_percent > config.max_regression_percent {
        return ExperimentResult::Regressed { regression_percent };
    }

    let baseline_over_5ms_per_1k = baseline.over_5ms_per_sample * 1_000.0;
    let candidate_over_5ms_per_1k = candidate.over_5ms_per_sample * 1_000.0;

    if candidate_over_5ms_per_1k
        > baseline_over_5ms_per_1k + config.max_over_5ms_regression_per_1k_samples
    {
        return ExperimentResult::Regressed { regression_percent };
    }

    if input.candidate.score.frame_p99_ms
        > input.baseline.score.frame_p99_ms + config.max_frame_p99_regression_ms
    {
        return ExperimentResult::Regressed { regression_percent };
    }

    let improvement_percent =
        improvement_percent_f64(baseline.score_per_sample, candidate.score_per_sample);

    if input.data_quality.is_acceptable_for_improvement()
        && improvement_percent >= config.min_improvement_percent
        && candidate.over_5ms_per_sample <= baseline.over_5ms_per_sample
        && input.candidate.score.frame_p99_ms
            <= input.baseline.score.frame_p99_ms + config.max_frame_p99_regression_ms
    {
        return ExperimentResult::Improved {
            improvement_percent,
        };
    }

    ExperimentResult::Inconclusive {
        reason: format!(
            "candidate did not meet conservative thresholds: improvement_percent={:.2} min_improvement_percent={:.2} regression_percent={:.2} max_regression_percent={:.2} baseline_score_per_sample={:.6} candidate_score_per_sample={:.6} baseline_over_5ms_per_1k_samples={:.6} candidate_over_5ms_per_1k_samples={:.6} baseline_raw_total={} candidate_raw_total={} baseline_raw_over_5ms={} candidate_raw_over_5ms={} baseline_scored_samples={} candidate_scored_samples={}",
            improvement_percent,
            config.min_improvement_percent,
            regression_percent,
            config.max_regression_percent,
            baseline.score_per_sample,
            candidate.score_per_sample,
            baseline_over_5ms_per_1k,
            candidate_over_5ms_per_1k,
            baseline.raw_total,
            candidate.raw_total,
            baseline.raw_over_5ms,
            candidate.raw_over_5ms,
            input.baseline.scored_samples,
            input.candidate.scored_samples
        ),
    }
}

fn comparable_score(label: &str, score: &WindowScore) -> Result<ComparableScore, String> {
    let score_per_sample = score
        .score_per_sample()
        .ok_or_else(|| format!("{label} score_per_sample is missing"))?;
    let over_5ms_per_sample = score
        .over_5ms_per_sample()
        .ok_or_else(|| format!("{label} over_5ms_per_sample is missing"))?;

    if !score_per_sample.is_finite() {
        return Err(format!("{label} score_per_sample is not finite"));
    }

    if !over_5ms_per_sample.is_finite() {
        return Err(format!("{label} over_5ms_per_sample is not finite"));
    }

    Ok(ComparableScore {
        score_per_sample,
        over_5ms_per_sample,
        raw_total: score.score.total,
        raw_over_5ms: score.score.over_5ms,
    })
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

    if !config.max_over_5ms_regression_per_1k_samples.is_finite()
        || config.max_over_5ms_regression_per_1k_samples < 0.0
    {
        return Err(format!(
            "max_over_5ms_regression_per_1k_samples must be finite and non-negative: {}",
            config.max_over_5ms_regression_per_1k_samples
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

fn improvement_percent_f64(baseline: f64, candidate: f64) -> f64 {
    if baseline <= 0.0 || candidate >= baseline {
        0.0
    } else {
        ((baseline - candidate) / baseline) * 100.0
    }
}

pub(crate) fn regression_percent_f64(baseline: f64, candidate: f64) -> f64 {
    if candidate <= baseline {
        0.0
    } else if baseline <= 0.0 {
        100.0
    } else {
        ((candidate - baseline) / baseline) * 100.0
    }
}

#[cfg(test)]
mod tests;
