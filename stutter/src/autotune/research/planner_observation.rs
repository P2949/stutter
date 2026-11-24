//! A justified abstention is a claim about the controller's planner, so it rests on what the planner saw: for every
//! completed attempt of an action study, the resident planner's own evaluations of the game process over a window
//! inside that attempt, from a direct source with nothing lost, and no candidate for the authorized action in any of
//! them. The absence of receipts alone is not evidence that no action was indicated (a controller that never ran, or
//! never saw the game, also leaves none). The record is produced by the resident runtime during a physical run;
//! nothing here fabricates one.

use serde::Deserialize;
use serde_json::Value;

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
    /// Passes that ran but denied every authorized action: a refusal, not an indication that none was needed.
    #[serde(default)]
    denied: u64,
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
        if entry.denied != 0 {
            return Err("PlannerDeniedTheAction");
        }
    }
    Ok(())
}

/// Whether every entry of the sealed `record` is exactly what its attempt's raw window (`resident-windows.json`) counted:
/// the evaluations, the candidates offered of the authorized action, the passes that denied it, the lost events and the
/// source quality, over the part of the window inside the attempt. A summary that differs from its raw source, or an
/// attempt with no raw window, is refused.
pub fn matches_windows(
    bytes: &[u8],
    action: ResearchAction,
    windows: &Value,
    processes: &[AttemptProcess],
) -> Result<(), &'static str> {
    const WHY: &str = "PlannerObservationNotFromTheWindows";
    let record: Value = serde_json::from_slice(bytes).map_err(|_| WHY)?;
    let windows = windows.as_array().ok_or(WHY)?;
    for entry in record["attempts"].as_array().ok_or(WHY)? {
        let process = processes
            .iter()
            .find(|p| p.slot as u64 == entry["slot"].as_u64().unwrap_or(u64::MAX))
            .ok_or(WHY)?;
        let window = windows
            .iter()
            .find(|w| {
                w["attempt"].as_u64() == Some(u64::from(process.slot))
                    && w["pid"].as_u64() == Some(u64::from(process.pid.as_u32()))
                    && w["start_ticks"].as_u64() == Some(process.start_ticks)
            })
            .ok_or(WHY)?;
        let counted = |name: &str| window[name].as_u64().ok_or(WHY);
        let candidates = match action {
            ResearchAction::CpuAffinityProfile => counted("affinity_candidates")?,
            ResearchAction::CgroupPlacement => counted("cgroup_candidates")?,
        };
        let lost = counted("lost_events")?.saturating_add(counted("invalid_intervals")?);
        let direct = counted("cpu_samples")? > 0 && lost == 0;
        let (start, end) = (
            counted("started_unix")?.max(process.started_unix),
            counted("ended_unix")?.min(process.completed_unix),
        );
        let same = entry["evaluations"].as_u64() == Some(counted("evaluations")?)
            && entry["candidates"].as_u64() == Some(candidates)
            && entry["denied"].as_u64().unwrap_or(0) == counted("denied_evaluations")?
            && entry["lost"].as_u64() == Some(lost)
            && entry["source_quality"].as_str() == Some(if direct { "direct" } else { "derived" })
            && entry["start_unix"].as_u64() == Some(start)
            && entry["end_unix"].as_u64() == Some(end);
        if !same {
            return Err(WHY);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "planner_observation_tests.rs"]
mod tests;
