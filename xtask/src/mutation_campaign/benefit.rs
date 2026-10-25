//! Matched-pair benefit analysis. Pure: it decides from recorded pair summaries and never runs a
//! workload, so a threshold sweep can be repeated from the stored data. Calibration and holdout
//! are a stable persisted split of the pairs; the holdout is evaluated once with thresholds
//! frozen from the calibration part.
use serde::{Deserialize, Serialize};
use stutter::api::autotune::{
    DEFAULT_MAX_REGRESSION_PERCENT, DEFAULT_MIN_IMPROVEMENT_PERCENT, ExperimentResult,
    MINIMUM_RUNNABLE_SAMPLES, RunnableSummary, RunnableThresholds, decide_runnable,
};

use super::manifest::ThresholdGrid;
use crate::evidence_bundle::sha256::sha256_hex;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scenario {
    Beneficial,
    Neutral,
    Regression,
}

impl Scenario {
    pub const ALL: [Self; 3] = [Self::Beneficial, Self::Neutral, Self::Regression];
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Order {
    BaselineFirst,
    CandidateFirst,
}

/// One matched baseline/candidate pair of a scenario, as measured by the real control path.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PairResult {
    pub scenario: Scenario,
    pub pair: usize,
    pub order: Order,
    pub baseline: RunnableSummaryRecord,
    pub candidate: RunnableSummaryRecord,
    /// The affinity action was physically observed on the target (the mechanism).
    pub placement_observed: bool,
    /// Frame pacing, thermal and power guardrails did not regress in the pair.
    pub guardrails_ok: bool,
    /// What the live manager decided at the product thresholds (audit only).
    pub manager_kept: bool,
}

/// `RunnableSummary` without the library type in the serialized form.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunnableSummaryRecord {
    pub p99_ns: u64,
    pub samples: u64,
    pub over_5ms: u64,
}

