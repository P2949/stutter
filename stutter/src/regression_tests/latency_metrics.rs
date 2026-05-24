//! Regression coverage for spike buffering, latency histograms, and saturation helpers.

use super::{support::*, *};

#[test]
fn spike_events_capture_only_threshold_crossing_events() {
    let config = test_config(vec![7], vec![], None);
    let monitor_config = config;
    let active_targets = BTreeMap::from([(
        7,
        task_info(7, 77, "KingdomCome.exe", "RenderThread", TaskClass::Game),
    )]);
    let spike_events = SpikeEventBuffer::default();

    let stats_by_task = BTreeMap::<u32, crate::metrics::TaskStats>::new();
    let below_threshold = scheduler_event_with_latency(7, "RenderThread", 999_999);
    let mut tasks = tasks::TaskTracker {
        active_targets,
        stats_by_task,
        ..Default::default()
    };
    let mut recorder = recorder::LiveRecorder {
        buffers: recorder::LiveBuffers {
            spike_events: Some(spike_events),
            ..Default::default()
        },
        ..Default::default()
    };

    events::handle_event(events::HandleEventInput {
        event: &below_threshold,
        config: &monitor_config,
        started: Instant::now(),
        tasks: &mut tasks,
        monotonic_start_ns: Some(100),
        recorder: &mut recorder,
        diagnostics: Default::default(),
    });
    assert!(
        recorder
            .buffers
            .spike_events
            .as_ref()
            .unwrap()
            .as_slice()
            .is_empty()
    );

    let at_threshold = scheduler_event_with_latency(7, "RenderThread", 1_000_000);
    events::handle_event(events::HandleEventInput {
        event: &at_threshold,
        config: &monitor_config,
        started: Instant::now(),
        tasks: &mut tasks,
        monotonic_start_ns: Some(100),
        recorder: &mut recorder,
        diagnostics: Default::default(),
    });

    let spike_events_slice = recorder.buffers.spike_events.as_ref().unwrap().as_slice();
    assert_eq!(spike_events_slice.len(), 1);
    let spike = &spike_events_slice[0];
    assert_eq!(spike.task.as_u32(), 7);
    assert!(spike.active);
    assert_eq!(spike.class, TaskClass::Game);
    assert_eq!(spike.process_pid.map(|pid| pid.as_u32()), Some(77));
    assert_eq!(spike.process_comm, "KingdomCome.exe");
    assert_eq!(spike.comm, "RenderThread");
    assert_eq!(spike.cpu, 0);
    assert_eq!(spike.prio, 120);
    assert_eq!(spike.latency_ns, 1_000_000);
    assert_eq!(spike.wakeup_ns, 100);
    assert_eq!(spike.switch_ns, 1_000_100);
    assert_eq!(spike.elapsed_ms, Some(1));
}

#[test]
fn spike_event_fault_deltas_are_captured_correctly() {
    let config = test_config(vec![7], vec![], None);
    let monitor_config = config;
    let active_targets = BTreeMap::from([(
        7,
        task_info(7, 77, "KingdomCome.exe", "RenderThread", TaskClass::Game),
    )]);
    let spike_events = SpikeEventBuffer::default();

    // First event establishes baseline faults
    let mut first_event = scheduler_event_with_latency(7, "RenderThread", 10);
    first_event.maj_flt = 10;
    first_event.min_flt = 20;

    let stats_by_task = BTreeMap::<u32, crate::metrics::TaskStats>::new();
    let mut tasks = tasks::TaskTracker {
        active_targets,
        stats_by_task,
        ..Default::default()
    };
    let mut recorder = recorder::LiveRecorder {
        buffers: recorder::LiveBuffers {
            spike_events: Some(spike_events),
            ..Default::default()
        },
        ..Default::default()
    };

    events::handle_event(events::HandleEventInput {
        event: &first_event,
        config: &monitor_config,
        started: Instant::now(),
        tasks: &mut tasks,
        monotonic_start_ns: Some(100),
        recorder: &mut recorder,
        diagnostics: Default::default(),
    });

    // Second event is a spike with additional faults
    let mut spike_event = scheduler_event_with_latency(7, "RenderThread", 1_000_000);
    spike_event.maj_flt = 15; // +5 delta
    spike_event.min_flt = 30; // +10 delta

    events::handle_event(events::HandleEventInput {
        event: &spike_event,
        config: &monitor_config,
        started: Instant::now(),
        tasks: &mut tasks,
        monotonic_start_ns: Some(100),
        recorder: &mut recorder,
        diagnostics: Default::default(),
    });

    let spike_events_slice = recorder.buffers.spike_events.as_ref().unwrap().as_slice();
    assert_eq!(spike_events_slice.len(), 1);
    let spike = &spike_events_slice[0];
    assert_eq!(spike.major_faults, 5);
    assert_eq!(spike.minor_faults, 10);

    // Also verify TaskStats internal top_spikes has the same deltas
    let stats = tasks.stats_by_task.get(&7).unwrap();
    assert_eq!(stats.top_spikes.len(), 1);
    assert_eq!(stats.top_spikes[0].major_faults, 5);
    assert_eq!(stats.top_spikes[0].minor_faults, 10);
}

