use super::*;
use crate::telemetry::quality::{MAX_ALIGNMENT_UNCERTAINTY_MS, QualityDenial};

const T0: u64 = 1_000_000_000;

fn frame(seq: u64, ns: u64) -> StampedFrame {
    StampedFrame {
        seq,
        monotonic_ns: ns,
    }
}

/// Feeds `count` consecutive frames `period_ns` apart starting at sequence `from`, each seen
/// `latency_ns` after its stamp; returns the receive time of the last one.
fn feed(stream: &mut StampedStream, from: u64, count: u64, period_ns: u64, latency_ns: u64) -> u64 {
    let mut rx = 0;
    for seq in from..from + count {
        let stamp = T0 + seq * period_ns;
        rx = stamp + latency_ns;
        stream.push(frame(seq, stamp), rx).unwrap();
    }
    rx
}

fn eligible(stream: &StampedStream, now: u64) -> Result<(), QualityDenial> {
    stream
        .quality_at(now, Attribution::ValidatedDirect, true)
        .frame_primary_eligible()
}

#[test]
fn a_calibrated_low_latency_stream_has_a_bounded_uncertainty_inside_the_gate() {
    let mut stream = StampedStream::default();
    let now = feed(&mut stream, 0, 1200, 16_600_000, 150_000);
    let calibration = stream.calibration().unwrap();
    assert_eq!(calibration.frames, 1200);
    assert_eq!(calibration.max_ns, 150_000);
    assert!(calibration.uncertainty_ms() < MAX_ALIGNMENT_UNCERTAINTY_MS);
    assert_eq!(eligible(&stream, now), Ok(()));
}

#[test]
fn without_a_calibration_the_uncertainty_is_unknown_and_refused_not_zero() {
    let mut stream = StampedStream::default();
    let now = feed(
        &mut stream,
        0,
        MIN_CALIBRATION_FRAMES as u64 - 1,
        16_600_000,
        1_000,
    );
    assert!(stream.calibration().is_none());
    let quality = stream.quality_at(now, Attribution::ValidatedDirect, true);
    assert!(quality.alignment_uncertainty_ms.is_nan());
    assert_eq!(
        quality.frame_primary_eligible(),
        Err(QualityDenial::AlignmentUncertain)
    );
}

/// The defect this module was rewritten for: a stream that has already seen more frames than the
/// old fixed buffer held must still notice a later stall.
#[test]
fn a_stall_after_more_than_131072_fast_frames_still_denies_the_stream() {
    let mut stream = StampedStream::default();
    let fast = 131_072 + 5_000;
    let now = feed(&mut stream, 0, fast, 4_000_000, 80_000);
    assert_eq!(eligible(&stream, now), Ok(()));
    // One 3 ms delivery, long after the old buffer would have stopped recording.
    let stamp = T0 + fast * 4_000_000;
    stream.push(frame(fast, stamp), stamp + 3_000_000).unwrap();
    let calibration = stream.calibration().unwrap();
    assert_eq!(
        calibration.max_ns, 3_000_000,
        "the stall was not reflected in the bound"
    );
    assert_eq!(calibration.lifetime_max_ns, 3_000_000);
    assert_eq!(
        eligible(&stream, stamp + 3_000_000),
        Err(QualityDenial::AlignmentUncertain)
    );
}

#[test]
fn a_stall_denies_for_exactly_as_long_as_it_stays_in_the_window_and_then_expires() {
    let mut stream = StampedStream::default();
    feed(&mut stream, 0, 2000, 16_600_000, 100_000);
    let stamp = T0 + 2000 * 16_600_000;
    stream.push(frame(2000, stamp), stamp + 5_000_000).unwrap();
    // Still inside the window after almost a full window of fast frames.
    let mut now = feed(
        &mut stream,
        2001,
        CALIBRATION_WINDOW as u64 - 1,
        16_600_000,
        100_000,
    );
    assert_eq!(stream.calibration().unwrap().max_ns, 5_000_000);
    assert_eq!(
        eligible(&stream, now),
        Err(QualityDenial::AlignmentUncertain)
    );
    // One more frame pushes it out; trust returns only then, and the history is still reported.
    now = feed(
        &mut stream,
        2001 + CALIBRATION_WINDOW as u64 - 1,
        1,
        16_600_000,
        100_000,
    );
    let calibration = stream.calibration().unwrap();
    assert_eq!(calibration.max_ns, 100_000);
    assert_eq!(
        calibration.lifetime_max_ns, 5_000_000,
        "the stall vanished from the record"
    );
    assert_eq!(eligible(&stream, now), Ok(()));
}

