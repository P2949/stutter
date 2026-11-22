//! The mandatory observation G3 stands on (PLAN G3): what the resident controller's own runtime saw of the real
//! game during each attempt, apart from the optional direct frame source. A game-scoped record per completed
//! attempt names the exact game process and the interval, and shows the renderer bound, the cgroup, CPU, GPU
//! and PSI samples with none lost, and the owners of what ran beside it. The runnable-event floor is NOT part of
//! this requirement: a quiet scene or a GPU-bound benchmark can have few runnable events and still be fully
//! observed. It applies only to an attempt that is used to claim a runnable-latency measurement
//! (`runnable_claim`), which then also needs its events lossless. The record is produced by the resident
//! runtime during a physical run; a campaign without it has no G3.

use serde::Deserialize;

use super::{child_verify::AttemptProcess, quality::MIN_RUNNABLE_EVENTS};

/// The file, beside the study, holding the per-attempt observations.
pub const FILE: &str = "resident-observation.json";
/// A sampled signal with fewer samples than this was not observed over the attempt.
pub const MIN_SAMPLES: u64 = 10;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Observation {
    pub slot: u32,
    pub pid: stutter_core::ids::Pid,
    pub start_ticks: u64,
    pub uid: u32,
    /// The interval the observation covers; it must lie inside the attempt's own start and end.
    pub window_start_unix: u64,
    pub window_end_unix: u64,
    pub renderer_bound: bool,
    pub cgroup: String,
    pub cpu_samples: u64,
    pub gpu_samples: u64,
    pub psi_samples: u64,
    /// Samples the source lost across the CPU, GPU and PSI signals over the window.
    pub lost_samples: u64,
    pub owners_identified: bool,
    pub runnable_events: u64,
    pub runnable_lost: u64,
    /// This attempt is used to claim a runnable-latency measurement, so its events must meet the floor.
    pub runnable_claim: bool,
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
    if record.schema != 2 {
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
        if one.window_start_unix < process.started_unix
            || one.window_end_unix > process.completed_unix
            || one.window_start_unix >= one.window_end_unix
        {
            return Err("ResidentObservationOutsideTheAttempt");
        }
        if !one.renderer_bound || one.cgroup.is_empty() || !one.owners_identified {
            return Err("ResidentObservationIncomplete");
        }
        if one.lost_samples != 0 {
            return Err("ResidentObservationSourceLossy");
        }
        if one.cpu_samples < MIN_SAMPLES
            || one.gpu_samples < MIN_SAMPLES
            || one.psi_samples < MIN_SAMPLES
        {
            return Err("ResidentObservationTooSparse");
        }
        if one.runnable_claim
            && (one.runnable_events < MIN_RUNNABLE_EVENTS || one.runnable_lost != 0)
        {
            return Err("ResidentObservationRunnableFloor");
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
