//! A measured mapping from a game's frame clock to the controller's clock.
//!
//! MangoHud rows carry the game's own elapsed time; the rolling window is keyed on the monitor
//! session's clock. They have different origins, so a row can only join the window through a
//! mapping, and the mapping has to come with an error bound or it is a guess.
//!
//! The mapping is an offset per capture epoch. Each poll reads everything the file holds, so the
//! newest frame of a batch was produced *after the previous poll* (otherwise the previous poll would
//! have read it) and before this one. Its lag behind the poll is therefore at most the gap since
//! the previous poll. Taking the smallest offset seen keeps the tightest sample, and a later sample
//! bounds how wrong an earlier one can be: the lags differ by exactly the difference of their
//! offsets. A poll that read only part of the backlog, the first poll of an epoch, and any sample
//! that contradicts the bound all leave the alignment *unknown* (`NaN`), never zero.
use std::time::Instant;

use crate::{mangohud::TailedFrame, recorder::FrameEvent};

#[derive(Clone, Copy, Debug)]
struct Best {
    /// Monitor-clock milliseconds minus game-clock milliseconds, at the tightest sample.
    offset_ms: i64,
    /// Upper bound on how much later than the true frame time this offset places a frame.
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
    /// Frames that could not be placed on the monitor clock (no anchor yet): lost, and counted.
    pub unplaced: u64,
    /// A frame's epoch differed from the previous one: the window must start fresh for it.
    pub epoch_changed: bool,
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

    /// Records a poll that read `complete` (all there was) frames and whose newest frame in
    /// `epoch` had game-clock time `last_raw_ms`, at monitor-clock time `monitor_now_ms`.
    fn observe(&mut self, now: Instant, complete: bool, monitor_now_ms: u64, last_raw_ms: u64) {
        let gap_ms = self
            .last_poll
            .map(|previous| now.saturating_duration_since(previous).as_secs_f64() * 1_000.0);
        self.last_poll = complete.then_some(now);
        if !complete {
            return;
        }
        let candidate = monitor_now_ms as i64 - last_raw_ms as i64;
        match self.best {
            None => {
                self.best = Some(Best {
                    offset_ms: candidate,
                    bound_ms: gap_ms.unwrap_or(f64::NAN),
                });
            }
            Some(best) if candidate < best.offset_ms => {
                // A smaller offset is a tighter sample; its own lag is bounded by this poll's gap.
                self.best = Some(Best {
                    offset_ms: candidate,
                    bound_ms: gap_ms.unwrap_or(f64::NAN),
                });
            }
            Some(best) => {
                // The lags differ by the offsets' difference, so this poll bounds the old sample.
                if let Some(gap) = gap_ms {
                    let tightened = gap - (candidate - best.offset_ms) as f64;
                    let bound = if best.bound_ms.is_nan() {
                        tightened
                    } else {
                        best.bound_ms.min(tightened)
                    };
                    // A negative bound contradicts the model (clock trouble): alignment is unknown.
                    self.best = Some(Best {
                        offset_ms: best.offset_ms,
                        bound_ms: if bound < 0.0 { f64::NAN } else { bound },
                    });
                }
            }
        }
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
            if let (Some(_), true) = (monitor_now_ms, self.epoch.is_some()) {
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
            // Only the final group of the poll is current; an earlier group is a finished epoch.
            let is_last = end == frames.len();
            let last_raw = group.last().map_or(0, |frame| frame.event.elapsed_ms);
            match monitor_now_ms {
                Some(monitor_now) if is_last => {
                    self.observe(now, complete, monitor_now, last_raw);
                }
                Some(monitor_now) => {
                    // A superseded epoch has no later poll to bound it; place it by this poll and
                    // leave the alignment unknown.
                    self.observe(now, false, monitor_now, last_raw);
                }
                None => {}
            }
            for frame in group {
                match self.best {
                    Some(best) => {
                        let mapped = (frame.event.elapsed_ms as i64 + best.offset_ms).max(0) as u64;
                        out.frames.push((
                            epoch,
                            FrameEvent {
                                elapsed_ms: mapped,
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