#[test]
fn a_new_epoch_drops_all_trust_and_must_calibrate_afresh() {
    let mut stream = StampedStream::default();
    let now = feed(&mut stream, 0, 3000, 16_600_000, 100_000);
    assert_eq!(eligible(&stream, now), Ok(()));
    stream.begin_epoch(1, 0);
    assert_eq!(stream.epoch(), 1);
    assert_eq!(
        (
            stream.accepted(),
            stream.rejected(),
            stream.unexplained_gaps()
        ),
        (0, 0, 0)
    );
    assert!(stream.calibration().is_none());
    assert_eq!(eligible(&stream, now), Err(QualityDenial::SourceUnhealthy));
    let later = feed(&mut stream, 0, 999, 16_600_000, 100_000);
    assert!(
        stream.calibration().is_none(),
        "one frame short of a calibration"
    );
    let later = later.max(feed(&mut stream, 999, 1, 16_600_000, 100_000));
    assert_eq!(eligible(&stream, later), Ok(()));
}

#[test]
fn a_stream_that_does_not_start_at_its_epochs_first_sequence_is_missing_frames() {
    // The epoch was announced to start at 0; the first frame seen is 5.
    let mut late = StampedStream::new(7, 0);
    late.push(frame(5, T0), T0 + 10_000).unwrap();
    assert_eq!(late.unexplained_gaps(), 5);
    let now = feed(&mut late, 6, 1100, 16_600_000, 50_000);
    assert_eq!(eligible(&late, now), Err(QualityDenial::UnexplainedLoss));
    // An epoch that legitimately starts elsewhere says so.
    let mut resumed = StampedStream::new(8, 5000);
    let now = feed(&mut resumed, 5000, 1100, 16_600_000, 50_000);
    assert_eq!(resumed.unexplained_gaps(), 0);
    assert_eq!(eligible(&resumed, now), Ok(()));
}

#[test]
fn a_silent_stream_is_unhealthy_whatever_its_last_window_said() {
    let mut stream = StampedStream::default();
    let now = feed(&mut stream, 0, 1500, 16_600_000, 100_000);
    assert_eq!(eligible(&stream, now), Ok(()));
    assert_eq!(eligible(&stream, now + MAX_SILENCE_NS), Ok(()));
    assert_eq!(
        eligible(&stream, now + MAX_SILENCE_NS + 1),
        Err(QualityDenial::SourceUnhealthy)
    );
    // And a stream that never produced a frame is not alive at all.
    assert!(!StampedStream::default().alive_at(now));
}

#[test]
fn impossible_and_stale_frames_are_rejected_and_counted_not_absorbed() {
    let mut stream = StampedStream::default();
    stream.push(frame(0, 1_000), 2_000).unwrap();
    assert_eq!(
        stream.push(frame(1, 9_000), 5_000),
        Err(StampError::FromTheFuture)
    );
    assert_eq!(
        stream.push(frame(0, 3_000), 4_000),
        Err(StampError::SequenceRegress)
    );
    assert_eq!(
        stream.push(frame(1, 1_000), 4_000),
        Err(StampError::OutOfOrder)
    );
    assert_eq!(
        stream.push(frame(1, 2_000), 2_000 + MAX_FRAME_AGE_NS + 1),
        Err(StampError::Stale)
    );
    assert_eq!(stream.accepted(), 1);
    assert_eq!(stream.rejected(), 4);
    let quality = stream.quality_at(2_000, Attribution::ValidatedDirect, true);
    assert_eq!(quality.dropped_frames, 4);
    assert_eq!(
        quality.frame_primary_eligible(),
        Err(QualityDenial::UnexplainedLoss)
    );
}

