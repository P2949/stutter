//! Offline candidate-search policies over a finite, legal configuration space (package W15).
//!
//! The space, its constraints and the trial budget are fixed and validated before any search, and a
//! policy can only ever propose a member of the enumerated legal set: an optimizer cannot invent an
//! operation, a value or a path. The policies are plain and auditable (a grid, a seeded random
//! order, paired elimination); there is no learned model. They are compared offline on historical
//! per-block outcomes by regret and time to benefit, and production keeps its conservative ranking
//! unless a comparison on real data justifies more.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Most candidates a space may enumerate; a larger one is refused rather than searched.
pub const MAX_SPACE: usize = 10_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dimension {
    pub name: String,
    pub values: Vec<String>,
}

/// One concrete choice per dimension, in dimension order.
pub type Candidate = Vec<String>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigSpace {
    pub dimensions: Vec<Dimension>,
    /// Combinations that are not legal, as `(dimension, value)` sets that must not all hold.
    pub forbidden: Vec<Vec<(String, String)>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpaceError {
    Empty,
    EmptyDimension,
    DuplicateDimension,
    DuplicateValue,
    UnknownInConstraint,
    TooLarge,
    NothingLegal,
}

impl ConfigSpace {
    /// Every legal candidate in a fixed order. Validates the space first.
    pub fn enumerate(&self) -> Result<Vec<Candidate>, SpaceError> {
        if self.dimensions.is_empty() {
            return Err(SpaceError::Empty);
        }
        let mut names = std::collections::BTreeSet::new();
        let mut size: usize = 1;
        for dimension in &self.dimensions {
            if dimension.values.is_empty() {
                return Err(SpaceError::EmptyDimension);
            }
            if !names.insert(&dimension.name) {
                return Err(SpaceError::DuplicateDimension);
            }
            let values: std::collections::BTreeSet<&String> = dimension.values.iter().collect();
            if values.len() != dimension.values.len() {
                return Err(SpaceError::DuplicateValue);
            }
            size = size
                .checked_mul(dimension.values.len())
                .ok_or(SpaceError::TooLarge)?;
            if size > MAX_SPACE {
                return Err(SpaceError::TooLarge);
            }
        }
        for rule in &self.forbidden {
            for (name, value) in rule {
                let known = self
                    .dimensions
                    .iter()
                    .any(|d| &d.name == name && d.values.contains(value));
                if !known {
                    return Err(SpaceError::UnknownInConstraint);
                }
            }
        }
        let mut all: Vec<Candidate> = vec![Vec::new()];
        for dimension in &self.dimensions {
            all = all
                .into_iter()
                .flat_map(|prefix| {
                    dimension.values.iter().map(move |value| {
                        let mut next = prefix.clone();
                        next.push(value.clone());
                        next
                    })
                })
                .collect();
        }
        all.retain(|candidate| !self.is_forbidden(candidate));
        if all.is_empty() {
            Err(SpaceError::NothingLegal)
        } else {
            Ok(all)
        }
    }

    fn is_forbidden(&self, candidate: &Candidate) -> bool {
        self.forbidden.iter().any(|rule| {
            rule.iter().all(|(name, value)| {
                self.dimensions
                    .iter()
                    .position(|d| &d.name == name)
                    .is_some_and(|index| &candidate[index] == value)
            })
        })
    }
}

/// One finished trial: the candidate and the improvement of one independent block.
#[derive(Clone, Debug, PartialEq)]
pub struct Trial {
    pub candidate: Candidate,
    pub improvement: f64,
}

/// A search policy. It sees only the legal set and past trials.
pub trait SearchPolicy {
    fn name(&self) -> &'static str;
    fn propose(&mut self, legal: &[Candidate], history: &[Trial]) -> Option<Candidate>;
}

/// Visits every candidate once in order, then repeats.
#[derive(Default)]
pub struct GridSearch {
    next: usize,
}

impl SearchPolicy for GridSearch {
    fn name(&self) -> &'static str {
        "grid"
    }

    fn propose(&mut self, legal: &[Candidate], _history: &[Trial]) -> Option<Candidate> {
        let candidate = legal.get(self.next % legal.len().max(1))?.clone();
        self.next += 1;
        Some(candidate)
    }
}

/// A seeded random order over the legal set, with every candidate tried once per round.
pub struct SeededRandom {
    state: u64,
    queue: Vec<usize>,
}

impl SeededRandom {
    pub fn new(seed: u64) -> Self {
        Self {
            state: seed.max(1),
            queue: Vec::new(),
        }
    }

    fn next(&mut self) -> u64 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        self.state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

impl SearchPolicy for SeededRandom {
    fn name(&self) -> &'static str {
        "seeded_random"
    }

