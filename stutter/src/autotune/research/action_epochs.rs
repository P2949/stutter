//! The measurement epochs of an action study (PLAN G5). An action's effect is the runnable latency *while it was applied*,
//! so the population a pair compares is cut from the attempt's own per-interval record at the times the gate's receipt
//! says the action was held (applied, until its restoration was accepted), never taken from the whole attempt, where the
//! time before the apply and after the restoration would dilute or invent the effect. The Off attempt has no action, so
//! it is read over the same offsets from its own start: what the same stretch of the scene looked like without one.
//!
//! The resident runtime keeps one `Epoch` per interval event (bounded; a window that outgrew the bound says so and no
//! pair is derived from it). The sealing step and the independent replay both cut the windows with the functions here, so
//! a recorded window that is not exactly what its attempt's epochs and receipt give is refused.

use serde::{Deserialize, Serialize};

use crate::metrics::{LatencyHistogram, LatencyHistogramBucket};

/// The most epochs one attempt keeps (an hour of one-second intervals and more); beyond it the window is marked truncated.
pub const MAX_EPOCHS: usize = 8_192;

/// The target game's runnable-delay population of one interval event, in the canonical bucket layout (counts only).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Epoch {
    /// Unix seconds at which the runtime observed the interval.
    pub unix: u64,
    pub events: u64,
    pub max_ns: u64,
    pub counts: Vec<u64>,
}

/// The sum of the epochs observed in `[from, to)`, as a canonical population: its buckets, its maximum and its events.
/// `None` when an epoch is not in the canonical layout (it is refused, never reinterpreted).
pub fn population(
    epochs: &[Epoch],
    from: u64,
    to: u64,
) -> Option<(Vec<LatencyHistogramBucket>, u64, u64)> {
    let mut buckets = LatencyHistogram::new().snapshot();
    let (mut max_ns, mut events) = (0u64, 0u64);
    for epoch in epochs.iter().filter(|e| (from..to).contains(&e.unix)) {
        if epoch.counts.len() != buckets.len() {
            return None;
        }
        for (bucket, count) in buckets.iter_mut().zip(&epoch.counts) {
            bucket.count = bucket.count.saturating_add(*count);
        }
        max_ns = max_ns.max(epoch.max_ns);
        events = events.saturating_add(epoch.events);
    }
    Some((buckets, max_ns, events))
}

/// The seconds the receipt says the action was held, from its application to the accepted restoration.
pub fn treated_interval(receipt: &serde_json::Value) -> Option<(u64, u64)> {
    let (applied, settled) = (
        receipt["applied_unix"].as_u64()?,
        receipt["settled_unix"].as_u64()?,
    );
    (applied < settled).then_some((applied, settled))
}

/// The On attempt's treated interval and the Off attempt's same offsets from its own start.
pub fn pair_ranges(
    treated: (u64, u64),
    on_started: u64,
    off_started: u64,
) -> ((u64, u64), (u64, u64)) {
    let offset = treated.0.saturating_sub(on_started);
    let length = treated.1 - treated.0;
    (
        treated,
        (off_started + offset, off_started + offset + length),
    )
}

/// Whether every window of the sealed latency `record` is exactly what its attempt's own epochs (from the raw `windows`
/// record) and the gate's `receipts` give: the same start, end, population and maximum. A summary that differs from
/// its raw source, or whose attempt kept no epochs, is refused.
pub fn record_matches_epochs(
    record: &serde_json::Value,
    windows: &serde_json::Value,
    receipts: &serde_json::Value,
    processes: &[super::child_verify::AttemptProcess],
) -> Result<(), &'static str> {
    const WHY: &str = "ActionWindowsNotFromTheEpochs";
    let pairs = record["pairs"].as_array().ok_or(WHY)?;
    let windows = windows.as_array().ok_or(WHY)?;
    let receipts = receipts.as_array().ok_or(WHY)?;
    let epochs_of = |process: &super::child_verify::AttemptProcess| -> Option<Vec<Epoch>> {
        let window = windows.iter().find(|w| {
            w["attempt"].as_u64() == Some(u64::from(process.slot))
                && w["pid"].as_u64() == Some(u64::from(process.pid.as_u32()))
                && w["start_ticks"].as_u64() == Some(process.start_ticks)
        })?;
        if window["epochs_truncated"] == true {
            return None;
        }
        serde_json::from_value(window["epochs"].clone()).ok()
    };
    let process_of = |slot: &serde_json::Value| {
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
            .find(|receipt| {
                receipt["pid"].as_u64() == Some(u64::from(on.pid.as_u32()))
                    && receipt["start_ticks"].as_u64() == Some(on.start_ticks)
            })
            .and_then(treated_interval)
            .ok_or(WHY)?;
        let (on_range, off_range) = pair_ranges(treated, on.started_unix, off.started_unix);
        for (process, range, window) in
            [(off, off_range, &pair["off"]), (on, on_range, &pair["on"])]
        {
            let epochs = epochs_of(process).ok_or(WHY)?;
            let (buckets, max_ns, _) = population(&epochs, range.0, range.1).ok_or(WHY)?;
            if window["start_unix"].as_u64() != Some(range.0)
                || window["end_unix"].as_u64() != Some(range.1)
                || window["max_ns"].as_u64() != Some(max_ns)
                || window["histogram"] != serde_json::to_value(&buckets).map_err(|_| WHY)?
            {
                return Err(WHY);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "action_epochs_tests.rs"]
mod tests;
