//! The mandatory observation G3 stands on (PLAN G3): what the resident controller's own runtime saw of the real
//! game during each attempt, apart from the optional direct frame source. A game-scoped record per completed
//! attempt names the exact game process and shows the renderer bound, the cgroup, CPU, GPU and PSI samples, the
//! runnable-delay events, and the owners of what ran beside it, from a source judged eligible. This module is the
//! sealed record and its independent reader; the record is produced by the resident runtime during a physical
//! run, and a campaign without it has no G3.

use serde::Deserialize;

use super::child_verify::AttemptProcess;

/// The file, beside the study, holding the per-attempt observations.
pub const FILE: &str = "resident-observation.json";
/// A sampled signal with fewer samples than this was not observed over the attempt.
pub const MIN_SAMPLES: u64 = 10;
/// Runnable-delay events needed for the latency signal.
pub const MIN_RUNNABLE_EVENTS: u64 = 1000;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub slot: u32,
    pub pid: stutter_core::ids::Pid,
    pub start_ticks: u64,
    pub uid: u32,
    pub renderer_bound: bool,
    pub cgroup: String,
    pub cpu_samples: u64,
    pub gpu_samples: u64,
    pub psi_samples: u64,
    pub runnable_events: u64,
    pub owners_identified: bool,
    pub quality: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: u32,
    attempts: Vec<Observation>,
}

/// Why the record is not the mandatory observation of exactly these completed attempts, or `Ok`.
pub fn verify(bytes: &[u8], processes: &[AttemptProcess]) -> Result<(), &'static str> {
    let record: Record = serde_json::from_slice(bytes).map_err(|_| "ResidentObservationInvalid")?;
    if record.schema != 1 {
        return Err("ResidentObservationInvalid");
    }
    for process in processes {
        let mut mine = record.attempts.iter().filter(|o| o.slot == process.slot);
        let (Some(one), None) = (mine.next(), mine.next()) else {
            return Err("ResidentObservationMissingForAnAttempt");
        };
        if one.pid != process.pid
            || one.start_ticks != process.start_ticks
            || one.uid != process.uid
        {
            return Err("ResidentObservationForAnotherProcess");
        }
        if one.quality != "Eligible" {
            return Err("ResidentObservationSourceNotQualified");
        }
        if !one.renderer_bound || one.cgroup.is_empty() || !one.owners_identified {
            return Err("ResidentObservationIncomplete");
        }
        if one.cpu_samples < MIN_SAMPLES
            || one.gpu_samples < MIN_SAMPLES
            || one.psi_samples < MIN_SAMPLES
            || one.runnable_events < MIN_RUNNABLE_EVENTS
        {
            return Err("ResidentObservationTooSparse");
        }
    }
    if record.attempts.len() != processes.len() {
        return Err("ResidentObservationForNoCompletedAttempt");
    }
    Ok(())
}

#[cfg(test)]
#[path = "resident_observation_tests.rs"]
mod tests;
