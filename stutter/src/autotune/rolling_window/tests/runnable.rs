use super::*;
use crate::metrics::LatencyHistogram;

fn record(elapsed_ms: u64, class: TaskClass, latency_ns: u64, samples: u64) -> IntervalRecord {
    let mut histogram = LatencyHistogram::new();
    for _ in 0..samples {
        histogram.record(latency_ns);
    }
    IntervalRecord {
        elapsed_ms,
        task: 42,
        process_pid: Some(42),
        class,
        samples,
        histogram: histogram.snapshot(),
        max_ns: latency_ns,
        over_1ms: if latency_ns > 1_000_000 { samples } else { 0 },
        over_2ms: if latency_ns > 2_000_000 { samples } else { 0 },
        over_5ms: if latency_ns > 5_000_000 { samples } else { 0 },
        ..IntervalRecord::default()
    }
}

#[test]
fn runnable_percentiles_merge_populations_instead_of_averaging_interval_p99s() {
    let mut window = RollingWindow::default();
    let mut large = record(1_000, TaskClass::Game, 1_000_000, 10_000);
    large.p99_ns = 1_000_000;
    large.truncated_samples = 9_000;
    large.stored_samples = 1_000;
    let mut small = record(2_000, TaskClass::GameWorkerThread, 20_000_000, 4);
    small.p99_ns = 20_000_000;
    window.push_intervals([large, small]);
    let signals = window.objective_signals_for_target(42, &std::collections::BTreeSet::from([42]));
    assert_eq!(signals.runnable_samples, Some(10_004));
    assert_eq!(signals.runnable_latency_p50_ns, Some(1_000_000));
    assert_eq!(signals.runnable_latency_p95_ns, Some(1_000_000));
    assert_eq!(signals.runnable_latency_p99_ns, Some(1_000_000));
    assert_eq!(signals.runnable_latency_p999_ns, Some(1_000_000));
    assert_eq!(signals.runnable_latency_max_ns, Some(20_000_000));
    assert_eq!(signals.runnable_over_1ms, Some(4));
    assert_eq!(signals.runnable_over_2ms, Some(4));
    assert_eq!(signals.runnable_over_5ms, Some(4));
    assert_eq!(
        signals.signal_quality.runnable_latency,
        ObjectiveSignalQuality::Direct
    );
}

#[test]
fn runnable_includes_game_classes_but_excludes_compositors_and_unrelated_work() {
    let mut window = RollingWindow::default();
    for class in [
        TaskClass::Game,
        TaskClass::GameRenderThread,
        TaskClass::GameWorkerThread,
        TaskClass::GameHelper,
        TaskClass::WineServer,
    ] {
        window.push_interval(record(1_000, class, 1_000_000, 100));
    }
    for class in [
        TaskClass::GameScope,
        TaskClass::Compositor,
        TaskClass::Compiler,
        TaskClass::Helper,
        TaskClass::Unknown,
        TaskClass::SteamRuntime,
    ] {
        window.push_interval(record(1_000, class, 100_000_000, 1_000));
    }
    let signals = window.objective_signals_for_target(42, &std::collections::BTreeSet::from([42]));
    assert_eq!(signals.runnable_samples, Some(500));
    assert_eq!(signals.runnable_latency_p99_ns, Some(1_000_000));
    assert_eq!(signals.runnable_latency_p999_ns, None);
    assert_eq!(signals.runnable_over_5ms, Some(0));
}

#[test]
fn runnable_tail_thresholds_are_strict_and_open_bucket_uses_observed_maximum() {
    let mut window = RollingWindow::default();
    for latency in [1_000_000, 2_000_000, 5_000_000, 100_000_000] {
        window.push_interval(record(1_000, TaskClass::Game, latency, 1_000));
    }
    let signals = window.objective_signals_for_target(42, &std::collections::BTreeSet::from([42]));
    assert_eq!(signals.runnable_over_1ms, Some(3_000));
    assert_eq!(signals.runnable_over_2ms, Some(2_000));
    assert_eq!(signals.runnable_over_5ms, Some(1_000));
    assert_eq!(signals.runnable_latency_p99_ns, Some(100_000_000));
}

#[test]
fn runnable_histogram_aggregation_tracks_window_expiry() {
    let mut window = RollingWindow::new(Duration::from_secs(1));
    window.push_interval(record(1_000, TaskClass::Game, 20_000_000, 1_000));
    window.push_interval(record(3_000, TaskClass::Game, 1_000_000, 2_000));
    let signals = window.objective_signals_for_target(42, &std::collections::BTreeSet::from([42]));
    assert_eq!(signals.runnable_samples, Some(2_000));
    assert_eq!(signals.runnable_latency_p99_ns, Some(1_000_000));
    assert_eq!(signals.runnable_latency_max_ns, Some(1_000_000));
    window.clear();
    assert_eq!(
        window
            .objective_signals_for_target(42, &std::collections::BTreeSet::from([42]))
            .runnable_samples,
        None
    );
}

#[test]
fn missing_or_corrupt_histogram_invalidates_the_whole_population() {
    for case in 0..7 {
        let mut window = RollingWindow::default();
        window.push_interval(record(1_000, TaskClass::Game, 1_000_000, 10_000));
        let mut bad = record(2_000, TaskClass::GameWorkerThread, 20_000_000, 4);
        match case {
            0 => bad.histogram.clear(),
            1 => bad.samples = 5,
            2 => bad.histogram[0].upper_bound_ns = Some(999),
            3 => bad.over_5ms = 0,
            4 => bad.max_ns = 1,
            5 => {
                bad.histogram[0].count = u64::MAX;
                bad.histogram[1].count = 1;
            }
            _ => {
                bad.samples = 0;
            }
        }
        window.push_interval(bad);
        let signals =
            window.objective_signals_for_target(42, &std::collections::BTreeSet::from([42]));
        assert_eq!(signals.runnable_samples, None, "case {case}");
        assert_eq!(signals.runnable_latency_p99_ns, None, "case {case}");
        assert_eq!(
            signals.signal_quality.runnable_latency,
            ObjectiveSignalQuality::Missing
        );
        assert!(signals.runnable_latency_unavailable_reason.is_some());
    }
}

#[test]
fn runnable_window_population_overflow_is_missing_evidence() {
    let mut huge = record(1_000, TaskClass::Game, 1_000, 1);
    huge.histogram[0].count = u64::MAX;
    huge.samples = u64::MAX;
    let mut window = RollingWindow::default();
    window.push_interval(huge);
    window.push_interval(record(2_000, TaskClass::Game, 1_000, 1));
    let signals = window.objective_signals_for_target(42, &std::collections::BTreeSet::from([42]));
    assert_eq!(
        signals.signal_quality.runnable_latency,
        ObjectiveSignalQuality::Missing
    );
    assert!(
        signals
            .runnable_latency_unavailable_reason
            .unwrap()
            .contains("overflow")
    );
}
