//! An OpenTuner-style technique ensemble and a held-out ranking-accuracy report (package W15).
//!
//! The ensemble picks, trial by trial, which member technique proposes next, crediting a technique
//! when its last proposal beat every earlier trial (UCB1 over that 0/1 reward). It adds no
//! proposal of its own: each candidate still comes from a member, which only sees the legal set,
//! so the ensemble can never leave the authorized finite space. The ranking report compares what a
//! training dataset predicts with what an untouched holdout shows. Both are offline: production
//! ranking is unchanged.
use super::{Candidate, Dataset, SearchPolicy, Trial};

pub struct Ensemble {
    techniques: Vec<Box<dyn SearchPolicy>>,
    reward: Vec<f64>,
    pulls: Vec<usize>,
    /// The technique whose proposal is awaiting its result, and the history length before it.
    pending: Option<(usize, usize)>,
}

impl Ensemble {
    pub fn new(techniques: Vec<Box<dyn SearchPolicy>>) -> Self {
        let count = techniques.len();
        Self {
            techniques,
            reward: vec![0.0; count],
            pulls: vec![0; count],
            pending: None,
        }
    }

    /// Credits the pending technique when the newest trial beat every earlier one.
    fn settle(&mut self, history: &[Trial]) {
        let Some((technique, before)) = self.pending.take() else {
            return;
        };
        if history.len() <= before {
            return;
        }
        let newest = history[history.len() - 1].improvement;
        let best_before = history[..history.len() - 1]
            .iter()
            .map(|trial| trial.improvement)
            .fold(f64::NEG_INFINITY, f64::max);
        if newest > best_before {
            self.reward[technique] += 1.0;
        }
    }

    fn choose(&self) -> Option<usize> {
        if let Some(untried) = self.pulls.iter().position(|pulls| *pulls == 0) {
            return Some(untried);
        }
        let total: usize = self.pulls.iter().sum();
        (0..self.techniques.len()).max_by(|a, b| {
            let score = |i: usize| {
                self.reward[i] / self.pulls[i] as f64
                    + (2.0 * (total as f64).ln() / self.pulls[i] as f64).sqrt()
            };
            score(*a).total_cmp(&score(*b))
        })
    }

    /// How often each member technique was asked, in construction order.
    pub fn pulls(&self) -> &[usize] {
        &self.pulls
    }
}

impl SearchPolicy for Ensemble {
    fn name(&self) -> &'static str {
        "ensemble"
    }

    fn propose(&mut self, legal: &[Candidate], history: &[Trial]) -> Option<Candidate> {
        self.settle(history);
        let technique = self.choose()?;
        let candidate = self.techniques[technique].propose(legal, history)?;
        self.pulls[technique] += 1;
        self.pending = Some((technique, history.len()));
        Some(candidate)
    }
}

/// How well a training dataset's ranking predicts an untouched holdout's.
#[derive(Clone, Debug, PartialEq)]
pub struct HeldOutRanking {
    /// Candidates present with at least one block in both datasets.
    pub candidates: usize,
    /// The training favourite is also the holdout favourite.
    pub top1_agrees: bool,
    /// The holdout mean of the training favourite minus the holdout's best mean (never above 0).
    pub holdout_regret: f64,
    /// Spearman rank correlation of the two mean orderings (ordinal ranks; ties broken by order).
    pub spearman: f64,
}

fn mean(blocks: &[f64]) -> Option<f64> {
    (!blocks.is_empty()).then(|| blocks.iter().sum::<f64>() / blocks.len() as f64)
}

fn ranks(values: &[f64]) -> Vec<f64> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|a, b| values[*b].total_cmp(&values[*a]));
    let mut ranks = vec![0.0; values.len()];
    for (rank, index) in order.into_iter().enumerate() {
        ranks[index] = rank as f64;
    }
    ranks
}

/// Compares the two datasets over their common candidates. Fewer than three is not a ranking.
pub fn held_out_ranking(train: &Dataset, holdout: &Dataset) -> Option<HeldOutRanking> {
    let common: Vec<(f64, f64)> = train
        .iter()
        .filter_map(|(candidate, blocks)| Some((mean(blocks)?, mean(holdout.get(candidate)?)?)))
        .filter(|(a, b)| a.is_finite() && b.is_finite())
        .collect();
    if common.len() < 3 {
        return None;
    }
    let (predicted, observed): (Vec<f64>, Vec<f64>) = common.into_iter().unzip();
    let favourite = ranks(&predicted).iter().position(|rank| *rank == 0.0)?;
    let best = observed.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let (rank_a, rank_b) = (ranks(&predicted), ranks(&observed));
    let n = predicted.len() as f64;
    let squared: f64 = rank_a
        .iter()
        .zip(&rank_b)
        .map(|(a, b)| (a - b).powi(2))
        .sum();
    Some(HeldOutRanking {
        candidates: predicted.len(),
        top1_agrees: observed[favourite] == best,
        holdout_regret: observed[favourite] - best,
        spearman: 1.0 - 6.0 * squared / (n * (n * n - 1.0)),
    })
}

#[cfg(test)]
#[path = "ensemble_tests.rs"]
mod tests;
