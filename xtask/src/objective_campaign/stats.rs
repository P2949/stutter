//! The pure part of an objective campaign: pairing runs, applying the controller's own decision
//! rule at a threshold, and the pre-registered threshold selection.
use serde::{Deserialize, Serialize};
use stutter::api::autotune::{CompileChange, classify_compile_change};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    Clean,
    Interfered,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Run {
    pub block: u32,
    pub position: u32,
    pub condition: Condition,
    pub elapsed_ns: u64,
    pub execution_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairKind {
    /// Two clean runs: nothing changed, so no keep is correct.
    Neutral,
    /// Interference removed: the candidate truly is faster.
    Beneficial,
    /// Interference added: the candidate truly is slower.
    Regression,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pair {
    pub block: u32,
    pub kind: PairKind,
    pub baseline_ns: u64,
    pub candidate_ns: u64,
}

impl Pair {
    /// Signed change of the candidate against the baseline, in percent (negative is faster).
    pub fn change_percent(&self) -> f64 {
        (self.candidate_ns as f64 - self.baseline_ns as f64) / self.baseline_ns as f64 * 100.0
    }
}

/// The adjacent pairs inside each block, classified by what actually differed between the runs.
/// Runs in different blocks are never paired.
pub fn pairs(runs: &[Run]) -> Vec<Pair> {
    let mut ordered: Vec<&Run> = runs.iter().collect();
    ordered.sort_by_key(|run| (run.block, run.position));
    ordered
        .windows(2)
        .filter(|window| window[0].block == window[1].block)
        .filter_map(|window| {
            let kind = match (window[0].condition, window[1].condition) {
                (Condition::Clean, Condition::Clean) => PairKind::Neutral,
                (Condition::Interfered, Condition::Clean) => PairKind::Beneficial,
                (Condition::Clean, Condition::Interfered) => PairKind::Regression,
                (Condition::Interfered, Condition::Interfered) => return None,
            };
            Some(Pair {
                block: window[0].block,
                kind,
                baseline_ns: window[0].elapsed_ns,
                candidate_ns: window[1].elapsed_ns,
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tally {
    pub neutral: u32,
    pub regression: u32,
    pub beneficial: u32,
    pub beneficial_kept: u32,
    /// An improvement declared where none exists (a neutral or regression pair).
    pub false_keeps: u32,
}

impl Tally {
    pub fn keep_rate(&self) -> f64 {
        if self.beneficial == 0 {
            0.0
        } else {
            f64::from(self.beneficial_kept) / f64::from(self.beneficial)
        }
    }
}

/// The controller's verdict on every pair at `threshold_percent`.
pub fn tally(pairs: &[Pair], threshold_percent: f64) -> Tally {
    let mut tally = Tally::default();
    for pair in pairs {
        let improved = matches!(
            classify_compile_change(pair.baseline_ns, pair.candidate_ns, threshold_percent),
            CompileChange::Improved(_)
        );
        match pair.kind {
            PairKind::Neutral => {
                tally.neutral += 1;
                tally.false_keeps += u32::from(improved);
            }
            PairKind::Regression => {
                tally.regression += 1;
                tally.false_keeps += u32::from(improved);
            }
            PairKind::Beneficial => {
                tally.beneficial += 1;
                tally.beneficial_kept += u32::from(improved);
            }
        }
    }
    tally
}

#[derive(Clone, Copy, Debug)]
pub struct Rule {
    pub noise_margin: f64,
    pub min_beneficial_keep_rate: f64,
    pub max_false_keeps: u32,
}

/// The largest absolute change between two clean runs: the noise the threshold must clear.
pub fn neutral_noise_percent(pairs: &[Pair]) -> f64 {
    pairs
        .iter()
        .filter(|pair| pair.kind == PairKind::Neutral)
        .map(|pair| pair.change_percent().abs())
        .fold(0.0, f64::max)
}

/// The smallest candidate threshold that satisfies the pre-registered rule on calibration pairs,
/// with its tally; `None` when no candidate does (the objective stays unproven).
pub fn select_threshold(calibration: &[Pair], grid: &[f64], rule: &Rule) -> Option<(f64, Tally)> {
    let noise = neutral_noise_percent(calibration);
    let mut candidates: Vec<f64> = grid.to_vec();
    candidates.sort_by(f64::total_cmp);
    candidates.into_iter().find_map(|threshold| {
        let tally = tally(calibration, threshold);
        (tally.false_keeps <= rule.max_false_keeps
            && threshold >= rule.noise_margin * noise
            && tally.keep_rate() >= rule.min_beneficial_keep_rate)
            .then_some((threshold, tally))
    })
}

/// Whether an untouched holdout, judged at the frozen threshold, meets the acceptance rule.
pub fn holdout_passes(tally: &Tally, rule: &Rule) -> bool {
    tally.false_keeps <= rule.max_false_keeps
        && tally.beneficial > 0
        && tally.neutral > 0
        && tally.regression > 0
        && tally.keep_rate() >= rule.min_beneficial_keep_rate
}
