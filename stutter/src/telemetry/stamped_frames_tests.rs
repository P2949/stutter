use super::*;
use crate::telemetry::quality::{MAX_ALIGNMENT_UNCERTAINTY_MS, QualityDenial};

fn frame(seq: u64, ns: u64) -> StampedFrame {
    StampedFrame {
        seq,
        monotonic_ns: ns,
    }
}

/// `count` consecutive frames 16.6 ms apart, each seen `latency_ns` after its stamp.
fn feed(stream: &mut StampedStream, count: u64, latency_ns: u64) {
    for seq in 0..count {
        let stamp = 1_000_000_000 + seq * 16_600_000;
        stream.push(frame(seq, stamp), stamp + latency_ns).unwrap();
    }
}

#[test]
fn a_calibrated_low_latency_stream_has_a_bounded_uncertainty_inside_the_gate() {
    let mut stream = StampedStream::default();
    feed(&mut stream, 1200, 150_000);
    let calibration = stream.calibration().unwrap();
    assert_eq!(calibration.frames, 1200);
    assert_eq!(calibration.max_ns, 150_000);
    assert!(calibration.uncertainty_ms() < MAX_ALIGNMENT_UNCERTAINTY_MS);
    assert_eq!(
        stream
            .quality(Attribution::ValidatedDirect, true)
            .frame_primary_eligible(),
        Ok(())
    );
}

#[test]
fn without_a_calibration_the_uncertainty_is_unknown_and_refused_not_zero() {
    let mut stream = StampedStream::default();
    feed(&mut stream, MIN_CALIBRATION_FRAMES as u64 - 1, 1_000);
    assert!(stream.calibration().is_none());
    let quality = stream.quality(Attribution::ValidatedDirect, true);
    assert!(quality.alignment_uncertainty_ms.is_nan());
    assert_eq!(
        quality.frame_primary_eligible(),
        Err(QualityDenial::AlignmentUncertain)
    );
}

#[test]
fn one_slow_delivery_over_the_gate_denies_the_whole_stream() {
    let mut stream = StampedStream::default();
    feed(&mut stream, 1500, 100_000);
    // A single 3 ms delivery in the calibration is the stream's bound, not an outlier to drop.
    let stamp = 1_000_000_000 + 1500 * 16_600_000;
    stream.push(frame(1500, stamp), stamp + 3_000_000).unwrap();
    assert_eq!(stream.calibration().unwrap().max_ns, 3_000_000);
    assert_eq!(
        stream
            .quality(Attribution::ValidatedDirect, true)
            .frame_primary_eligible(),
        Err(QualityDenial::AlignmentUncertain)
    );
}

#[test]
fn impossible_and_stale_frames_are_rejected_and_counted_not_absorbed() {
    let mut stream = StampedStream::default();
    stream.push(frame(0, 1_000), 2_000).unwrap();
    // Stamped after the controller's own reading of the same clock.
    assert_eq!(
        stream.push(frame(1, 9_000), 5_000),
        Err(StampError::FromTheFuture)
    );
    // Counter repeats, goes backwards, or the stamp does not advance.
    assert_eq!(
        stream.push(frame(0, 3_000), 4_000),
        Err(StampError::SequenceRegress)
    );
    assert_eq!(
        stream.push(frame(1, 1_000), 4_000),
        Err(StampError::OutOfOrder)
    );
    // Seen far too long after it was stamped.
    assert_eq!(
        stream.push(frame(1, 2_000), 2_000 + MAX_FRAME_AGE_NS + 1),
        Err(StampError::Stale)
    );
    assert_eq!(stream.accepted(), 1);
    assert_eq!(stream.rejected(), 4);
    let quality = stream.quality(Attribution::ValidatedDirect, true);
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
            .quality(Attribution::ValidatedDirect, true)
            .unexplained_sequence_gaps,
        3
    );
}

#[test]
fn the_bound_is_never_set_by_a_guess_unvalidated_attribution_and_unhealthy_sources_still_deny() {
    let mut stream = StampedStream::default();
    feed(&mut stream, 1200, 50_000);
    assert_eq!(
        stream
            .quality(Attribution::ValidatedDirect, false)
            .frame_primary_eligible(),
        Err(QualityDenial::SourceUnhealthy)
    );
    let unattributed = stream.quality(Attribution::ObservedUnattributed, true);
    assert!(unattributed.frame_primary_eligible().is_err());
}
