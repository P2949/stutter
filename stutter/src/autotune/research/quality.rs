//! Source-quality verdicts for one attempt's captured telemetry (PLAN P4). Missing data is
//! `Unknown`, never zero; any single defect makes the frame source ineligible without
//! invalidating the native benchmark or a legal runnable-delay study.

/// Direct frame-primary gate: at least this many valid frames per window.
pub const MIN_DIRECT_FRAMES: u64 = 2_400;
/// Direct frame-primary gate: authenticated clock alignment bound, in nanoseconds.
pub const MAX_CLOCK_ALIGNMENT_NS: u64 = 2_000_000;
/// Supported runnable objective floor per comparison window.
pub const MIN_RUNNABLE_EVENTS: u64 = 1_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrameSourceFacts {
    pub producers: u32,
    pub swapchains: u32,
    pub renderer_matches_game: bool,
    pub valid_frames: Option<u64>,
    pub sequence_gaps: u64,
    pub dropped: Option<u64>,
    pub stale: bool,
    pub clock_alignment_ns: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameVerdict {
    Eligible,
    Unavailable,
    DuplicateProducer,
    DuplicateSwapchain,
    WrongRenderer,
    Lossy,
    Stale,
    TooFewFrames,
    ClockUncertain,
}

pub fn judge_frame_source(f: &FrameSourceFacts) -> FrameVerdict {
    if f.producers == 0 || f.swapchains == 0 {
        return FrameVerdict::Unavailable;
    }
    if f.producers > 1 {
        return FrameVerdict::DuplicateProducer;
    }
    if f.swapchains > 1 {
        return FrameVerdict::DuplicateSwapchain;
    }
    if !f.renderer_matches_game {
        return FrameVerdict::WrongRenderer;
    }
    if f.stale {
        return FrameVerdict::Stale;
    }
    // Unknown loss counters are not evidence of zero loss.
    if f.sequence_gaps > 0 || f.dropped != Some(0) {
        return FrameVerdict::Lossy;
    }
    match f.valid_frames {
        Some(n) if n >= MIN_DIRECT_FRAMES => {}
        _ => return FrameVerdict::TooFewFrames,
    }
    match f.clock_alignment_ns {
        Some(ns) if ns <= MAX_CLOCK_ALIGNMENT_NS => FrameVerdict::Eligible,
        _ => FrameVerdict::ClockUncertain,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunnableVerdict {
    Eligible,
    Unknown,
    BelowSampleFloor,
    EventsLost,
    TargetChanged,
}

pub fn judge_runnable(
    events: Option<u64>,
    lost: Option<u64>,
    same_target: bool,
) -> RunnableVerdict {
    if !same_target {
        return RunnableVerdict::TargetChanged;
    }
    let Some(events) = events else {
        return RunnableVerdict::Unknown;
    };
    if lost != Some(0) {
        return RunnableVerdict::EventsLost;
    }
    if events < MIN_RUNNABLE_EVENTS {
        RunnableVerdict::BelowSampleFloor
    } else {
        RunnableVerdict::Eligible
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good() -> FrameSourceFacts {
        FrameSourceFacts {
            producers: 1,
            swapchains: 1,
            renderer_matches_game: true,
            valid_frames: Some(2_400),
            sequence_gaps: 0,
            dropped: Some(0),
            stale: false,
            clock_alignment_ns: Some(2_000_000),
        }
    }

    #[test]
    fn boundary_values_are_eligible() {
        assert_eq!(judge_frame_source(&good()), FrameVerdict::Eligible);
    }

    #[test]
    fn each_defect_has_its_own_verdict() {
        let cases: [(fn(&mut FrameSourceFacts), FrameVerdict); 9] = [
            (|f| f.producers = 0, FrameVerdict::Unavailable),
            (|f| f.producers = 2, FrameVerdict::DuplicateProducer),
            (|f| f.swapchains = 2, FrameVerdict::DuplicateSwapchain),
            (
                |f| f.renderer_matches_game = false,
                FrameVerdict::WrongRenderer,
            ),
            (|f| f.stale = true, FrameVerdict::Stale),
            (|f| f.sequence_gaps = 1, FrameVerdict::Lossy),
            (|f| f.dropped = None, FrameVerdict::Lossy),
            (|f| f.valid_frames = Some(2_399), FrameVerdict::TooFewFrames),
            (
                |f| f.clock_alignment_ns = Some(2_000_001),
                FrameVerdict::ClockUncertain,
            ),
        ];
        for (mutate, want) in cases {
            let mut f = good();
            mutate(&mut f);
            assert_eq!(judge_frame_source(&f), want);
        }
        let mut f = good();
        f.clock_alignment_ns = None;
        assert_eq!(judge_frame_source(&f), FrameVerdict::ClockUncertain);
    }

    #[test]
    fn runnable_unknown_is_not_zero() {
        assert_eq!(
            judge_runnable(None, Some(0), true),
            RunnableVerdict::Unknown
        );
        assert_eq!(
            judge_runnable(Some(999), Some(0), true),
            RunnableVerdict::BelowSampleFloor
        );
        assert_eq!(
            judge_runnable(Some(1_000), Some(0), true),
            RunnableVerdict::Eligible
        );
        assert_eq!(
            judge_runnable(Some(5_000), None, true),
            RunnableVerdict::EventsLost
        );
        assert_eq!(
            judge_runnable(Some(5_000), Some(3), true),
            RunnableVerdict::EventsLost
        );
        assert_eq!(
            judge_runnable(Some(5_000), Some(0), false),
            RunnableVerdict::TargetChanged
        );
    }
}
