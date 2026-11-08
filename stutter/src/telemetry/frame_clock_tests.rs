use std::time::Duration;

use super::*;

fn frame(epoch: u64, raw_ms: u64) -> TailedFrame {
    TailedFrame {
        epoch,
        event: FrameEvent {
            elapsed_ms: raw_ms,
            frametime_ms: 16.6,
        },
    }
}

fn at(base: Instant, ms: u64) -> Instant {
    base + Duration::from_millis(ms)
}

#[test]
fn a_game_started_long_after_the_monitor_lands_on_the_monitor_clock() {
    // The monitor has run for an hour; the game's own clock reads 10 s when first polled.
    let base = Instant::now();
    let mut clock = FrameClock::default();
    let first = clock.align(
        at(base, 0),
        Some(3_600_000),
        4096,
        vec![frame(0, 9_950), frame(0, 10_000)],
    );
    assert_eq!(first.unplaced, 0);
    // Mapped near the monitor's now, not near the game's 10 s: the offset puts 10_000 at ~3.6M.
    assert_eq!(first.frames[1].1.elapsed_ms, 3_600_000);
    assert_eq!(first.frames[0].1.elapsed_ms, 3_599_950);
    // The first poll of an epoch has no gap to bound its lag: alignment is unknown, not zero.
    assert!(clock.uncertainty_ms().is_nan());
}

#[test]
fn a_second_poll_bounds_the_alignment_by_the_gap_and_the_tightest_sample_wins() {
    let base = Instant::now();
    let mut clock = FrameClock::default();
    clock.align(at(base, 0), Some(1_000_000), 4096, vec![frame(0, 500)]);
    // 100 ms later the game clock reads 590 (true offset: frames were produced right on time).
    let second = clock.align(at(base, 100), Some(1_000_100), 4096, vec![frame(0, 595)]);
    assert_eq!(
        second.frames[0].1.elapsed_ms, 1_000_095,
        "the smaller offset is the tighter sample"
    );
    let bound = clock.uncertainty_ms();
    assert!(bound.is_finite() && bound <= 100.0, "{bound}");
    // A third poll can only tighten or keep it.
    clock.align(at(base, 200), Some(1_000_200), 4096, vec![frame(0, 695)]);
    assert!(clock.uncertainty_ms() <= bound);
}

#[test]
fn a_partial_read_a_new_epoch_and_a_contradiction_all_leave_the_alignment_unknown() {
    let base = Instant::now();
    let mut clock = FrameClock::default();
    clock.align(at(base, 0), Some(5_000), 4096, vec![frame(0, 100)]);
    clock.align(at(base, 50), Some(5_050), 4096, vec![frame(0, 150)]);
    assert!(clock.uncertainty_ms().is_finite());
    // A rotation starts a new epoch: nothing about the old offset carries over.
    let rotated = clock.align(at(base, 100), Some(5_100), 4096, vec![frame(1, 20)]);
    assert!(rotated.epoch_changed);
    assert!(clock.uncertainty_ms().is_nan());
    // A poll that filled its room read only part of the backlog: no bound is claimed from it.
    let mut partial = FrameClock::default();
    partial.align(at(base, 0), Some(5_000), 2, vec![frame(0, 1), frame(0, 2)]);
    assert!(partial.uncertainty_ms().is_nan());
    // A later poll whose offset exceeds the gap contradicts the model (a stalled clock).
    let mut odd = FrameClock::default();
    odd.align(at(base, 0), Some(5_000), 4096, vec![frame(0, 100)]);
    odd.align(at(base, 10), Some(5_010), 4096, vec![frame(0, 110)]);
    odd.align(at(base, 20), Some(5_020), 4096, vec![frame(0, 20)]);
    assert!(
        odd.uncertainty_ms().is_nan(),
        "a contradicting sample must not leave a finite bound"
    );
}

#[test]
fn without_a_monitor_time_reference_frames_are_unplaced_and_counted_not_guessed() {
    let base = Instant::now();
    let mut clock = FrameClock::default();
    let out = clock.align(at(base, 0), None, 4096, vec![frame(0, 1), frame(0, 17)]);
    assert!(out.frames.is_empty());
    assert_eq!(out.unplaced, 2);
    assert!(clock.uncertainty_ms().is_nan());
}

#[test]
fn a_batch_spanning_two_epochs_keeps_them_apart_and_only_the_current_one_is_trusted() {
    let base = Instant::now();
    let mut clock = FrameClock::default();
    clock.align(at(base, 0), Some(2_000), 4096, vec![frame(0, 10)]);
    let out = clock.align(
        at(base, 40),
        Some(2_040),
        4096,
        vec![frame(0, 45), frame(1, 5), frame(1, 21)],
    );
    assert!(out.epoch_changed);
    let epochs: Vec<u64> = out.frames.iter().map(|(epoch, _)| *epoch).collect();
    assert_eq!(epochs, [0, 1, 1]);
    assert!(
        clock.uncertainty_ms().is_nan(),
        "the new epoch's first poll has no bound"
    );
}
