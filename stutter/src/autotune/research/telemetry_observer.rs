//! What the resident controller itself saw of the game during each research attempt (PLAN P4), taken
//! from its own frame source with no second monitor. While a research attempt runs, the observer
//! counts the frame events the runtime received and snapshots the frame source's own status (the
//! producer it is bound to, its loss and gap counters, health and clock alignment). At the end of
//! the attempt this becomes a receipt that states, for that attempt, whether the source was bound to
//! the attested game process and what `quality::judge_frame_source` says of it. Nothing here
//! measures anything new or qualifies a source: a field the runtime cannot know is left unknown, and
//! an unknown is never zero.

use std::sync::{Arc, Mutex};

use serde::Serialize;

use super::{
    action_child::ChildTarget,
    quality::{FrameSourceFacts, FrameVerdict, judge_frame_source},
};
use crate::telemetry::live_source::FrameSourceStatus;

/// A frame older than this at the end of the attempt makes the source stale.
const STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(5);

/// The frame source's own status at one moment, reduced to what a receipt needs.
#[derive(Clone, Debug, PartialEq)]
pub struct SourceSnapshot {
    pub producer_pid: Option<u32>,
    pub producer_start_ticks: Option<u64>,
    pub dropped_frames: u64,
    pub unexplained_gaps: u64,
    pub healthy: bool,
    pub competing_targets: usize,
    pub alignment_uncertainty_ms: f64,
    /// Time since the source last produced a frame, when it ever did.
    pub frame_age: Option<std::time::Duration>,
}

impl SourceSnapshot {
    pub fn of(status: &FrameSourceStatus, now: std::time::Instant) -> Self {
        Self {
            producer_pid: status.claim.producer_pid,
            producer_start_ticks: status.claim.producer_start_ticks,
            dropped_frames: status.dropped_frames,
            unexplained_gaps: status.unexplained_gaps,
            healthy: status.healthy,
            competing_targets: status.competing_targets,
            alignment_uncertainty_ms: status.alignment_uncertainty_ms,
            frame_age: status
                .last_frame_at
                .map(|at| now.saturating_duration_since(at)),
        }
    }
}

/// One attempt's receipt.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AttemptTelemetry {
    /// The journal slot this receipt belongs to; a receipt without one cannot be matched to an attempt.
    pub attempt: Option<u32>,
    pub pid: stutter_core::ids::Pid,
    pub start_ticks: u64,
    pub uid: u32,
    /// Frame events the resident runtime received while this attempt's game process was attested.
    pub frames: u64,
    /// The frame source was bound to this exact process (pid and start time), not merely a game.
    pub source_bound_to_game: bool,
    pub dropped_frames: u64,
    pub unexplained_gaps: u64,
    pub source_healthy: bool,
    /// Source snapshots taken over the attempt and the distinct producers they reported; bound and
    /// healthy mean so in every one of them, from exactly one producer.
    pub snapshots: u64,
    pub source_epochs: u32,
    /// Frames received per whole unix second `[second, count]`, for counting the ones that fall inside
    /// the native benchmark window; absent buckets mean none.
    pub frame_seconds: Vec<(u64, u32)>,
    pub frame_seconds_truncated: bool,
    pub competing_targets: usize,
    pub clock_alignment_ns: Option<u64>,
    /// `Eligible` only when `quality::judge_frame_source` accepts the source; otherwise why not.
    pub frame_verdict: &'static str,
}

fn verdict_name(verdict: FrameVerdict) -> &'static str {
    match verdict {
        FrameVerdict::Eligible => "Eligible",
        FrameVerdict::Unavailable => "Unavailable",
        FrameVerdict::DuplicateProducer => "DuplicateProducer",
        FrameVerdict::DuplicateSwapchain => "DuplicateSwapchain",
        FrameVerdict::WrongRenderer => "WrongRenderer",
        FrameVerdict::Lossy => "Lossy",
        FrameVerdict::Stale => "Stale",
        FrameVerdict::TooFewFrames => "TooFewFrames",
        FrameVerdict::ClockUncertain => "ClockUncertain",
    }
}

/// Whether a snapshot's producer is this exact process (pid and start time).
fn snapshot_is_bound(source: &SourceSnapshot, target: &ChildTarget) -> bool {
    source.producer_pid == Some(u32::from(target.pid))
        && source.producer_start_ticks == Some(target.start_ticks)
}

#[derive(Debug)]
struct Running {
    target: ChildTarget,
    /// The journal slot this attempt is (the host's launched attempt index), when known.
    attempt: Option<u32>,
    frames: u64,
    source: Option<SourceSnapshot>,
    /// How many source snapshots were taken over the attempt.
    snapshots: u64,
    /// Some snapshot showed an unhealthy source: a healthy last snapshot does not undo it.
    unhealthy_seen: bool,
    /// Some snapshot showed a source not bound to this process.
    unbound_seen: bool,
    /// Every distinct producer identity the source reported; more than one is a source change.
    producers: Vec<(Option<u32>, Option<u64>)>,
    /// Frames received per whole unix second, so a verifier can count only the ones inside the
    /// native benchmark window (a menu or loading screen is not a measurement).
    frame_seconds: std::collections::BTreeMap<u64, u32>,
    /// More distinct seconds were seen than are kept; the buckets then cannot show a window count.
    seconds_truncated: bool,
}

