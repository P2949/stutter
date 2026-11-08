//! Offline measurement import and a bounded Bayesian-style policy (package W15).
//!
//! `import_measurements` reads trial records exported by an external tuner (Kernel Tuner,
//! OtterTune or a hand-made file) into a [`Dataset`]. Nothing in the file is executed. A trial for
//! a candidate outside the legal space is dropped, a non-finite improvement is an error, and a
//! candidate with *any* trial that failed its correctness check is removed entirely: a fast wrong
//! answer is never a benefit. `OptimisticBayes` keeps a per-candidate Gaussian posterior with a
//! weak prior and proposes the candidate with the best optimistic bound. It is a plain, auditable
//! formula, sees only the legal set and history, and is never deployed to production.
use std::collections::BTreeSet;

use anyhow::{Context, bail, ensure};
use serde::{Deserialize, Serialize};

use super::{Candidate, ConfigSpace, Dataset, SearchPolicy, Trial};

/// Most trial records read from one file.
pub const MAX_TRIALS: usize = 100_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Record {
    candidate: Vec<String>,
    improvement: f64,
    /// Whether the run passed the workload's correctness check. Absent means unverified: refused.
    correct: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Export {
    source: String,
    trials: Vec<Record>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    pub source: String,
    pub dataset: Dataset,
    /// Trials for candidates outside the legal space.
    pub outside_space: usize,
    /// Candidates removed because at least one of their trials was incorrect.
    pub incorrect_candidates: usize,
    /// Trials with no correctness verdict: they are not used.
    pub unverified: usize,
}

pub fn import_measurements(json: &[u8], space: &ConfigSpace) -> anyhow::Result<Imported> {
    let export: Export = serde_json::from_slice(json).context("not a measurement export")?;
    ensure!(
        !export.source.trim().is_empty(),
        "the export names no source"
    );
    ensure!(export.trials.len() <= MAX_TRIALS, "too many trials");
    let legal: BTreeSet<Candidate> = space
        .enumerate()
        .map_err(|error| anyhow::anyhow!("the space is invalid: {error:?}"))?
        .into_iter()
        .collect();
    let mut dataset = Dataset::new();
    let mut incorrect: BTreeSet<Candidate> = BTreeSet::new();
    let (mut outside_space, mut unverified) = (0, 0);
    for record in export.trials {
        if !record.improvement.is_finite() {
            bail!("a trial has a non-finite improvement");
        }
        if !legal.contains(&record.candidate) {
            outside_space += 1;
            continue;
        }
        match record.correct {
            None => unverified += 1,
            Some(false) => {
                incorrect.insert(record.candidate);
            }
            Some(true) => dataset
                .entry(record.candidate)
                .or_default()
                .push(record.improvement),
        }
    }
    for candidate in &incorrect {
        dataset.remove(candidate);
    }
    ensure!(!dataset.is_empty(), "no verified correct trial remains");
    Ok(Imported {
        source: export.source,
        dataset,
        outside_space,
        incorrect_candidates: incorrect.len(),
        unverified,
    })
}

/// Optimism in the face of uncertainty with a Gaussian posterior per candidate.
pub struct OptimisticBayes {
    /// Weight of the zero-mean prior, in pseudo-observations.
    prior_strength: f64,
    /// Width of the optimism bonus, in posterior standard deviations.
    optimism: f64,
}

impl OptimisticBayes {
    pub fn new(prior_strength: f64, optimism: f64) -> Self {
        Self {
            prior_strength: prior_strength.max(0.1),
            optimism: optimism.max(0.0),
        }
    }

    fn bound(&self, blocks: &[f64]) -> f64 {
        let n = blocks.len() as f64;
        let mean = blocks.iter().sum::<f64>() / n.max(1.0);
        let spread = if blocks.len() > 1 {
            (blocks.iter().map(|b| (b - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt()
        } else {
            // One block says nothing about spread: assume the observation's own size.
            mean.abs().max(1.0)
        };
        let posterior_mean = n * mean / (n + self.prior_strength);
        posterior_mean + self.optimism * spread / (n + self.prior_strength).sqrt()
    }
}

impl SearchPolicy for OptimisticBayes {
    fn name(&self) -> &'static str {
        "optimistic_bayes"
    }

    fn propose(&mut self, legal: &[Candidate], history: &[Trial]) -> Option<Candidate> {
        legal
            .iter()
            .max_by(|a, b| {
                let blocks = |c: &Candidate| -> Vec<f64> {
                    history
                        .iter()
                        .filter(|t| &t.candidate == c)
                        .map(|t| t.improvement)
                        .collect()
                };
                let score = |c: &Candidate| {
                    let blocks = blocks(c);
                    // An untried candidate always gets one look first.
                    if blocks.is_empty() {
                        f64::INFINITY
                    } else {
                        self.bound(&blocks)
                    }
                };
                score(a).total_cmp(&score(b)).then_with(|| b.cmp(a))
            })
            .cloned()
    }
}

#[cfg(test)]
#[path = "learning_tests.rs"]
mod tests;
