//! A measured mapping from a game's frame clock to the controller's clock.
//!
//! MangoHud rows carry the game's own elapsed time; the rolling window is keyed on the monitor
//! session's clock. They have different origins, so a row can only join the window through a
//! mapping, and the mapping has to come with an error bound or it is a guess.
//!
//! The mapping is an offset per capture epoch. Each *complete* poll reads everything the file holds,
//! so the newest frame of a batch was produced after the previous poll (otherwise the previous poll
//! would have read it) and before this one: its lag behind the poll is at most the gap since the
//! previous poll. The smallest offset seen is the tightest sample, and a later sample bounds how
//! wrong an earlier one can be, because the lags differ by exactly the difference of the offsets.
//!
//! Anything that breaks those premises makes the alignment *unknown* (`NaN`), never zero: the first
//! poll of an epoch, a poll that only read part of a backlog, a sample that contradicts the bound,
//! and a row whose source time is absurd. And a frame mapped under one alignment is never allowed to
//! be qualified by a later, better one: whenever the offset is revised, or an unknown alignment
//! becomes known, the caller must drop the frames already in its window (`invalidate_window`).
use std::time::Instant;

use crate::{mangohud::TailedFrame, recorder::FrameEvent};

/// The largest source timestamp accepted: 366 days in milliseconds. A game's frame log does not
/// run longer, and anything beyond it is a corrupt or hostile value that must not alias a time.
pub const MAX_SOURCE_ELAPSED_MS: u64 = 366 * 24 * 60 * 60 * 1_000;

/// Offsets closer than this are the same alignment: a revision below it is jitter, and revising on
/// every millisecond of jitter would keep flushing the window.
const REVISION_SLACK_MS: i128 = 1;

#[derive(Clone, Copy, Debug)]
struct Best {
    /// Monitor-clock milliseconds minus game-clock milliseconds, at the tightest sample.
    offset_ms: i128,
    /// Upper bound on how much later than the true frame time this offset places a frame, in
    /// milliseconds; `NaN` while it cannot be bounded.
    bound_ms: f64,
}

#[derive(Debug, Default)]
pub struct FrameClock {
    epoch: Option<u64>,
    best: Option<Best>,
    last_poll: Option<Instant>,
}

/// What one poll's frames turned into.
#[derive(Debug, Default)]
pub struct Aligned {
    /// Frames mapped into the monitor clock, with the epoch each came from.
    pub frames: Vec<(u64, FrameEvent)>,
    /// Frames that could not be placed on the monitor clock (no anchor, absurd source time):
    /// lost, and counted.
    pub unplaced: u64,
    /// A frame's epoch differed from the previous one: the window must start fresh for it.
    pub epoch_changed: bool,
    /// The alignment was revised or newly became known: frames mapped earlier are not covered by
    /// it, so the window must be dropped before these frames are inserted.
    pub invalidate_window: bool,
}

impl FrameClock {
    /// The current alignment uncertainty in milliseconds; `NaN` when it is not known.
    pub fn uncertainty_ms(&self) -> f64 {
        self.best.map_or(f64::NAN, |best| best.bound_ms)
    }

    fn reset(&mut self, epoch: u64) {
        self.epoch = Some(epoch);
        self.best = None;
        self.last_poll = None;
    }

