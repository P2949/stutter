//! Per-attempt cache state (PLAN D6, section 6.1). A study over a cold cohort (prefixes no game run
//! had initialized) makes each arm's first run a cold-cache run: shader pipelines are compiled and
//! assets streamed for the first time, so its frame times describe first use, not steady state, and
//! pooling it with the arm's later warm runs would mix two cohorts. The state of each attempt is a
//! function of the study's sealed cohort and its sealed evidence, so a verifier recomputes it and
//! nothing needs to be trusted: the first completed run of each arm in a cold-cohort study is
//! `ColdFirstUse`, and every other run is `Warm`. A cohort that is not recorded as cold keeps every
//! run as it was (such studies cannot be qualified comparisons anyway).

use std::collections::{BTreeMap, BTreeSet};

use super::ProtonSource;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheState {
    ColdFirstUse,
    Warm,
}

/// One completed run: its journal slot, the arm it ran, and its native average FPS.
pub type Run = (u32, ProtonSource, f64);

/// The cache state of each run, in slot order.
pub fn classify(cohort: Option<&str>, runs: &[Run]) -> Vec<(u32, CacheState)> {
    let mut ordered: Vec<&Run> = runs.iter().collect();
    ordered.sort_by_key(|(slot, _, _)| *slot);
    let cold = cohort == Some("cold");
    let mut seen = BTreeSet::new();
    ordered
        .into_iter()
        .map(|(slot, source, _)| {
            let first = seen.insert(*source);
            (
                *slot,
                if cold && first {
                    CacheState::ColdFirstUse
                } else {
                    CacheState::Warm
                },
            )
        })
        .collect()
}

/// Each arm's FPS over its warm runs only: the cold first-use runs are set aside, never averaged in.
pub fn warm_fps(cohort: Option<&str>, runs: &[Run]) -> BTreeMap<ProtonSource, Vec<f64>> {
    let states: BTreeMap<u32, CacheState> = classify(cohort, runs).into_iter().collect();
    let mut ordered: Vec<&Run> = runs.iter().collect();
    ordered.sort_by_key(|(slot, _, _)| *slot);
    let mut fps: BTreeMap<ProtonSource, Vec<f64>> = BTreeMap::new();
    for (slot, source, value) in ordered {
        if states.get(slot) == Some(&CacheState::Warm) {
            fps.entry(*source).or_default().push(*value);
        }
    }
    fps
}

#[cfg(test)]
#[path = "cache_state_tests.rs"]
mod tests;