/// An attempt long enough to fill this many distinct seconds is far past any research attempt.
const MAX_FRAME_SECONDS: usize = 4096;

impl Running {
    fn receipt(&self) -> AttemptTelemetry {
        let source = self.source.as_ref();
        // Bound and healthy over the whole interval, not merely in the last snapshot, and from one
        // producer throughout: a source that changed or failed part-way is not one measurement window.
        let continuous = !self.unbound_seen && self.producers.len() <= 1;
        let bound =
            continuous && source.is_some_and(|source| snapshot_is_bound(source, &self.target));
        let healthy = !self.unhealthy_seen && source.is_some_and(|source| source.healthy);
        let clock = source.and_then(|source| {
            let ns = source.alignment_uncertainty_ms * 1_000_000.0;
            (ns.is_finite() && ns >= 0.0).then_some(ns as u64)
        });
        // The runtime knows one producer per source, whether it is the game, and how many other
        // candidate games compete for it; competing candidates are the proxy for an ambiguous
        // swapchain. Counters it cannot read stay unknown.
        let facts = FrameSourceFacts {
            producers: u32::from(source.is_some()),
            swapchains: source.map_or(0, |source| 1 + source.competing_targets as u32),
            renderer_matches_game: bound,
            valid_frames: Some(self.frames),
            sequence_gaps: source.map_or(0, |source| source.unexplained_gaps),
            dropped: source.map(|source| source.dropped_frames),
            stale: !healthy
                || source.is_none_or(|source| source.frame_age.is_none_or(|age| age > STALE_AFTER)),
            clock_alignment_ns: clock,
        };
        AttemptTelemetry {
            attempt: self.attempt,
            pid: self.target.pid,
            start_ticks: self.target.start_ticks,
            uid: self.target.uid,
            frames: self.frames,
            source_bound_to_game: bound,
            dropped_frames: source.map_or(0, |source| source.dropped_frames),
            unexplained_gaps: source.map_or(0, |source| source.unexplained_gaps),
            source_healthy: healthy,
            snapshots: self.snapshots,
            source_epochs: self.producers.len() as u32,
            frame_seconds: self
                .frame_seconds
                .iter()
                .map(|(second, count)| (*second, *count))
                .collect(),
            frame_seconds_truncated: self.seconds_truncated,
            competing_targets: source.map_or(0, |source| source.competing_targets),
            clock_alignment_ns: clock,
            frame_verdict: verdict_name(judge_frame_source(&facts)),
        }
    }
}

/// Shared between the runtime (which reports events and the frame source) and the funnel (which
/// opens and closes attempts and keeps the receipts).
pub type SharedTelemetry = Arc<Mutex<TelemetryObserver>>;

#[derive(Debug, Default)]
pub struct TelemetryObserver {
    running: Option<Running>,
    finished: Vec<AttemptTelemetry>,
}

impl TelemetryObserver {
    /// A research attempt's game process is attested: start (or continue) observing it. A different
    /// process ends the previous attempt first.
    pub fn arm(&mut self, target: ChildTarget, attempt: Option<u32>) {
        if self
            .running
            .as_ref()
            .is_some_and(|running| running.target == target && running.attempt == attempt)
        {
            return;
        }
        self.end_attempt();
        self.running = Some(Running {
            target,
            attempt,
            frames: 0,
            source: None,
            snapshots: 0,
            unhealthy_seen: false,
            unbound_seen: false,
            producers: Vec::new(),
            frame_seconds: std::collections::BTreeMap::new(),
            seconds_truncated: false,
        });
    }

    /// A frame event reached the runtime.
    pub fn count_frame(&mut self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        self.count_frame_at(now);
    }

    /// As `count_frame`, for a frame received during unix second `second`.
    pub fn count_frame_at(&mut self, second: u64) {
        if let Some(running) = self.running.as_mut() {
            running.frames = running.frames.saturating_add(1);
            if let Some(bucket) = running.frame_seconds.get_mut(&second) {
                *bucket = bucket.saturating_add(1);
            } else if running.frame_seconds.len() < MAX_FRAME_SECONDS {
                running.frame_seconds.insert(second, 1);
            } else {
                running.seconds_truncated = true;
            }
        }
    }

    /// The frame source's status after an event.
    pub fn observe_source(&mut self, snapshot: SourceSnapshot) {
        if let Some(running) = self.running.as_mut() {
            running.snapshots = running.snapshots.saturating_add(1);
            running.unhealthy_seen |= !snapshot.healthy;
            running.unbound_seen |= !snapshot_is_bound(&snapshot, &running.target);
            let producer = (snapshot.producer_pid, snapshot.producer_start_ticks);
            if !running.producers.contains(&producer) && running.producers.len() < 8 {
                running.producers.push(producer);
            }
            running.source = Some(snapshot);
        }
    }

    /// The attempt ended (or its process is gone): keep its receipt.
    pub fn end_attempt(&mut self) {
        if let Some(running) = self.running.take() {
            self.finished.push(running.receipt());
        }
    }

    /// Every receipt so far, ending the running attempt, and start afresh.
    pub fn take_receipts(&mut self) -> Vec<AttemptTelemetry> {
        self.end_attempt();
        std::mem::take(&mut self.finished)
    }

    pub fn is_observing(&self) -> bool {
        self.running.is_some()
    }
}

#[cfg(test)]
#[path = "telemetry_observer_tests.rs"]
mod tests;
