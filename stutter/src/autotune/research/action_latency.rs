//! The measured objective an action study must carry beside its safety evidence (PLAN G5): the game's
//! runnable latency, as the 99th percentile, with the action off and on over paired attempts. Applying and
//! restoring an action is a safety claim; whether it helped is a measurement, and a study without a valid one
//! is not a qualified action. The record carries sealed per-window populations (histograms), never a stated
//! percentile or a declared quality: this reader recomputes the 99th percentile itself, judges each window's
//! source with the same runnable-source rule the controller uses, applies the frozen keep and revert thresholds
//! and the foreground guardrail, and requires every planned pair. The record is produced from the resident
//! runtime's own samples during a physical run; nothing here fabricates one.

use serde::Deserialize;

use super::{
    Treatment,
    child_verify::AttemptProcess,
    quality::{RunnableVerdict, judge_runnable},
};

/// The file, beside the study, holding the paired runnable-latency record.
pub const FILE: &str = "action-latency.json";
/// Paired off/on attempts needed before a comparison means anything.
pub const MIN_PAIRS: usize = 3;
/// The frozen practical-effect threshold on the relative change of the 99th percentile: the median pair must move
/// by at least this much for a keep (down) or a revert (up); anything inside is a null result.
pub const KEEP_RELATIVE: f64 = 0.05;
/// The foreground frame-time guardrail: the action may not make the foreground's 99th percentile worse by more.
pub const FOREGROUND_REGRESSION: f64 = 0.05;
const PERCENTILE: f64 = 0.99;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityRef {
    pub pid: stutter_core::ids::Pid,
    pub start_ticks: u64,
    pub uid: u32,
}

/// One histogram bucket: events whose runnable delay was at most `le_ms` (and above the previous bucket).
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bucket {
    pub le_ms: f64,
    pub count: u64,
}

/// One attempt's scoped window.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Window {
    pub start_unix: u64,
    pub end_unix: u64,
    /// Events the source dropped inside the window.
    pub lost: u64,
    /// The scoped target (pid, start time, uid) was the same process for the whole window.
    pub same_target: bool,
    pub histogram: Vec<Bucket>,
    /// The foreground's own 99th-percentile frame time over the same window, for the guardrail.
    pub foreground_p99_ms: f64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LatencyPair {
    pub off_slot: u32,
    pub on_slot: u32,
    pub off_process: IdentityRef,
    pub on_process: IdentityRef,
    pub off: Window,
    pub on: Window,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LatencyRecord {
    pub schema: u32,
    pub metric: String,
    pub pairs: Vec<LatencyPair>,
}

/// What the objective's frozen rule says of the measurement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// The median pair's 99th percentile fell by at least `KEEP_RELATIVE` and most pairs improved.
    Keep,
    /// The median pair's 99th percentile rose by at least `KEEP_RELATIVE`.
    Revert,
    /// Inside the threshold: a valid result that finds no effect.
    Null,
}

#[derive(Debug, PartialEq)]
pub struct LatencyResult {
    pub pairs: usize,
    pub improved: usize,
    pub decision: Decision,
}

/// The nearest-rank 99th percentile of a histogram, `None` when its buckets are not a valid population.
fn p99(histogram: &[Bucket]) -> Option<(f64, u64)> {
    let mut last = 0.0f64;
    let mut total = 0u64;
    for bucket in histogram {
        if !(bucket.le_ms.is_finite() && bucket.le_ms > last) {
            return None;
        }
        last = bucket.le_ms;
        total = total.checked_add(bucket.count)?;
    }
    if total == 0 {
        return None;
    }
    let rank = (PERCENTILE * total as f64).ceil() as u64;
    let mut cumulative = 0u64;
    for bucket in histogram {
        cumulative += bucket.count;
        if cumulative >= rank {
            return Some((bucket.le_ms, total));
        }
    }
    None
}

