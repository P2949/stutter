//! A renderer-stamped frame stream with a *measured* alignment bound (opt-in prototype).
//!
//! MangoHud's CSV carries only its own elapsed time, so its rows can be placed on the controller's
//! clock only through a guessed offset (hundreds of milliseconds of uncertainty). This stream is
//! different in kind: the producer stamps each frame with `CLOCK_MONOTONIC` at the moment it
//! presents it, and pushes it to the controller over a socket. Producer and controller share that
//! clock (the caller verifies the same time namespace, as the KPI source does), so there is no
//! offset to estimate. What this module measures is how late the controller *sees* a frame compared
//! with its stamp. It does **not** measure how accurately the producer stamped the real
//! presentation event; that is the instrumentation's, and a real renderer integration must measure
//! it against a reference separately.
//!
//! The bound is never allowed to go stale. Every accepted frame updates it in constant time and
//! constant memory, and no delivery is ever discarded:
//!
//! * the alignment uncertainty is the **maximum delivery latency over the most recent
//!   [`CALIBRATION_WINDOW`] accepted frames** (a sliding maximum), so a single stall denies the
//!   stream for as long as it stays in that window and then expires explicitly, rather than
//!   lingering forever or being averaged away;
//! * fewer than [`MIN_CALIBRATION_FRAMES`] accepted frames in the epoch means no calibration, and
//!   the uncertainty is unknown (`NaN`), which the gate refuses, never zero;
//! * [`StampedStream::begin_epoch`] resets every statistic and the loss counters, so a new source
//!   epoch has to calibrate afresh;
//! * the first frame must carry the verified epoch's first sequence number: a stream that starts
//!   later is missing frames nobody explained, not a complete epoch;
//! * a stream that has gone silent for longer than [`MAX_SILENCE_NS`] is unhealthy, whatever its
//!   last window said.
//!
//! Percentiles come from a fixed 256-bucket histogram (about 19% resolution, bounded memory) and
//! describe the epoch for reports; only the windowed maximum gates.
use std::collections::VecDeque;

use serde::Deserialize;

use super::{
    frame_source::{Attribution, MeasurementStage},
    quality::StreamQuality,
};

/// Fewest accepted frames a calibration may rest on.
pub const MIN_CALIBRATION_FRAMES: usize = 1000;
/// How many of the most recent accepted frames the gating maximum covers (about 68 s at 60 Hz).
pub const CALIBRATION_WINDOW: usize = 4096;
/// A frame stamped longer ago than this is stale, not merely late.
pub const MAX_FRAME_AGE_NS: u64 = 250_000_000;
/// No accepted frame for this long and the stream is unhealthy.
pub const MAX_SILENCE_NS: u64 = 500_000_000;

