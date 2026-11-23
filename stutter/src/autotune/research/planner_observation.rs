//! A justified abstention is a claim about the controller's planner, so it rests on what the planner saw: for every
//! completed attempt of an action study, the resident planner's own evaluations of the game process over a window
//! inside that attempt, from a direct source with nothing lost, and no candidate for the authorized action in any of
//! them. The absence of receipts alone is not evidence that no action was indicated (a controller that never ran, or
//! never saw the game, also leaves none). The record is produced by the resident runtime during a physical run;
//! nothing here fabricates one.

use serde::Deserialize;

use super::{ResearchAction, action_latency::IdentityRef, child_verify::AttemptProcess};

/// The file, beside the study, holding the planner's per-attempt observations.
pub const FILE: &str = "planner-observation.json";
/// Evaluations of an attempt below which the planner cannot be said to have watched it.
pub const MIN_EVALUATIONS: u64 = 10;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    slot: u32,
    process: IdentityRef,
    start_unix: u64,
    end_unix: u64,
    /// Planning passes that considered the game process in the window.
    evaluations: u64,
    /// Passes that produced a candidate for the authorized action.
    candidates: u64,
    lost: u64,
    source_quality: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    action: String,
    attempts: Vec<Entry>,
}

/// Why the record cannot support a justified abstention of this action over these completed attempts, or `Ok`.
pub fn verify(
    bytes: &[u8],
    action: ResearchAction,
    processes: &[AttemptProcess],
) -> Result<(), &'static str> {
    let record: Record =
        serde_json::from_slice(bytes).map_err(|_| "PlannerObservationUnreadable")?;
    if record.schema != 1 || record.action != format!("{action:?}") {
        return Err("PlannerObservationForAnotherAction");
    }
    let mut slots: Vec<u32> = record.attempts.iter().map(|entry| entry.slot).collect();
    slots.sort_unstable();
    let mut expected: Vec<u32> = processes.iter().map(|process| process.slot).collect();
    expected.sort_unstable();
    if slots != expected || expected.is_empty() {
        return Err("PlannerObservationNotEveryAttempt");
    }
    for entry in &record.attempts {
        let Some(process) = processes.iter().find(|process| process.slot == entry.slot) else {
            return Err("PlannerObservationNotEveryAttempt");
        };
        if process.pid != entry.process.pid
            || process.start_ticks != entry.process.start_ticks
            || process.uid != entry.process.uid
        {
            return Err("PlannerObservationForAnotherProcess");
        }
        if entry.start_unix < process.started_unix
            || entry.end_unix > process.completed_unix
            || entry.start_unix >= entry.end_unix
        {
            return Err("PlannerObservationOutsideTheAttempt");
        }
        if entry.source_quality != "direct" || entry.lost != 0 {
            return Err("PlannerObservationSourceNotQualified");
        }
        if entry.evaluations < MIN_EVALUATIONS {
            return Err("PlannerObservationTooFewEvaluations");
        }
        if entry.candidates != 0 {
            return Err("PlannerHadACandidate");
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "planner_observation_tests.rs"]
mod tests;