/// Why the record cannot stand as the study's measurement, or its result. Every planned pair (a block whose Off and
/// On attempts both completed) must be present and no other; each names the schedule's own attempts and the very
/// processes they recorded; each window lies inside its attempt, comes from one unchanged target with no lost events
/// and at least the runnable sample floor, and has a valid histogram the 99th percentile is recomputed from.
pub fn verify(
    bytes: &[u8],
    treatments: &[Treatment],
    processes: &[AttemptProcess],
) -> Result<LatencyResult, &'static str> {
    let record: LatencyRecord =
        serde_json::from_slice(bytes).map_err(|_| "ActionLatencyUnreadable")?;
    if record.schema != 2 || record.metric != "GameRunnableLatency" {
        return Err("ActionLatencyWrongMetric");
    }
    let completed = |slot: u32| processes.iter().find(|p| p.slot == slot);
    // The pairs the schedule planned and both of whose attempts completed: none may be left out.
    let mut planned: Vec<(u32, u32)> = Vec::new();
    for block in 0..treatments.len() / 2 {
        let (a, b) = (block as u32 * 2, block as u32 * 2 + 1);
        let (off, on) = if treatments[a as usize] == Treatment::Off {
            (a, b)
        } else {
            (b, a)
        };
        if treatments[off as usize] == Treatment::Off
            && treatments[on as usize] == Treatment::On
            && completed(off).is_some()
            && completed(on).is_some()
        {
            planned.push((off, on));
        }
    }
    if planned.len() < MIN_PAIRS {
        return Err("ActionLatencyTooFewPairs");
    }
    let mut named: Vec<(u32, u32)> = record
        .pairs
        .iter()
        .map(|p| (p.off_slot, p.on_slot))
        .collect();
    named.sort_unstable();
    planned.sort_unstable();
    if named != planned {
        return Err("ActionLatencyPairNotInTheSchedule");
    }
    let mut changes = Vec::new();
    for pair in &record.pairs {
        for (slot, identity, window) in [
            (pair.off_slot, &pair.off_process, &pair.off),
            (pair.on_slot, &pair.on_process, &pair.on),
        ] {
            let Some(process) = completed(slot) else {
                return Err("ActionLatencyPairNotInTheSchedule");
            };
            if process.pid != identity.pid
                || process.start_ticks != identity.start_ticks
                || process.uid != identity.uid
            {
                return Err("ActionLatencyForAnotherProcess");
            }
            if window.start_unix < process.started_unix
                || window.end_unix > process.completed_unix
                || window.start_unix >= window.end_unix
            {
                return Err("ActionLatencyWindowOutsideTheAttempt");
            }
            let Some((_, events)) = p99(&window.histogram) else {
                return Err("ActionLatencyNotFinite");
            };
            match judge_runnable(Some(events), Some(window.lost), window.same_target) {
                RunnableVerdict::Eligible => {}
                RunnableVerdict::BelowSampleFloor => return Err("ActionLatencyTooFewEvents"),
                _ => return Err("ActionLatencySourceNotQualified"),
            }
            if !(window.foreground_p99_ms.is_finite() && window.foreground_p99_ms > 0.0) {
                return Err("ActionLatencyNotFinite");
            }
        }
        // The guardrail: the action may not have made the foreground's tail worse.
        if pair.on.foreground_p99_ms > pair.off.foreground_p99_ms * (1.0 + FOREGROUND_REGRESSION) {
            return Err("ActionLatencyGuardrailBroken");
        }
        let (Some((off, _)), Some((on, _))) = (p99(&pair.off.histogram), p99(&pair.on.histogram))
        else {
            return Err("ActionLatencyNotFinite");
        };
        changes.push(on / off - 1.0);
    }
    changes.sort_by(f64::total_cmp);
    let median = if changes.len() % 2 == 1 {
        changes[changes.len() / 2]
    } else {
        (changes[changes.len() / 2 - 1] + changes[changes.len() / 2]) / 2.0
    };
    let improved = changes.iter().filter(|change| **change < 0.0).count();
    let decision = if median <= -KEEP_RELATIVE && improved * 2 > changes.len() {
        Decision::Keep
    } else if median >= KEEP_RELATIVE {
        Decision::Revert
    } else {
        Decision::Null
    };
    Ok(LatencyResult {
        pairs: changes.len(),
        improved,
        decision,
    })
}

#[cfg(test)]
#[path = "action_latency_tests.rs"]
mod tests;
