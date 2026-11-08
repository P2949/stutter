//! A matched-workload comparison of two sched_ext (or kernel) schedulers.
//!
//! A scheduler comparison is only meaningful when the same workload ran under both on the same
//! kernel, measuring the same metric, in independent blocks. This refuses anything else, pairs the
//! blocks by id, converts each pair to a signed percent improvement (positive means the candidate
//! is better) and reports the interval of the mean. It is offline analysis: it never loads,
//! switches or tunes a scheduler, and a result is a comparison, not a deployment decision.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::statistics::{Interval, paired_interval};

/// Fewest paired blocks a comparison needs.
pub const MIN_BLOCKS: usize = 3;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SchedulerRun {
    pub scheduler: String,
    pub kernel: String,
    pub workload_id: String,
    /// Digest of the exact workload and input: both schedulers must have run the same one.
    pub workload_digest: String,
    /// The independent block this run belongs to; the pairing key.
    pub block_id: u32,
    pub metric: String,
    pub unit: String,
    pub lower_is_better: bool,
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompareRefused {
    /// A scheduler has no run at all.
    MissingScheduler(String),
    /// The two sides differ in something that makes the comparison meaningless.
    Mismatched(&'static str),
    /// A block id appears twice for one scheduler.
    DuplicateBlock(u32),
    /// A block ran under only one of the two.
    UnpairedBlock(u32),
    TooFewBlocks(usize),
    BadValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    CandidateBetter,
    CandidateWorse,
    Inconclusive,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SchedulerComparison {
    pub baseline: String,
    pub candidate: String,
    pub interval: Interval,
    pub verdict: Verdict,
}

type Key = (String, String, String, String, bool);

fn identity(run: &SchedulerRun) -> Key {
    (
        run.kernel.clone(),
        run.workload_digest.clone(),
        run.metric.clone(),
        run.unit.clone(),
        run.lower_is_better,
    )
}

pub fn compare_schedulers(
    runs: &[SchedulerRun],
    baseline: &str,
    candidate: &str,
    confidence: f64,
) -> Result<SchedulerComparison, CompareRefused> {
    let side = |name: &str| -> Result<BTreeMap<u32, &SchedulerRun>, CompareRefused> {
        let mut blocks = BTreeMap::new();
        for run in runs.iter().filter(|run| run.scheduler == name) {
            if !run.value.is_finite() || run.value <= 0.0 {
                return Err(CompareRefused::BadValue);
            }
            if blocks.insert(run.block_id, run).is_some() {
                return Err(CompareRefused::DuplicateBlock(run.block_id));
            }
        }
        if blocks.is_empty() {
            Err(CompareRefused::MissingScheduler(name.to_owned()))
        } else {
            Ok(blocks)
        }
    };
    let (base, cand) = (side(baseline)?, side(candidate)?);
    let first = base.values().next().map(|run| identity(run));
    let reference = base.values().next().copied();
    let odd = base
        .values()
        .chain(cand.values())
        .find(|run| Some(identity(run)) != first);
    if let (Some(odd), Some(reference)) = (odd, reference) {
        return Err(CompareRefused::Mismatched(
            if odd.kernel != reference.kernel {
                "kernel"
            } else if odd.workload_digest != reference.workload_digest {
                "workload"
            } else if odd.metric != reference.metric || odd.unit != reference.unit {
                "metric or unit"
            } else {
                "direction"
            },
        ));
    }
    if let Some(unpaired) = base.keys().find(|id| !cand.contains_key(id)) {
        return Err(CompareRefused::UnpairedBlock(*unpaired));
    }
    if let Some(unpaired) = cand.keys().find(|id| !base.contains_key(id)) {
        return Err(CompareRefused::UnpairedBlock(*unpaired));
    }
    if base.len() < MIN_BLOCKS {
        return Err(CompareRefused::TooFewBlocks(base.len()));
    }
    let improvements: Vec<f64> = base
        .iter()
        .map(|(id, a)| {
            let b = cand[id];
            let change = if a.lower_is_better {
                a.value - b.value
            } else {
                b.value - a.value
            };
            change / a.value * 100.0
        })
        .collect();
    let interval = paired_interval(&improvements, confidence).ok_or(CompareRefused::BadValue)?;
    let verdict = if interval.low > 0.0 {
        Verdict::CandidateBetter
    } else if interval.high < 0.0 {
        Verdict::CandidateWorse
    } else {
        Verdict::Inconclusive
    };
    Ok(SchedulerComparison {
        baseline: baseline.to_owned(),
        candidate: candidate.to_owned(),
        interval,
        verdict,
    })
}

#[cfg(test)]
#[path = "sched_compare_tests.rs"]
mod tests;
