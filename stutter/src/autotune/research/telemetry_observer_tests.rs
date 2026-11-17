use std::time::Duration;

use super::*;

fn target(pid: u32) -> ChildTarget {
    ChildTarget {
        pid: stutter_core::ids::Pid::new(pid),
        start_ticks: 99,
        uid: 1000,
    }
}

fn source(pid: u32) -> SourceSnapshot {
    SourceSnapshot {
        producer_pid: Some(pid),
        producer_start_ticks: Some(99),
        dropped_frames: 0,
        unexplained_gaps: 0,
        healthy: true,
        competing_targets: 0,
        alignment_uncertainty_ms: 0.5,
        frame_age: Some(Duration::from_millis(10)),
    }
}

fn observe(frames: u64, snapshot: Option<SourceSnapshot>) -> AttemptTelemetry {
    let mut observer = TelemetryObserver::default();
    observer.arm(target(4242), Some(0));
    for _ in 0..frames {
        observer.count_frame();
    }
    if let Some(snapshot) = snapshot {
        observer.observe_source(snapshot);
    }
    observer.take_receipts().remove(0)
}

#[test]
fn a_bound_healthy_lossless_source_with_enough_frames_is_eligible() {
    let receipt = observe(2_400, Some(source(4242)));
    assert!(receipt.source_bound_to_game && receipt.source_healthy);
    assert_eq!(receipt.frames, 2_400);
    assert_eq!(receipt.frame_verdict, "Eligible");
    assert_eq!(receipt.clock_alignment_ns, Some(500_000));
}

#[test]
fn each_defect_names_itself_and_an_unknown_is_never_zero() {
    // No source at all: unavailable, whatever frames were counted.
    assert_eq!(observe(5_000, None).frame_verdict, "Unavailable");
    // A source bound to another process is the wrong renderer.
    assert_eq!(
        observe(5_000, Some(source(777))).frame_verdict,
        "WrongRenderer"
    );
    // Too few frames.
    assert_eq!(
        observe(100, Some(source(4242))).frame_verdict,
        "TooFewFrames"
    );
    // Loss and gaps.
    let mut lossy = source(4242);
    lossy.dropped_frames = 3;
    assert_eq!(observe(5_000, Some(lossy)).frame_verdict, "Lossy");
    let mut gapped = source(4242);
    gapped.unexplained_gaps = 1;
    assert_eq!(observe(5_000, Some(gapped)).frame_verdict, "Lossy");
    // An unhealthy or silent source is stale; competing games make the swapchain ambiguous.
    let mut sick = source(4242);
    sick.healthy = false;
    assert_eq!(observe(5_000, Some(sick)).frame_verdict, "Stale");
    let mut silent = source(4242);
    silent.frame_age = None;
    assert_eq!(observe(5_000, Some(silent)).frame_verdict, "Stale");
    let mut old = source(4242);
    old.frame_age = Some(Duration::from_secs(30));
    assert_eq!(observe(5_000, Some(old)).frame_verdict, "Stale");
    let mut shared = source(4242);
    shared.competing_targets = 1;
    assert_eq!(
        observe(5_000, Some(shared)).frame_verdict,
        "DuplicateSwapchain"
    );
    // A clock we cannot align to within the bound.
    let mut loose = source(4242);
    loose.alignment_uncertainty_ms = 10.0;
    assert_eq!(observe(5_000, Some(loose)).frame_verdict, "ClockUncertain");
    let mut unknown = source(4242);
    unknown.alignment_uncertainty_ms = f64::NAN;
    assert_eq!(
        observe(5_000, Some(unknown)).frame_verdict,
        "ClockUncertain"
    );
}

#[test]
fn a_new_process_ends_the_previous_attempt_and_frames_count_only_while_observing() {
    let mut observer = TelemetryObserver::default();
    observer.count_frame(); // nothing attested yet: not counted
    observer.arm(target(1), Some(0));
    observer.count_frame();
    observer.count_frame();
    observer.arm(target(1), Some(0)); // the same process continues
    observer.count_frame();
    observer.arm(target(2), Some(1)); // a new attempt
    observer.count_frame();
    assert!(observer.is_observing());
    let receipts = observer.take_receipts();
    assert_eq!(
        receipts
            .iter()
            .map(|r| (u32::from(r.pid), r.frames))
            .collect::<Vec<_>>(),
        [(1, 3), (2, 1)]
    );
    assert!(!observer.is_observing());
    assert!(observer.take_receipts().is_empty());
}

fn over_time(snapshots: Vec<SourceSnapshot>) -> AttemptTelemetry {
    let mut observer = TelemetryObserver::default();
    observer.arm(target(4242), Some(0));
    for snapshot in snapshots {
        for _ in 0..1_000 {
            observer.count_frame();
        }
        observer.observe_source(snapshot);
    }
    observer.take_receipts().remove(0)
}

#[test]
fn a_source_must_be_bound_and_healthy_across_the_whole_attempt_not_just_at_its_end() {
    // Healthy throughout, from one producer: eligible, and the receipt says how much was seen.
    let steady = over_time(vec![source(4242), source(4242), source(4242)]);
    assert_eq!(steady.frame_verdict, "Eligible");
    assert_eq!((steady.snapshots, steady.source_epochs), (3, 1));

    // Unhealthy part-way, healthy again at the end: the last snapshot does not undo the fault.
    let mut sick = source(4242);
    sick.healthy = false;
    let recovered = over_time(vec![source(4242), sick, source(4242)]);
    assert!(!recovered.source_healthy);
    assert_ne!(recovered.frame_verdict, "Eligible");

    // Bound to another process part-way, then to the game: not one continuous window.
    let switched = over_time(vec![source(777), source(4242), source(4242)]);
    assert!(!switched.source_bound_to_game);
    assert_eq!(switched.source_epochs, 2);
    assert_ne!(switched.frame_verdict, "Eligible");

    // The same process reappearing under a new start time is a different producer.
    let mut restarted = source(4242);
    restarted.producer_start_ticks = Some(100);
    let reborn = over_time(vec![source(4242), restarted, source(4242)]);
    assert!(!reborn.source_bound_to_game);
    assert_ne!(reborn.frame_verdict, "Eligible");
}
