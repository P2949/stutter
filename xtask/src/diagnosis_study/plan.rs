//! What the study does next, derived purely from the run and decision logs. Controls and
//! positives are captured in counterbalanced pairs (AB, BA, AB, ...): never all controls
//! followed by all positives. A family whose target impact is not demonstrated is extended in
//! steps up to a cap, then closed as invalid for calibration; it is never tuned around.
use super::{
    model::StudyConfig,
    store::{Decision, Outcome, Role, RunRecord},
};
use crate::validation_campaign::scenarios::model::Family;

/// One capture to take.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Slot {
    pub scenario: String,
    pub family: Option<Family>,
    pub role: Role,
    pub pair: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    Capture(Slot),
    Evaluate {
        family: Family,
        pairs: usize,
    },
    /// A slot spent its failure budget: stop with the retained rejections.
    Blocked(String),
    Done,
}

/// Roles of pair `pair` in capture order: even pairs AB, odd pairs BA.
pub fn pair_order(pair: usize) -> [Role; 2] {
    if pair.is_multiple_of(2) {
        [Role::Control, Role::Positive]
    } else {
        [Role::Positive, Role::Control]
    }
}

pub fn control_id(family: Family) -> String {
    format!("v2-{}-control", family.as_str())
}

pub fn positive_id(family: Family) -> String {
    format!("v2-{}-positive", family.as_str())
}

fn scenario_id(family: Family, role: Role) -> String {
    match role {
        Role::Control => control_id(family),
        _ => positive_id(family),
    }
}

fn accepted(runs: &[RunRecord], scenario: &str, pair: usize) -> bool {
    runs.iter()
        .any(|r| r.accepted && r.scenario == scenario && r.pair == pair)
}

fn failures(runs: &[RunRecord], scenario: &str, pair: usize) -> usize {
    runs.iter()
        .filter(|r| !r.accepted && r.scenario == scenario && r.pair == pair)
        .count()
}

/// Pairs the family is currently trying to complete; `None` once it is closed.
pub fn target_pairs(config: &StudyConfig, decisions: &[Decision], family: Family) -> Option<usize> {
    match decisions.iter().rev().find(|d| d.family == family) {
        None => Some(config.sampling.initial_pairs),
        Some(d) if d.outcome == Outcome::Extend => Some(d.pairs + config.sampling.extension_step),
        Some(_) => None,
    }
}

pub fn family_passed(decisions: &[Decision], family: Family) -> bool {
    decisions
        .iter()
        .rev()
        .find(|d| d.family == family)
        .is_some_and(|d| d.outcome == Outcome::Pass)
}

fn slot_or_block(
    config: &StudyConfig,
    runs: &[RunRecord],
    slot: Slot,
) -> Option<Result<Action, String>> {
    if accepted(runs, &slot.scenario, slot.pair) {
        return None;
    }
    let failed = failures(runs, &slot.scenario, slot.pair);
    if failed >= config.sampling.failed_attempts_per_capture {
        return Some(Err(format!(
            "{} #{} failed {failed} times (budget {}); see runs.ndjson",
            slot.scenario, slot.pair, config.sampling.failed_attempts_per_capture
        )));
    }
    Some(Ok(Action::Capture(slot)))
}

pub fn next_action(config: &StudyConfig, runs: &[RunRecord], decisions: &[Decision]) -> Action {
    for family in Family::ALL {
        let Some(target) = target_pairs(config, decisions, family) else {
            continue;
        };
        for pair in 0..target {
            for role in pair_order(pair) {
                let slot = Slot {
                    scenario: scenario_id(family, role),
                    family: Some(family),
                    role,
                    pair,
                };
                match slot_or_block(config, runs, slot) {
                    None => {}
                    Some(Ok(action)) => return action,
                    Some(Err(reason)) => return Action::Blocked(reason),
                }
            }
        }
        return Action::Evaluate {
            family,
            pairs: target,
        };
    }
    for mixed in &config.mixed {
        // A mixed capture only means something when every family it combines is valid.
        if !mixed.covers.iter().all(|f| family_passed(decisions, *f)) {
            continue;
        }
        for index in 0..mixed.count {
            let slot = Slot {
                scenario: mixed.id.clone(),
                family: None,
                role: Role::Mixed,
                pair: index,
            };
            match slot_or_block(config, runs, slot) {
                None => {}
                Some(Ok(action)) => return action,
                Some(Err(reason)) => return Action::Blocked(reason),
            }
        }
    }
    Action::Done
}
