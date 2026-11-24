//! The raw events behind an attempt's guard facts (frame tail, thermal, power), kept with the second the observer saw each
//! one, so the facts can be recomputed over exactly the stretch an action was held instead of the whole attempt. Sealing
//! and the replay both derive the facts here, through the production objective-signal builder over a window holding only
//! those events; the replay refuses a sealed window whose guard values are not what the archived events give.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::GuardFacts;
use crate::{
    autotune::{
        research::{
            action_epochs::{pair_ranges, treated_interval},
            child_verify::AttemptProcess,
        },
        rolling_window::RollingWindow,
    },
    recorder::{CpuFreqRecord, FrameEvent, GpuSample},
};

/// The file, beside a study, holding every attempt's raw guard events.
pub const FILE: &str = "resident-guard-events.json";
/// Events kept for one attempt; past it the attempt has no cut guard facts (never a sampled subset).
pub const MAX_EVENTS: usize = 200_000;
/// The largest events file a study may hold.
pub const MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GuardEvent {
    Frame { at: u64, event: FrameEvent },
    Gpu { at: u64, sample: GpuSample },
    CpuFreq { at: u64, event: CpuFreqRecord },
}

impl GuardEvent {
    fn at(&self) -> u64 {
        match self {
            Self::Frame { at, .. } | Self::Gpu { at, .. } | Self::CpuFreq { at, .. } => *at,
        }
    }
}

/// The guard facts the production builder reads from `window` for the target `pid`, or `None` when any was not observed.
pub(super) fn facts_of(window: &RollingWindow, pid: u32) -> Option<GuardFacts> {
    let members = BTreeSet::from([pid]);
    let signals = window.objective_signals_for_target(pid, &members);
    Some(GuardFacts {
        frame_p99_ms: signals.frame_p99_ms?,
        frame_samples: signals.frame_samples?,
        foreground_over_5ms: signals.foreground_over_5ms?,
        thermal_degraded: signals.thermal_degraded?,
        thermal_throttle_count: signals.thermal_throttle_count?,
        cpu_power_limited: signals.cpu_power_limited?,
        gpu_power_limited: signals.gpu_power_limited?,
    })
}

/// The guard facts over the events seen in `[from, to)` seconds.
pub fn guards_over(events: &[GuardEvent], pid: u32, from: u64, to: u64) -> Option<GuardFacts> {
    let mut window = RollingWindow::new(std::time::Duration::from_secs(24 * 3_600));
    for event in events.iter().filter(|e| (from..to).contains(&e.at())) {
        match event {
            GuardEvent::Frame { event, .. } => window.push_frame(event.clone()),
            GuardEvent::Gpu { sample, .. } => window.push_gpu_sample(sample.clone()),
            GuardEvent::CpuFreq { event, .. } => window.push_cpu_freq_event(event.clone()),
        }
    }
    facts_of(&window, pid)
}

/// The events document of finished windows, one entry per attempt.
pub fn record(windows: &[super::AttemptResident]) -> Value {
    Value::Array(
        windows
            .iter()
            .map(|window| {
                serde_json::json!({
                    "attempt": window.attempt,
                    "pid": window.target.pid, "start_ticks": window.target.start_ticks,
                    "truncated": window.guard_events_truncated,
                    "events": window.guard_events,
                })
            })
            .collect(),
    )
}

fn events_of(doc: &[Value], process: &AttemptProcess) -> Option<Vec<GuardEvent>> {
    let entry = doc.iter().find(|w| {
        w["attempt"].as_u64() == Some(u64::from(process.slot))
            && w["pid"].as_u64() == Some(u64::from(process.pid.as_u32()))
            && w["start_ticks"].as_u64() == Some(process.start_ticks)
    })?;
    if entry["truncated"] == true {
        return None;
    }
    serde_json::from_value(entry["events"].clone()).ok()
}

/// Whether every window of the sealed latency `record` carries exactly the guard facts its attempt's archived events give
/// over the window's own stretch.
pub fn record_matches_events(
    record: &Value,
    events: &Value,
    receipts: &Value,
    processes: &[AttemptProcess],
) -> Result<(), &'static str> {
    const WHY: &str = "ActionGuardsNotFromTheEvents";
    let (pairs, doc, receipts) = (
        record["pairs"].as_array().ok_or(WHY)?,
        events.as_array().ok_or(WHY)?,
        receipts.as_array().ok_or(WHY)?,
    );
    let process_of = |slot: &Value| {
        processes
            .iter()
            .find(|p| u64::from(p.slot) == slot.as_u64().unwrap_or(u64::MAX))
    };
    for pair in pairs {
        let (Some(off), Some(on)) = (process_of(&pair["off_slot"]), process_of(&pair["on_slot"]))
        else {
            return Err(WHY);
        };
        let treated = receipts
            .iter()
            .find(|r| {
                r["pid"].as_u64() == Some(u64::from(on.pid.as_u32()))
                    && r["start_ticks"].as_u64() == Some(on.start_ticks)
            })
            .and_then(treated_interval)
            .ok_or(WHY)?;
        let (on_range, off_range) = pair_ranges(treated, on.started_unix, off.started_unix);
        for (process, range, window) in
            [(off, off_range, &pair["off"]), (on, on_range, &pair["on"])]
        {
            let facts = guards_over(
                &events_of(doc, process).ok_or(WHY)?,
                process.pid.as_u32(),
                range.0,
                range.1,
            )
            .ok_or(WHY)?;
            if window["foreground_p99_ms"].as_f64() != Some(facts.frame_p99_ms)
                || window["foreground_samples"].as_u64() != Some(facts.frame_samples)
                || window["foreground_over_5ms"].as_u64() != Some(facts.foreground_over_5ms)
                || window["thermal_degraded"].as_bool() != Some(facts.thermal_degraded)
                || window["thermal_throttle_count"].as_u64() != Some(facts.thermal_throttle_count)
                || window["cpu_power_limited"].as_bool() != Some(facts.cpu_power_limited)
                || window["gpu_power_limited"].as_bool() != Some(facts.gpu_power_limited)
            {
                return Err(WHY);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "guard_events_tests.rs"]
mod tests;
