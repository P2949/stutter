//! A renderer-stamped frame stream with a *measured* alignment bound (opt-in prototype).
//!
//! MangoHud's CSV carries only its own elapsed time, so its rows can be placed on the controller's
//! clock only through a guessed offset (hundreds of milliseconds of uncertainty). This stream is
//! different in kind: the producer stamps each frame with `CLOCK_MONOTONIC` at the moment it
//! presents it, and pushes it to the controller over a socket. Producer and controller share that
//! clock (the caller verifies the same time namespace, as the KPI source does), so there is no
//! offset to estimate. What remains is how late the controller *sees* a frame compared with its
//! stamp, and that is measured: every delivery's `rx - stamp` is recorded, a frame that arrives
//! before it was stamped (a clock or identity violation) is rejected, and the stream's alignment
//! uncertainty is the maximum delivery latency over a calibration of at least
//! [`MIN_CALIBRATION_FRAMES`] frames. No calibration means the uncertainty is unknown (`NaN`),
//! which the quality gate refuses, never zero. The 2 ms gate itself is unchanged.
use serde::Deserialize;

use super::{
    frame_source::{Attribution, MeasurementStage},
    quality::StreamQuality,
};

/// Fewest deliveries a calibration may rest on.
pub const MIN_CALIBRATION_FRAMES: usize = 1000;
/// Most deliveries one stream keeps, so a long run cannot grow without bound.
pub const MAX_RECORDED_DELIVERIES: usize = 1 << 17;
/// A frame stamped longer ago than this is stale, not merely late.
pub const MAX_FRAME_AGE_NS: u64 = 250_000_000;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub struct StampedFrame {
    /// The producer's own frame counter, consecutive from its first frame.
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
}

#[derive(Debug, Default)]
pub struct StampedStream {
    last: Option<StampedFrame>,
    accepted: u64,
    rejected: u64,
    /// Frames the counter says exist that never arrived.
    unexplained_gaps: u64,
    deliveries_ns: Vec<u64>,
}

/// A measured bound on delivery latency, and what it rests on.
#[derive(Clone, Debug, PartialEq)]
pub struct Calibration {
    pub frames: usize,
    pub max_ns: u64,
    pub p99_ns: u64,
    pub p999_ns: u64,
    pub mean_ns: f64,
}

impl Calibration {
    /// The alignment uncertainty this calibration supports, in milliseconds: the worst delivery
    /// latency observed. Frames never arrive before they are stamped, so it is a one-sided bound.
    pub fn uncertainty_ms(&self) -> f64 {
        self.max_ns as f64 / 1_000_000.0
    }
}

impl StampedStream {
    pub fn accepted(&self) -> u64 {
        self.accepted
    }

    pub fn rejected(&self) -> u64 {
        self.rejected
    }

    pub fn unexplained_gaps(&self) -> u64 {
        self.unexplained_gaps
    }

    /// Validates one frame against the controller's reading of the same clock taken as it arrived,
    /// and records its delivery latency. A rejected frame is counted and changes nothing else.
    pub fn push(&mut self, frame: StampedFrame, rx_ns: u64) -> Result<u64, StampError> {
        let verdict = self.check(frame, rx_ns);
        let Ok(latency) = verdict else {
            self.rejected += 1;
            return verdict;
        };
        if let Some(last) = self.last {
            // Consecutive counters are required; a jump forward is loss nobody explained.
            self.unexplained_gaps += frame.seq - last.seq - 1;
        }
        self.last = Some(frame);
        self.accepted += 1;
        if self.deliveries_ns.len() < MAX_RECORDED_DELIVERIES {
            self.deliveries_ns.push(latency);
        }
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
        if let Some(last) = self.last {
            if frame.seq <= last.seq {
                return Err(StampError::SequenceRegress);
            }
            if frame.monotonic_ns <= last.monotonic_ns {
                return Err(StampError::OutOfOrder);
            }
        }
        Ok(latency)
    }

    /// The measured delivery bound, or `None` until at least [`MIN_CALIBRATION_FRAMES`] frames
    /// were accepted.
    pub fn calibration(&self) -> Option<Calibration> {
        if self.deliveries_ns.len() < MIN_CALIBRATION_FRAMES {
            return None;
        }
        let mut sorted = self.deliveries_ns.clone();
        sorted.sort_unstable();
        let at = |fraction: f64| sorted[(((sorted.len() - 1) as f64) * fraction).round() as usize];
        Some(Calibration {
            frames: sorted.len(),
            max_ns: *sorted.last()?,
            p99_ns: at(0.99),
            p999_ns: at(0.999),
            mean_ns: sorted.iter().map(|v| *v as f64).sum::<f64>() / sorted.len() as f64,
        })
    }

    /// The quality of this stream for the frame-primary gate. Attribution is the caller's to
    /// establish (the producer's identity); the alignment is unknown, so refused, until measured.
    pub fn quality(&self, attribution: Attribution, source_healthy: bool) -> StreamQuality {
        StreamQuality {
            attribution,
            stage: MeasurementStage::ApplicationFrame,
            unexplained_sequence_gaps: self.unexplained_gaps,
            dropped_frames: self.rejected,
            alignment_uncertainty_ms: self
                .calibration()
                .map_or(f64::NAN, |calibration| calibration.uncertainty_ms()),
            source_healthy,
        }
    }
}

#[cfg(test)]
#[path = "stamped_frames_tests.rs"]
mod tests;