const SUB_BUCKETS: u32 = 4;
const BUCKETS: usize = 64 * SUB_BUCKETS as usize;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub struct StampedFrame {
    /// The producer's own frame counter, consecutive from its epoch's first frame.
    pub seq: u64,
    /// `CLOCK_MONOTONIC` nanoseconds at the producer's present call.
    pub monotonic_ns: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StampError {
    /// The stamp is not later than the previous frame's.
    OutOfOrder,
    /// The stamp is after the controller's own reading of the same clock: impossible on one clock.
    FromTheFuture,
    /// Stamped more than [`MAX_FRAME_AGE_NS`] before it was seen.
    Stale,
    /// The counter went backwards or repeated.
    SequenceRegress,
    /// The counter is at its maximum, so no frame can follow it: not a counter a real producer
    /// reaches, and refused rather than wrapped.
    SequenceOverflow,
}

#[derive(Debug)]
pub struct StampedStream {
    epoch: u64,
    /// The sequence number the next accepted frame must carry to leave no unexplained gap.
    expected_seq: u64,
    last_stamp_ns: Option<u64>,
    last_rx_ns: Option<u64>,
    accepted: u64,
    rejected: u64,
    unexplained_gaps: u64,
    /// Sliding maximum of delivery latency over the last [`CALIBRATION_WINDOW`] accepted frames:
    /// (index of the accepted frame, latency), latencies strictly decreasing.
    window: VecDeque<(u64, u64)>,
    lifetime_max_ns: u64,
    sum_ns: u128,
    histogram: [u64; BUCKETS],
}

impl Default for StampedStream {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

/// A measured bound on delivery latency, and what it rests on.
#[derive(Clone, Debug, PartialEq)]
pub struct Calibration {
    /// Accepted frames the gating maximum covers (at most [`CALIBRATION_WINDOW`]).
    pub frames: usize,
    /// Accepted frames this epoch.
    pub accepted_total: u64,
    /// The gating maximum: worst delivery latency over the last `frames` accepted frames.
    pub max_ns: u64,
    /// Worst delivery latency this epoch, for reports (not gating).
    pub lifetime_max_ns: u64,
    /// Epoch percentiles (histogram bucket upper bounds, about 19% resolution).
    pub p99_ns: u64,
    pub p999_ns: u64,
    pub mean_ns: f64,
}

impl Calibration {
    /// The alignment uncertainty this calibration supports, in milliseconds: the worst delivery
    /// latency in the window. Frames never arrive before they are stamped, so it is one-sided.
    pub fn uncertainty_ms(&self) -> f64 {
        self.max_ns as f64 / 1_000_000.0
    }
}

fn bucket_of(latency_ns: u64) -> usize {
    if latency_ns < u64::from(SUB_BUCKETS) {
        return latency_ns as usize;
    }
    let magnitude = 63 - latency_ns.leading_zeros();
    let sub =
        (latency_ns >> (magnitude - SUB_BUCKETS.trailing_zeros())) & u64::from(SUB_BUCKETS - 1);
    ((magnitude * SUB_BUCKETS) as usize + sub as usize).min(BUCKETS - 1)
}

/// The smallest latency a bucket can hold, and so (for the next one) the upper bound of this one.
fn bucket_floor(bucket: usize) -> u64 {
    if bucket < SUB_BUCKETS as usize {
        return bucket as u64;
    }
    let magnitude = bucket as u32 / SUB_BUCKETS;
    let sub = u64::from(bucket as u32 % SUB_BUCKETS);
    (1_u64 << magnitude) + (sub << (magnitude - SUB_BUCKETS.trailing_zeros()))
}

impl StampedStream {
    /// A stream for source epoch `epoch` whose first frame must carry `first_seq`.
    pub fn new(epoch: u64, first_seq: u64) -> Self {
        Self {
            epoch,
            expected_seq: first_seq,
            last_stamp_ns: None,
            last_rx_ns: None,
            accepted: 0,
            rejected: 0,
            unexplained_gaps: 0,
            window: VecDeque::new(),
            lifetime_max_ns: 0,
            sum_ns: 0,
            histogram: [0; BUCKETS],
        }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Starts a new source epoch (a producer restart, a rotation, a re-exec, a phase change): all
    /// trust, statistics and loss counters are dropped and the next frame must carry `first_seq`.
    pub fn begin_epoch(&mut self, epoch: u64, first_seq: u64) {
        *self = Self::new(epoch, first_seq);
    }

    pub fn accepted(&self) -> u64 {
        self.accepted
    }

    pub fn rejected(&self) -> u64 {
        self.rejected
    }

    pub fn unexplained_gaps(&self) -> u64 {
        self.unexplained_gaps
    }

    /// Counts one message that could not be parsed or accepted at all (malformed, truncated,
    /// oversize or dropped before it became a frame). A boundary that is unproven is loss: the
    /// stream's quality reports it exactly as a rejected frame.
    pub fn record_invalid(&mut self) {
        self.rejected = self.rejected.saturating_add(1);
    }

    /// Validates one frame against the controller's reading of the same clock taken as it arrived,
    /// and records its delivery latency. A rejected frame is counted and changes nothing else.
    pub fn push(&mut self, frame: StampedFrame, rx_ns: u64) -> Result<u64, StampError> {
        let verdict = self.check(frame, rx_ns);
        let Ok(latency) = verdict else {
            self.rejected = self.rejected.saturating_add(1);
            return verdict;
        };
        // Every counter below the expected one was already counted; a jump forward is loss nobody
        // explained, including a first frame that is not the epoch's first. `check` has refused the
        // maximum counter, so `seq + 1` cannot overflow, and the counters saturate.
        self.unexplained_gaps = self
            .unexplained_gaps
            .saturating_add(frame.seq - self.expected_seq);
        self.expected_seq = frame.seq + 1;
        self.last_stamp_ns = Some(frame.monotonic_ns);
        self.last_rx_ns = Some(rx_ns);
        let index = self.accepted;
        self.accepted = self.accepted.saturating_add(1);
        // Sliding maximum: drop what the new latency dominates and what left the window.
        while self
            .window
            .back()
            .is_some_and(|(_, older)| *older <= latency)
        {
            self.window.pop_back();
        }
        self.window.push_back((index, latency));
        while self
            .window
            .front()
            .is_some_and(|(at, _)| at + CALIBRATION_WINDOW as u64 <= index)
        {
            self.window.pop_front();
        }
        self.lifetime_max_ns = self.lifetime_max_ns.max(latency);
        self.sum_ns = self.sum_ns.saturating_add(u128::from(latency));
        let bucket = bucket_of(latency);
        self.histogram[bucket] = self.histogram[bucket].saturating_add(1);
        Ok(latency)
    }

    fn check(&self, frame: StampedFrame, rx_ns: u64) -> Result<u64, StampError> {
        if frame.monotonic_ns > rx_ns {
            return Err(StampError::FromTheFuture);
        }
        let latency = rx_ns - frame.monotonic_ns;
        if latency > MAX_FRAME_AGE_NS {
            return Err(StampError::Stale);
        }
        if frame.seq == u64::MAX {
            return Err(StampError::SequenceOverflow);
        }
        if frame.seq < self.expected_seq {
            return Err(StampError::SequenceRegress);
        }
        if self
            .last_stamp_ns
            .is_some_and(|last| frame.monotonic_ns <= last)
        {
            return Err(StampError::OutOfOrder);
        }
        Ok(latency)
    }

    fn percentile_ns(&self, fraction: f64) -> u64 {
        let target = ((self.accepted as f64) * fraction).ceil().max(1.0) as u64;
        let mut seen = 0_u64;
        for (bucket, count) in self.histogram.iter().enumerate() {
            seen += count;
            if seen >= target {
                // Upper bound of this bucket, but never above the largest latency actually seen.
                return bucket_floor((bucket + 1).min(BUCKETS - 1)).min(self.lifetime_max_ns);
            }
        }
        self.lifetime_max_ns
    }

    /// The measured delivery bound, or `None` until at least [`MIN_CALIBRATION_FRAMES`] frames were
    /// accepted this epoch. Constant time and memory whatever the stream's age.
    pub fn calibration(&self) -> Option<Calibration> {
        if self.accepted < MIN_CALIBRATION_FRAMES as u64 {
            return None;
        }
        let max_ns = self.window.front().map(|(_, latency)| *latency)?;
        Some(Calibration {
            frames: (self.accepted as usize).min(CALIBRATION_WINDOW),
            accepted_total: self.accepted,
            max_ns,
            lifetime_max_ns: self.lifetime_max_ns,
            p99_ns: self.percentile_ns(0.99),
            p999_ns: self.percentile_ns(0.999),
            mean_ns: self.sum_ns as f64 / self.accepted as f64,
        })
    }

    /// Whether frames are still arriving: the last accepted frame is within [`MAX_SILENCE_NS`] of
    /// `now_ns` on the shared clock.
    pub fn alive_at(&self, now_ns: u64) -> bool {
        self.last_rx_ns
            .is_some_and(|last| now_ns.saturating_sub(last) <= MAX_SILENCE_NS)
    }

    /// The quality of this stream at `now_ns` for the frame-primary gate. Attribution is the caller's
    /// to establish (the producer's identity); the alignment is unknown, so refused, until measured,
    /// and a silent stream is unhealthy whatever its window last said.
    pub fn quality_at(
        &self,
        now_ns: u64,
        attribution: Attribution,
        source_healthy: bool,
    ) -> StreamQuality {
        StreamQuality {
            attribution,
            stage: MeasurementStage::ApplicationFrame,
            unexplained_sequence_gaps: self.unexplained_gaps,
            dropped_frames: self.rejected,
            alignment_uncertainty_ms: self
                .calibration()
                .map_or(f64::NAN, |calibration| calibration.uncertainty_ms()),
            source_healthy: source_healthy && self.alive_at(now_ns),
        }
    }
}

#[cfg(test)]
#[path = "stamped_frames_tests.rs"]
mod tests;
