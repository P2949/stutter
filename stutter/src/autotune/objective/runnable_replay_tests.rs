//! The research replay and the live objective decide identically: every case below runs through
//! `replay_pair`, which calls the production comparison, and is checked against what the controller's own
//! contract says of the same evidence.

use super::*;
use crate::metrics::LatencyHistogram;

/// A window of 2000 events: 1900 at 0.5 ms and 100 at `tail_ns` (so the 99th percentile sits in the tail's bucket),
/// of which `over` are slower than 5 ms in addition to the tail.
fn window(tail_ns: u64) -> ReplayWindow {
    let mut histogram = LatencyHistogram::new();
    for _ in 0..1_900 {
        histogram.record(500_000);
    }
    for _ in 0..100 {
        histogram.record(tail_ns);
    }
    ReplayWindow {
        histogram: histogram.snapshot(),
        max_ns: tail_ns,
        target_pid: 42,
        target_start_ticks: 100,
        frame_p99_ms: 12.0,
        scored_samples: 1_000,
        foreground_over_5ms: 10,
        thermal_degraded: false,
        thermal_throttle_count: 0,
        cpu_power_limited: false,
        gpu_power_limited: false,
    }
}

fn improved(result: &ExperimentResult) -> bool {
    matches!(result, ExperimentResult::Improved { .. })
}

fn regressed(result: &ExperimentResult) -> bool {
    matches!(result, ExperimentResult::Regressed { .. })
}

#[test]
fn an_improvement_the_resolution_can_show_is_improved_and_a_worse_tail_is_regressed() {
    // 20 ms to 1.5 ms: the candidate's bound (2 ms) clears 12.5% from the bottom of the baseline's bucket.
    let better = replay_pair(&window(20_000_000), &window(1_500_000));
    assert!(improved(&better), "{better:?}");
    let worse = replay_pair(&window(1_500_000), &window(20_000_000));
    assert!(regressed(&worse), "{worse:?}");
    // The same population is no change.
    let same = replay_pair(&window(20_000_000), &window(20_000_000));
    assert!(
        matches!(same, ExperimentResult::Inconclusive { .. }),
        "{same:?}"
    );
}

#[test]
fn a_histogram_too_coarse_for_the_claim_is_inconclusive_not_a_manufactured_improvement() {
    // 10 ms against 5 ms is a bucket step the production decision reads as 50%, but the baseline's real tail may
    // sit just above 5 ms and the candidate's just below it: that is not an improvement the data can show.
    let result = replay_pair(&window(10_000_000), &window(5_000_000));
    assert!(
        matches!(result, ExperimentResult::Inconclusive { ref reason } if reason.contains("resolution")),
        "{result:?}"
    );
}

#[test]
fn the_productions_other_rejections_apply_unchanged() {
    // A candidate with more events over 5 ms is a regression, whatever its 99th percentile.
    let mut lighter = window(20_000_000);
    let mut heavy = LatencyHistogram::new();
    for _ in 0..1_960 {
        heavy.record(500_000);
    }
    for _ in 0..40 {
        heavy.record(20_000_000);
    }
    lighter.histogram = heavy.snapshot();
    let worse_tail = replay_pair(&lighter, &window(20_000_000));
    assert!(regressed(&worse_tail), "{worse_tail:?}");
    // The foreground's own frame tail rising by more than the production slack, or its over-5-ms share rising.
    let mut slower = window(1_500_000);
    slower.frame_p99_ms = 14.5;
    assert!(regressed(&replay_pair(&window(20_000_000), &slower)));
    let mut stuttering = window(1_500_000);
    stuttering.foreground_over_5ms = 50;
    assert!(regressed(&replay_pair(&window(20_000_000), &stuttering)));
    // Thermal and power: a window that became throttled or power limited.
    for edit in [
        (|w: &mut ReplayWindow| w.thermal_degraded = true) as fn(&mut ReplayWindow),
        |w| w.thermal_throttle_count = 3,
        |w| w.cpu_power_limited = true,
        |w| w.gpu_power_limited = true,
    ] {
        let mut hot = window(1_500_000);
        edit(&mut hot);
        assert!(regressed(&replay_pair(&window(20_000_000), &hot)));
    }
}

#[test]
fn a_window_that_is_not_a_valid_population_never_decides() {
    let inconclusive = |result: ExperimentResult| !improved(&result) && !regressed(&result);
    // Too few events.
    let mut thin = window(1_500_000);
    thin.histogram = LatencyHistogram::new().snapshot();
    let mut few = LatencyHistogram::new();
    for _ in 0..10 {
        few.record(1_000_000);
    }
    thin.histogram = few.snapshot();
    thin.max_ns = 1_000_000;
    assert!(inconclusive(replay_pair(&window(20_000_000), &thin)));
    // Another process, a layout that is not the canonical one, and a maximum the buckets cannot hold.
    let mut moved = window(1_500_000);
    moved.target_start_ticks = 101;
    assert!(inconclusive(replay_pair(&window(20_000_000), &moved)));
    let mut layout = window(1_500_000);
    layout.histogram.pop();
    assert!(inconclusive(replay_pair(&window(20_000_000), &layout)));
    let mut max = window(1_500_000);
    max.max_ns = 40_000_000;
    assert!(inconclusive(replay_pair(&window(20_000_000), &max)));
}
