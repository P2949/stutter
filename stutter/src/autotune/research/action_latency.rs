//! The measured objective an action study must carry beside its safety evidence (PLAN G5): the game's
//! runnable latency with the action off and on over paired attempts. Applying and restoring an action is a safety
//! claim; whether it helped is a measurement, and a study without a valid one is not a qualified action.
//!
//! The record carries sealed per-window populations in the controller's own canonical histogram layout, never a
//! stated percentile or a declared quality. Each pair is decided by the production objective comparison
//! (`objective::runnable_replay`): the controller's thresholds, its worsening-tail rejection and its frame,
//! foreground, power and thermal guards. This module only validates that the windows are measurements of the
//! planned attempts, requires every planned pair, and combines the pair decisions. It defines no keep or revert
//! threshold of its own. The record is produced from the resident runtime's own samples during a physical run;
//! nothing here fabricates one.

use serde::Deserialize;

use super::{
    Treatment,
    child_verify::AttemptProcess,
    quality::{RunnableVerdict, judge_runnable},
};
use crate::{
    autotune::{
        comparison::ExperimentResult,
        objective::runnable_replay::{ReplayWindow, replay_pair, window_problem},
    },
    metrics::LatencyHistogramBucket,
};

/// The file, beside the study, holding the paired runnable-latency record.
pub const FILE: &str = "action-latency.json";
/// Paired off/on attempts needed before a comparison means anything.
pub const MIN_PAIRS: usize = 3;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityRef {
    pub pid: stutter_core::ids::Pid,
    pub start_ticks: u64,
    pub uid: u32,
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
    /// The target game's merged runnable-delay population, in the canonical bucket layout, and its maximum.
    pub histogram: Vec<LatencyHistogramBucket>,
    pub max_ns: u64,
    /// The foreground's own frame-time tail, frame count and over-5-ms count over the same window.
    pub foreground_p99_ms: f64,
    pub foreground_samples: u64,
    pub foreground_over_5ms: u64,
    pub thermal_degraded: bool,
    pub thermal_throttle_count: u64,
    pub cpu_power_limited: bool,
    pub gpu_power_limited: bool,
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

/// What the controller's own decision says of the planned pairs, taken together.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Most pairs improved under the production decision and none regressed.
    Keep,
    /// Most pairs regressed under the production decision.
    Revert,
    /// Anything else: a valid measurement that does not show a benefit.
    Null,
}

#[derive(Debug, PartialEq)]
pub struct LatencyResult {
    pub pairs: usize,
    pub improved: usize,
    pub regressed: usize,
    pub decision: Decision,
}

impl LatencyResult {
    /// The result as the final report carries it: the objective, the pair counts and the decision, never a native FPS.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "objective": "game_runnable_latency",
            "pairs": self.pairs, "improved": self.improved, "regressed": self.regressed,
            "decision": match self.decision {
                Decision::Keep => "keep",
                Decision::Revert => "revert",
                Decision::Null => "null",
            },
        })
    }
}

fn replay_window(window: &Window, target: &IdentityRef) -> ReplayWindow {
    // Each window is its own attempt's game process (already checked against that attempt); the production
    // comparison needs the same target on both sides, so both carry the pair's baseline identity.
    ReplayWindow {
        histogram: window.histogram.clone(),
        max_ns: window.max_ns,
        target_pid: target.pid.as_u32(),
        target_start_ticks: target.start_ticks,
        frame_p99_ms: window.foreground_p99_ms,
        scored_samples: window.foreground_samples,
        foreground_over_5ms: window.foreground_over_5ms,
        thermal_degraded: window.thermal_degraded,
        thermal_throttle_count: window.thermal_throttle_count,
        cpu_power_limited: window.cpu_power_limited,
        gpu_power_limited: window.gpu_power_limited,
    }
}

/// Why the record cannot stand as the study's measurement, or its result. Every planned pair (a block whose Off and
/// On attempts both completed) must be present and no other; each names the schedule's own attempts and the very
/// processes they recorded; each window lies inside its attempt, comes from one unchanged target with no lost events
/// and at least the production sample floor, and is a valid canonical population.
pub fn verify(
    bytes: &[u8],
    treatments: &[Treatment],
    processes: &[AttemptProcess],
) -> Result<LatencyResult, &'static str> {
    let record: LatencyRecord =
        serde_json::from_slice(bytes).map_err(|_| "ActionLatencyUnreadable")?;
    if record.schema != 3 || record.metric != "GameRunnableLatency" {
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
    let (mut improved, mut regressed) = (0usize, 0usize);
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
            if !(window.foreground_p99_ms.is_finite() && window.foreground_p99_ms > 0.0) {
                return Err("ActionLatencyNotFinite");
            }
            let events = window
                .histogram
                .iter()
                .try_fold(0u64, |total, bucket| total.checked_add(bucket.count));
            match judge_runnable(events, Some(window.lost), window.same_target) {
                RunnableVerdict::Eligible => {}
                RunnableVerdict::BelowSampleFloor => return Err("ActionLatencyTooFewEvents"),
                _ => return Err("ActionLatencySourceNotQualified"),
            }
            if window_problem(&replay_window(window, identity)).is_some() {
                return Err("ActionLatencyHistogramInvalid");
            }
        }
        let baseline = replay_window(&pair.off, &pair.off_process);
        let candidate = replay_window(&pair.on, &pair.off_process);
        match replay_pair(&baseline, &candidate) {
            ExperimentResult::Improved { .. } => improved += 1,
            ExperimentResult::Regressed { .. } => regressed += 1,
            _ => {}
        }
    }
    let pairs = record.pairs.len();
    let decision = if improved * 2 > pairs && regressed == 0 {
        Decision::Keep
    } else if regressed * 2 > pairs {
        Decision::Revert
    } else {
        Decision::Null
    };
    Ok(LatencyResult {
        pairs,
        improved,
        regressed,
        decision,
    })
}

#[cfg(test)]
#[path = "action_latency_tests.rs"]
mod tests;