    fn propose(&mut self, legal: &[Candidate], _history: &[Trial]) -> Option<Candidate> {
        if legal.is_empty() {
            return None;
        }
        if self.queue.is_empty() {
            let mut order: Vec<usize> = (0..legal.len()).collect();
            for index in (1..order.len()).rev() {
                let pick = (self.next() % (index as u64 + 1)) as usize;
                order.swap(index, pick);
            }
            self.queue = order;
        }
        self.queue.pop().map(|index| legal[index].clone())
    }
}

/// Successive halving: every survivor gets `per_round` blocks, then the worse half is dropped.
pub struct PairedElimination {
    per_round: usize,
}

impl PairedElimination {
    pub fn new(per_round: usize) -> Self {
        Self {
            per_round: per_round.max(1),
        }
    }

    fn means(history: &[Trial]) -> BTreeMap<&Candidate, (f64, usize)> {
        let mut sums: BTreeMap<&Candidate, (f64, usize)> = BTreeMap::new();
        for trial in history {
            let entry = sums.entry(&trial.candidate).or_insert((0.0, 0));
            entry.0 += trial.improvement;
            entry.1 += 1;
        }
        sums
    }
}

impl SearchPolicy for PairedElimination {
    fn name(&self) -> &'static str {
        "paired_elimination"
    }

    fn propose(&mut self, legal: &[Candidate], history: &[Trial]) -> Option<Candidate> {
        let means = Self::means(history);
        let mut survivors: Vec<&Candidate> = legal.iter().collect();
        let mut round_blocks = self.per_round;
        // Walk the rounds the history already completed, halving the survivors each time.
        loop {
            let counts_ok = survivors
                .iter()
                .all(|c| means.get(c).is_some_and(|(_, n)| *n >= round_blocks));
            if !counts_ok || survivors.len() <= 1 {
                break;
            }
            survivors.sort_by(|a, b| {
                let mean = |c: &&Candidate| means[c].0 / means[c].1 as f64;
                mean(b).total_cmp(&mean(a))
            });
            survivors.truncate(survivors.len().div_ceil(2));
            round_blocks += self.per_round;
        }
        survivors
            .into_iter()
            .find(|c| means.get(c).is_none_or(|(_, n)| *n < round_blocks))
            .or_else(|| legal.first())
            .cloned()
    }
}

/// Historical per-block improvements per candidate (positive is better).
pub type Dataset = BTreeMap<Candidate, Vec<f64>>;

#[derive(Clone, Debug, PartialEq)]
pub struct SearchReport {
    pub policy: &'static str,
    pub trials_used: usize,
    pub chosen: Option<Candidate>,
    /// The best true mean minus the chosen candidate's true mean (zero when it chose the best).
    pub regret: f64,
    /// Trials until the eventually chosen candidate was first tried: how soon it was found.
    pub trials_to_first_try_of_chosen: Option<usize>,
}

/// Replays a policy against a historical dataset with a fixed budget. Block outcomes are consumed
/// in order per candidate, so a replay never invents a measurement that was not recorded.
pub fn evaluate_offline(
    policy: &mut dyn SearchPolicy,
    space: &ConfigSpace,
    dataset: &Dataset,
    budget: usize,
) -> Result<SearchReport, SpaceError> {
    let legal = space.enumerate()?;
    let mut cursor: BTreeMap<&Candidate, usize> = BTreeMap::new();
    let mut history: Vec<Trial> = Vec::new();
    for _ in 0..budget {
        let Some(candidate) = policy.propose(&legal, &history) else {
            break;
        };
        // Only members of the legal set may ever be run; anything else ends the replay.
        let Some(blocks) = legal
            .iter()
            .find(|c| **c == candidate)
            .and_then(|c| dataset.get(c))
        else {
            break;
        };
        let index = cursor
            .entry(legal.iter().find(|c| **c == candidate).unwrap_or(&legal[0]))
            .or_default();
        let Some(&improvement) = blocks.get(*index) else {
            break;
        };
        *index += 1;
        history.push(Trial {
            candidate,
            improvement,
        });
    }
    let true_mean = |candidate: &Candidate| {
        dataset
            .get(candidate)
            .filter(|v| !v.is_empty())
            .map(|v| v.iter().sum::<f64>() / v.len() as f64)
    };
    let best = legal
        .iter()
        .filter_map(true_mean)
        .fold(f64::NEG_INFINITY, f64::max);
    let means = PairedElimination::means(&history);
    let chosen = means
        .iter()
        .max_by(|a, b| (a.1.0 / a.1.1 as f64).total_cmp(&(b.1.0 / b.1.1 as f64)))
        .map(|(candidate, _)| (*candidate).clone());
    let regret = chosen
        .as_ref()
        .and_then(true_mean)
        .map_or(f64::NAN, |mean| best - mean);
    let first_try = chosen
        .as_ref()
        .and_then(|chosen| history.iter().position(|trial| &trial.candidate == chosen));
    Ok(SearchReport {
        policy: policy.name(),
        trials_used: history.len(),
        chosen,
        regret,
        trials_to_first_try_of_chosen: first_try,
    })
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