#[test]
fn alert_payload_captures_spike_task_identity() {
    let event = scheduler_event_with_latency(7, "RenderThread", 250_000_000);
    let mut stats = metrics::TaskStats::new(7, "RenderThread".to_owned(), 10);
    stats.apply_task_info(&task_info(
        7,
        77,
        "KingdomCome.exe",
        "RenderThread",
        TaskClass::Game,
    ));

    let payload = AlertPayload::from_task_stats(&stats, &event, 1234, None, None, None);

    assert_eq!(payload.title, "stutter latency alert");
    assert_eq!(payload.task, 7);
    assert_eq!(payload.class, TaskClass::Game);
    assert_eq!(payload.comm, "RenderThread");
    assert_eq!(payload.process_pid, Some(77));
    assert_eq!(payload.process_comm, "KingdomCome.exe");
    assert_eq!(payload.latency_ns, 250_000_000);
    assert_eq!(payload.latency_ms, 250);
    assert_eq!(payload.elapsed_ms, 1234);
    assert!(payload.message.contains("latency=250.000ms"));
}

#[test]
fn spike_event_buffer_caps_and_marks_truncated() {
    let mut buffer = SpikeEventBuffer::with_max_events(2);

    buffer.push(spike_event(1, 1_000));
    buffer.push(spike_event(2, 2_000));
    buffer.push(spike_event(3, 3_000));

    assert_eq!(buffer.as_slice().len(), 2);
    assert!(buffer.truncated());
    assert_eq!(buffer.as_slice()[0].task, 1);
    assert_eq!(buffer.as_slice()[1].task, 2);
}

#[test]
fn histogram_records_boundaries_and_overflow() {
    let mut histogram = metrics::LatencyHistogram::new();

    histogram.record(1_000);
    histogram.record(1_001);
    histogram.record(60_000_000);

    let buckets = histogram.snapshot();

    assert_eq!(buckets[0].upper_bound_ns, Some(1_000));
    assert_eq!(buckets[0].count, 1);
    assert_eq!(buckets[1].upper_bound_ns, Some(2_000));
    assert_eq!(buckets[1].count, 1);
    assert_eq!(buckets.last().unwrap().upper_bound_ns, None);
    assert_eq!(buckets.last().unwrap().count, 1);
}

#[test]
fn histogram_percentile_uses_conservative_bucket_upper_bound() {
    let mut histogram = metrics::LatencyHistogram::new();

    for _ in 0..95 {
        histogram.record(1_000);
    }
    for _ in 0..5 {
        histogram.record(1_500_000);
    }

    assert_eq!(histogram.percentile_upper_bound(100, 0.95), Some(1_000));
    assert_eq!(histogram.percentile_upper_bound(100, 0.99), Some(2_000_000));
}

#[test]
fn untruncated_snapshot_uses_exact_percentiles() {
    let mut stats = metrics::LatencyStats::new();

    stats.record(1_234);
    stats.record(9_876);

    let snapshot = stats.snapshot().unwrap();

    assert_eq!(snapshot.percentile_scope, "exact");
    assert_eq!(snapshot.stored_samples, 2);
    assert_eq!(snapshot.samples_truncated, 0);
    assert_eq!(snapshot.p95_ns, 9_876);
    assert_eq!(snapshot.p99_ns, 9_876);
}

#[test]
fn truncated_snapshot_uses_histogram_percentiles() {
    let mut stats = metrics::LatencyStats::new();

    for _ in 0..metrics::MAX_EXACT_SAMPLES {
        stats.record(1_000);
    }
    for _ in 0..4_000 {
        stats.record(2_000_000);
    }

    let snapshot = stats.snapshot().unwrap();

    assert_eq!(snapshot.percentile_scope, "histogram");
    assert_eq!(snapshot.samples_truncated, 4_000);
    assert_eq!(snapshot.p95_ns, 2_000_000);
    assert_eq!(snapshot.p99_ns, 2_000_000);
}

#[test]
fn snapshot_and_reset_clears_histogram_state() {
    let mut stats = metrics::LatencyStats::new();

    stats.record(1_000);
    assert!(stats.snapshot_and_reset().is_some());
    stats.record(60_000_000);

    let snapshot = stats.snapshot().unwrap();
    assert_eq!(snapshot.count, 1);
    assert_eq!(snapshot.histogram[0].count, 0);
    assert_eq!(snapshot.histogram.last().unwrap().count, 1);
}
#[test]
fn stat_wait_sum_saturation_helper_keeps_normal_values() {
    let (value, saturated) = crate::recorder::saturating_u128_to_u64(123_456u128);
    assert_eq!(value, 123_456);
    assert!(!saturated);
}

#[test]
fn stat_wait_sum_saturation_helper_caps_large_values() {
    let too_large = u64::MAX as u128 + 1;
    let (value, saturated) = crate::recorder::saturating_u128_to_u64(too_large);
    assert_eq!(value, u64::MAX);
    assert!(saturated);
}

#[test]
fn stat_wait_sum_saturation_helper_allows_u64_max() {
    let (value, saturated) = crate::recorder::saturating_u128_to_u64(u64::MAX as u128);
    assert_eq!(value, u64::MAX);
    assert!(!saturated);
}

#[test]
fn old_json_defaults_saturation_flag_to_false() {
    let task = SessionTask {
        stat_wait_sum_ns_saturated: true,
        ..Default::default()
    };
    let json = serde_json::to_string(&task).unwrap();
    // Remove the field from JSON to simulate old version
    let old_json = json.replace("\"stat_wait_sum_ns_saturated\":true,", "");
    let old_json = old_json.replace("\"stat_wait_sum_ns_saturated\":true", "");

    let deserialized: SessionTask = serde_json::from_str(&old_json).unwrap();
    assert!(!deserialized.stat_wait_sum_ns_saturated);
}