impl From<RunnableSummaryRecord> for RunnableSummary {
    fn from(record: RunnableSummaryRecord) -> Self {
        Self {
            p99_ns: record.p99_ns,
            samples: record.samples,
            over_5ms: record.over_5ms,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Part {
    Calibration,
    Holdout,
}

/// Stable assignment: the same scenario/pair always lands in the same part, whatever else is
/// collected, and the calibration share is `calibration_parts : holdout_parts`.
pub fn part_of(
    scenario: Scenario,
    pair: usize,
    calibration_parts: usize,
    holdout_parts: usize,
) -> Part {
    let digest = sha256_hex(format!("{scenario:?}|{pair}").as_bytes());
    let bucket = u64::from_str_radix(&digest[..8], 16).unwrap_or(0);
    let total = (calibration_parts + holdout_parts) as u64;
    if bucket % total < calibration_parts as u64 {
        Part::Calibration
    } else {
        Part::Holdout
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Tally {
    pub beneficial: usize,
    pub beneficial_kept: usize,
    pub neutral: usize,
    pub regression: usize,
    pub regression_reverted: usize,
    /// A neutral or regression pair that would have been kept.
    pub false_keeps: usize,
    /// Pairs whose mechanism or guardrails were not satisfied; excluded from keeps.
    pub guarded_out: usize,
}

impl Tally {
    pub fn beneficial_keep_rate(&self) -> f64 {
        if self.beneficial == 0 {
            0.0
        } else {
            self.beneficial_kept as f64 / self.beneficial as f64
        }
    }
}

fn result_for(pair: &PairResult, thresholds: RunnableThresholds) -> ExperimentResult {
    decide_runnable(pair.baseline.into(), pair.candidate.into(), thresholds)
}

/// Decisions of `thresholds` over `pairs`. A keep needs the primary outcome improved AND the
/// mechanism observed AND guardrails intact; the mechanism can only withhold a keep.
pub fn tally<'a>(
    pairs: impl IntoIterator<Item = &'a PairResult>,
    thresholds: RunnableThresholds,
) -> Tally {
    let mut tally = Tally::default();
    for pair in pairs {
        let result = result_for(pair, thresholds);
        let kept = matches!(result, ExperimentResult::Improved { .. })
            && pair.placement_observed
            && pair.guardrails_ok;
        if matches!(result, ExperimentResult::Improved { .. })
            && !(pair.placement_observed && pair.guardrails_ok)
        {
            tally.guarded_out += 1;
        }
        match pair.scenario {
            Scenario::Beneficial => {
                tally.beneficial += 1;
                tally.beneficial_kept += usize::from(kept);
            }
            Scenario::Neutral => {
                tally.neutral += 1;
                tally.false_keeps += usize::from(kept);
            }
            Scenario::Regression => {
                tally.regression += 1;
                tally.false_keeps += usize::from(kept);
                tally.regression_reverted +=
                    usize::from(matches!(result, ExperimentResult::Regressed { .. }));
            }
        }
    }
    tally
}

pub fn default_thresholds() -> RunnableThresholds {
    RunnableThresholds::default()
}

/// Every grid point as a threshold set.
pub fn grid_points(grid: &ThresholdGrid) -> Vec<RunnableThresholds> {
    let mut points = Vec::new();
    for improvement in &grid.min_improvement_percent {
        for regression in &grid.max_regression_percent {
            for samples in &grid.min_samples {
                points.push(RunnableThresholds {
                    min_improvement_percent: *improvement,
                    max_regression_percent: *regression,
                    min_samples: *samples,
                });
            }
        }
    }
    points
}

/// How far a set is from the product defaults, in relative terms; the tie-breaker.
fn distance_from_default(thresholds: RunnableThresholds) -> f64 {
    (thresholds.min_improvement_percent - DEFAULT_MIN_IMPROVEMENT_PERCENT).abs()
        / DEFAULT_MIN_IMPROVEMENT_PERCENT
        + (thresholds.max_regression_percent - DEFAULT_MAX_REGRESSION_PERCENT).abs()
            / DEFAULT_MAX_REGRESSION_PERCENT
        + (thresholds.min_samples as f64 - MINIMUM_RUNNABLE_SAMPLES as f64).abs()
            / MINIMUM_RUNNABLE_SAMPLES as f64
}

/// The grid point to freeze: priority (1) zero false keeps, (2) most beneficial keeps, (3) most
/// regressions reverted, (4) smallest change from the defaults. `None` when no grid point has
/// zero false keeps (the operation cannot be calibrated).
pub fn select_thresholds(
    calibration: &[&PairResult],
    grid: &ThresholdGrid,
) -> Option<(RunnableThresholds, Tally)> {
    grid_points(grid)
        .into_iter()
        .map(|point| (point, tally(calibration.iter().copied(), point)))
        .filter(|(_, tally)| tally.false_keeps == 0)
        .min_by(|(a, ta), (b, tb)| {
            tb.beneficial_kept
                .cmp(&ta.beneficial_kept)
                .then(tb.regression_reverted.cmp(&ta.regression_reverted))
                .then(
                    distance_from_default(*a)
                        .partial_cmp(&distance_from_default(*b))
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
        })
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HoldoutVerdict {
    pub thresholds: RunnableThresholdsRecord,
    pub tally: Tally,
    pub zero_false_keeps: bool,
    pub beneficial_keep_rate: f64,
    pub passed: bool,
}

/// `RunnableThresholds` for JSON (the library type is not serializable).
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct RunnableThresholdsRecord {
    pub min_improvement_percent: f64,
    pub max_regression_percent: f64,
    pub min_samples: u64,
}

impl From<RunnableThresholds> for RunnableThresholdsRecord {
    fn from(thresholds: RunnableThresholds) -> Self {
        Self {
            min_improvement_percent: thresholds.min_improvement_percent,
            max_regression_percent: thresholds.max_regression_percent,
            min_samples: thresholds.min_samples,
        }
    }
}

/// The single holdout evaluation with frozen thresholds: zero false keeps and the required
/// beneficial keep rate.
pub fn evaluate_holdout(
    holdout: &[&PairResult],
    frozen: RunnableThresholds,
    min_beneficial_keep_rate: f64,
) -> HoldoutVerdict {
    let tally = tally(holdout.iter().copied(), frozen);
    let rate = tally.beneficial_keep_rate();
    let zero_false_keeps = tally.false_keeps == 0;
    HoldoutVerdict {
        thresholds: frozen.into(),
        tally,
        zero_false_keeps,
        beneficial_keep_rate: rate,
        passed: zero_false_keeps && tally.beneficial > 0 && rate >= min_beneficial_keep_rate,
    }
}

#[cfg(test)]
mod tests;