    /// Records one poll. `complete` is whether it read everything the file held. Returns whether
    /// frames mapped before this poll are no longer covered by the alignment.
    fn observe(
        &mut self,
        now: Instant,
        complete: bool,
        monitor_now_ms: u64,
        last_raw_ms: u64,
    ) -> bool {
        let gap_ms = self
            .last_poll
            .map(|previous| now.saturating_duration_since(previous).as_secs_f64() * 1_000.0);
        self.last_poll = complete.then_some(now);
        if !complete {
            // Only part of a backlog was read: its newest frame says nothing about freshness.
            let was_known = self.best.is_some_and(|best| best.bound_ms.is_finite());
            if let Some(best) = self.best.as_mut() {
                best.bound_ms = f64::NAN;
            }
            return was_known;
        }
        let candidate = i128::from(monitor_now_ms) - i128::from(last_raw_ms);
        let own_bound = gap_ms.unwrap_or(f64::NAN);
        let Some(best) = self.best else {
            self.best = Some(Best {
                offset_ms: candidate,
                bound_ms: own_bound,
            });
            return false;
        };
        if candidate < best.offset_ms - REVISION_SLACK_MS {
            // A tighter sample: the offset is revised, so everything mapped before is stale.
            self.best = Some(Best {
                offset_ms: candidate,
                bound_ms: own_bound,
            });
            return true;
        }
        // The lags differ by the offsets' difference, so this poll bounds the old sample.
        let mut bound_ms = best.bound_ms;
        if let Some(gap) = gap_ms {
            let tightened = gap - (candidate - best.offset_ms) as f64;
            bound_ms = if bound_ms.is_nan() {
                tightened
            } else {
                bound_ms.min(tightened)
            };
            if bound_ms < 0.0 {
                // The samples contradict the model (a stalled or stepped clock): unknown.
                bound_ms = f64::NAN;
            }
        }
        let was_known = best.bound_ms.is_finite();
        self.best = Some(Best {
            offset_ms: best.offset_ms,
            bound_ms,
        });
        // Newly known: frames mapped while it was unknown must not be qualified by it. Newly
        // unknown after being known: the earlier frames' bound no longer holds.
        was_known != bound_ms.is_finite()
    }

    /// Maps one poll's frames. `monitor_now_ms` is the monitor-clock time at `now` (`None` before
    /// the monitor has produced any time reference), `room` the most frames the poll could take.
    pub fn align(
        &mut self,
        now: Instant,
        monitor_now_ms: Option<u64>,
        room: usize,
        frames: Vec<TailedFrame>,
    ) -> Aligned {
        let mut out = Aligned::default();
        let complete = frames.len() < room;
        if frames.is_empty() {
            if monitor_now_ms.is_some() && self.epoch.is_some() {
                self.last_poll = Some(now);
            }
            return out;
        }
        let mut index = 0;
        while index < frames.len() {
            let epoch = frames[index].epoch;
            let end = frames[index..]
                .iter()
                .position(|frame| frame.epoch != epoch)
                .map_or(frames.len(), |len| index + len);
            if self.epoch != Some(epoch) {
                self.reset(epoch);
                out.epoch_changed = true;
            }
            let group = &frames[index..end];
            // A source time beyond any real log is corrupt: that row is lost, and it can neither
            // anchor the mapping nor be placed.
            let sound: Vec<&TailedFrame> = group
                .iter()
                .filter(|frame| frame.event.elapsed_ms <= MAX_SOURCE_ELAPSED_MS)
                .collect();
            out.unplaced += (group.len() - sound.len()) as u64;
            // Only the final group of the poll is current; an earlier one is a finished epoch and
            // has no later poll to bound it.
            let is_last = end == frames.len();
            if let (Some(monitor_now), Some(last)) = (monitor_now_ms, sound.last()) {
                let invalidated =
                    self.observe(now, complete && is_last, monitor_now, last.event.elapsed_ms);
                out.invalidate_window |= invalidated;
            }
            for frame in sound {
                match self.best {
                    Some(best) => {
                        let mapped = i128::from(frame.event.elapsed_ms) + best.offset_ms;
                        out.frames.push((
                            epoch,
                            FrameEvent {
                                elapsed_ms: u64::try_from(mapped.max(0)).unwrap_or(u64::MAX),
                                frametime_ms: frame.event.frametime_ms,
                            },
                        ));
                    }
                    None => out.unplaced += 1,
                }
            }
            index = end;
        }
        out
    }
}

#[cfg(test)]
#[path = "frame_clock_tests.rs"]
mod tests;