#[test]
fn a_skipped_counter_is_unexplained_loss_even_when_every_delivery_is_fast() {
    let mut stream = StampedStream::default();
    for (seq, stamp) in [(0, 1_000_000), (1, 17_000_000), (5, 80_000_000)] {
        stream.push(frame(seq, stamp), stamp + 10_000).unwrap();
    }
    assert_eq!(stream.unexplained_gaps(), 3);
    assert_eq!(
        stream
            .quality_at(80_010_000, Attribution::ValidatedDirect, true)
            .unexplained_sequence_gaps,
        3
    );
}

#[test]
fn unvalidated_attribution_and_unhealthy_sources_still_deny() {
    let mut stream = StampedStream::default();
    let now = feed(&mut stream, 0, 1200, 16_600_000, 50_000);
    assert_eq!(
        stream
            .quality_at(now, Attribution::ValidatedDirect, false)
            .frame_primary_eligible(),
        Err(QualityDenial::SourceUnhealthy)
    );
    assert!(
        stream
            .quality_at(now, Attribution::ObservedUnattributed, true)
            .frame_primary_eligible()
            .is_err()
    );
}

#[test]
fn percentiles_come_from_a_bounded_histogram_and_never_exceed_the_worst_delivery() {
    let mut stream = StampedStream::default();
    // 990 fast frames at 40 us, then 10 at 900 us.
    let mut rx = 0;
    for seq in 0..1000_u64 {
        let stamp = T0 + seq * 16_600_000;
        rx = stamp + if seq < 990 { 40_000 } else { 900_000 };
        stream.push(frame(seq, stamp), rx).unwrap();
    }
    let calibration = stream.calibration().unwrap();
    assert_eq!(calibration.max_ns, 900_000);
    assert!(
        calibration.p99_ns >= 40_000 && calibration.p99_ns <= 900_000,
        "{calibration:?}"
    );
    assert!(calibration.p999_ns >= calibration.p99_ns);
    assert!(calibration.p999_ns <= calibration.lifetime_max_ns);
    assert!((calibration.mean_ns - 48_600.0).abs() < 1.0);
    let _ = rx;
}

#[test]
fn the_histogram_buckets_are_monotonic_and_cover_the_whole_range() {
    let mut last = 0;
    for ns in [
        0_u64,
        1,
        3,
        4,
        5,
        7,
        8,
        100,
        1_000,
        65_535,
        1_000_000,
        250_000_000,
        u64::MAX,
    ] {
        let bucket = bucket_of(ns);
        assert!(bucket >= last, "{ns}");
        assert!(bucket < BUCKETS);
        assert!(
            bucket_floor(bucket) <= ns,
            "{ns}: floor {}",
            bucket_floor(bucket)
        );
        last = bucket;
    }
}

/// Memory is constant and cost per frame is bounded at the highest display rates, over far more
/// frames than the old fixed buffer ever held.
#[test]
fn sustained_60_155_and_240_hz_streams_use_bounded_memory_and_time() {
    for hz in [60_u64, 155, 240] {
        let period = 1_000_000_000 / hz;
        let mut stream = StampedStream::default();
        let started = std::time::Instant::now();
        let frames = 400_000_u64;
        let mut now = 0;
        for seq in 0..frames {
            let stamp = T0 + seq * period;
            // A mild, deterministic latency wobble, so the sliding maximum really moves.
            now = stamp + 20_000 + (seq % 97) * 300;
            stream.push(frame(seq, stamp), now).unwrap();
            assert!(stream.window.len() <= CALIBRATION_WINDOW);
        }
        let elapsed = started.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "{hz} Hz: {frames} frames took {elapsed:?}"
        );
        assert_eq!(stream.accepted(), frames);
        assert_eq!(eligible(&stream, now), Ok(()));
        let calibration = stream.calibration().unwrap();
        assert_eq!(calibration.frames, CALIBRATION_WINDOW);
        assert_eq!(calibration.max_ns, 20_000 + 96 * 300);
    }
}

/// `calibration()` is called on every controller tick, so it must not scale with the stream's age.
#[test]
fn reading_the_calibration_costs_the_same_on_a_young_and_an_old_stream() {
    let mut stream = StampedStream::default();
    feed(&mut stream, 0, 150_000, 4_000_000, 60_000);
    let started = std::time::Instant::now();
    for _ in 0..20_000 {
        std::hint::black_box(stream.calibration());
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "20000 calibration reads took {elapsed:?}"
    );
}
