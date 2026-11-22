//! The measured objective an action study must carry beside its safety evidence (PLAN G5): the game's
//! runnable latency with the action off and on, over paired attempts. Applying and restoring an action is
//! a safety claim; whether it helped is a measurement, and a study without a valid one is not a qualified
//! action. This module is the sealed record and its independent reader. The record is produced from the
//! resident runtime's own source-qualified samples during a physical run; nothing here fabricates one.

use serde::Deserialize;

use super::{Treatment, child_verify::AttemptProcess};

/// The file, beside the study, holding the paired runnable-latency record.
pub const FILE: &str = "action-latency.json";
/// A window with fewer scoped events than this cannot show a latency difference.
pub const MIN_EVENTS: u64 = 1000;
/// Paired off/on attempts needed before a comparison means anything.
pub const MIN_PAIRS: usize = 3;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityRef {
    pub pid: stutter_core::ids::Pid,
    pub start_ticks: u64,
    pub uid: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LatencyPair {
    pub off_slot: u32,
    pub on_slot: u32,
    pub off_process: IdentityRef,
    pub on_process: IdentityRef,
    pub off_events: u64,
    pub on_events: u64,
    pub off_p95_ms: f64,
    pub on_p95_ms: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LatencyRecord {
    pub schema: u32,
    pub metric: String,
    /// The frame-independent source quality the resident runtime judged for the run ("Eligible").
    pub source_quality: String,
    /// The protected objectives held: no foreground frame-time regression, other processes intact.
    pub guardrails_held: bool,
    pub pairs: Vec<LatencyPair>,
}

/// What a valid record says: the pairs it covers and how many showed lower latency with the action on.
#[derive(Debug, PartialEq, Eq)]
pub struct LatencyResult {
    pub pairs: usize,
    pub improved: usize,
}

/// Why the record cannot stand as the study's measurement, or its result. Each pair must name the schedule's own
/// Off and On attempts of one block, the very game processes those attempts recorded, enough scoped events on
/// both sides, and finite positive latencies; the source must have been qualified and the guardrails held.
pub fn verify(
    bytes: &[u8],
    treatments: &[Treatment],
    processes: &[AttemptProcess],
) -> Result<LatencyResult, &'static str> {
    let record: LatencyRecord =
        serde_json::from_slice(bytes).map_err(|_| "ActionLatencyUnreadable")?;
    if record.schema != 1 || record.metric != "GameRunnableLatency" {
        return Err("ActionLatencyWrongMetric");
    }
    if record.source_quality != "Eligible" {
        return Err("ActionLatencySourceNotQualified");
    }
    if !record.guardrails_held {
        return Err("ActionLatencyGuardrailBroken");
    }
    if record.pairs.len() < MIN_PAIRS {
        return Err("ActionLatencyTooFewPairs");
    }
    let process_of = |slot: u32| processes.iter().find(|process| process.slot == slot);
    let same = |slot: u32, identity: &IdentityRef| {
        process_of(slot).is_some_and(|process| {
            process.pid == identity.pid
                && process.start_ticks == identity.start_ticks
                && process.uid == identity.uid
        })
    };
    let mut used = std::collections::BTreeSet::new();
    let mut improved = 0;
    for pair in &record.pairs {
        let scheduled = treatments.get(pair.off_slot as usize) == Some(&Treatment::Off)
            && treatments.get(pair.on_slot as usize) == Some(&Treatment::On)
            && pair.off_slot / 2 == pair.on_slot / 2;
        if !scheduled || !used.insert(pair.off_slot) || !used.insert(pair.on_slot) {
            return Err("ActionLatencyPairNotInTheSchedule");
        }
        if !same(pair.off_slot, &pair.off_process) || !same(pair.on_slot, &pair.on_process) {
            return Err("ActionLatencyForAnotherProcess");
        }
        if pair.off_events < MIN_EVENTS || pair.on_events < MIN_EVENTS {
            return Err("ActionLatencyTooFewEvents");
        }
        let valid = |value: f64| value.is_finite() && value > 0.0;
        if !valid(pair.off_p95_ms) || !valid(pair.on_p95_ms) {
            return Err("ActionLatencyNotFinite");
        }
        if pair.on_p95_ms < pair.off_p95_ms {
            improved += 1;
        }
    }
    Ok(LatencyResult {
        pairs: record.pairs.len(),
        improved,
    })
}

#[cfg(test)]
#[path = "action_latency_tests.rs"]
mod tests;
