use super::*;
use crate::{
    diagnosis::{Confidence, LiveDiagnosisEntry, StutterCause},
    process_tree::TaskClass,
};

fn interval(elapsed_ms: u64, samples: u64) -> IntervalRecord {
    IntervalRecord {
        elapsed_ms,
        samples,
        ..Default::default()
    }
}

fn frame(elapsed_ms: u64, frametime_ms: f64) -> FrameEvent {
    FrameEvent {
        elapsed_ms,
        frametime_ms,
    }
}

fn irq_event(elapsed_ms: u64, duration_ns: u64) -> IrqEventRecord {
    IrqEventRecord {
        elapsed_ms: Some(elapsed_ms),
        irq: 44,
        cpu: 2,
        enter_ns: 1_000,
        exit_ns: 1_000 + duration_ns,
        duration_ns,
    }
}

fn block_io_event(elapsed_ms: u64, duration_ns: u64) -> BlockIoRecord {
    BlockIoRecord {
        elapsed_ms,
        tid: 77.into(),
        dev: 1,
        nr_sector: 8,
        correlation_basis: "dev-sector".into(),
        sector: 99,
        duration_ns,
        timestamp_ns: 2_000 + duration_ns,
        rwbs: "R".to_owned(),
    }
}

fn gpu_sample(elapsed_ms: u64, temp_millidegrees: u32) -> GpuSample {
    GpuSample {
        elapsed_ms,
        temp_millidegrees: Some(temp_millidegrees),
        gpu_busy_percent: Some(96),
        gpu_clock_mhz: Some(250),
        ..GpuSample::default()
    }
}

fn diagnosis(elapsed_ms: u64, cause: StutterCause) -> LiveDiagnosisEntry {
    LiveDiagnosisEntry {
        elapsed_ms,
        cause,
        confidence: Confidence::Medium,
        anchor_class: TaskClass::Game,
        anchor_comm: "RenderThread".to_owned(),
        evidence: vec!["test evidence".to_owned()],
    }
}

#[test]
fn default_window_is_thirty_seconds() {
    let window = RollingWindow::default();

    assert_eq!(window.duration(), Duration::from_secs(30));
    assert!(window.is_empty());
}

#[test]
fn push_interval_prunes_old_intervals_by_duration() {
    let mut window = RollingWindow::new(Duration::from_secs(2));

    window.push_interval(interval(1000, 10));
    window.push_interval(interval(2500, 20));
    window.push_interval(interval(3501, 30));

    assert_eq!(
        window
            .intervals
            .iter()
            .map(|record| record.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![2500, 3501]
    );
    assert_eq!(window.scored_samples(), 50);
}

#[test]
fn push_frame_prunes_old_frames_by_duration() {
    let mut window = RollingWindow::new(Duration::from_secs(1));

    window.push_frame(frame(1000, 16.0));
    window.push_frame(frame(1500, 17.0));
    window.push_frame(frame(2101, 18.0));

    assert_eq!(
        window
            .frames
            .iter()
            .map(|frame| frame.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![1500, 2101]
    );
}

#[test]
fn push_frame_drops_invalid_frametimes_and_counts_them() {
    let mut window = RollingWindow::new(Duration::from_secs(10));

    window.push_frame(frame(1000, 0.0));
    window.push_frame(frame(1050, 16.0));
    window.push_frame(frame(1100, f64::NAN));
    window.push_frame(frame(1200, -1.0));
    window.push_frame(frame(1300, 20.0));

    assert_eq!(window.dropped_invalid_frame_count(), 3);
    assert_eq!(
        window
            .frames
            .iter()
            .map(|frame| frame.frametime_ms)
            .collect::<Vec<_>>(),
        vec![16.0, 20.0]
    );
    assert_eq!(window.frame_max_ms(), 20.0);
    assert_eq!(window.frame_p99_ms(), 20.0);
}

#[test]
fn dropped_invalid_frametime_still_advances_window_pruning() {
    let mut window = RollingWindow::new(Duration::from_secs(1));

    window.push_frame(frame(1000, 16.0));
    window.push_frame(frame(2501, 0.0));

    assert!(window.frames.is_empty());
    assert_eq!(window.dropped_invalid_frame_count(), 1);
}

#[test]
fn push_diagnosis_prunes_old_diagnoses_by_duration() {
    let mut window = RollingWindow::new(Duration::from_secs(3));

    window.push_diagnosis(diagnosis(1000, StutterCause::Unknown));
    window.push_diagnosis(diagnosis(3000, StutterCause::GpuBoundCandidate));
    window.push_diagnosis(diagnosis(4501, StutterCause::GameThreadSchedulerDelay));

    assert_eq!(
        window
            .diagnoses
            .iter()
            .map(|diagnosis| diagnosis.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![3000, 4501]
    );
}

#[test]
fn prune_to_prunes_all_streams_using_same_cutoff() {
    let mut window = RollingWindow::new(Duration::from_secs(2));
    window.intervals.push_back(interval(1000, 10));
    window.intervals.push_back(interval(3000, 20));
    window.frames.push_back(frame(999, 16.0));
    window.frames.push_back(frame(2500, 22.0));
    window
        .diagnoses
        .push_back(diagnosis(1500, StutterCause::Unknown));
    window
        .diagnoses
        .push_back(diagnosis(3200, StutterCause::CpuPressureCandidate));

    window.prune_to(3500);

    assert_eq!(
        window
            .intervals
            .iter()
            .map(|record| record.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![3000]
    );
    assert_eq!(
        window
            .frames
            .iter()
            .map(|frame| frame.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![2500]
    );
    assert_eq!(
        window
            .diagnoses
            .iter()
            .map(|diagnosis| diagnosis.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![1500, 3200]
    );
}

#[test]
fn retain_latest_window_uses_latest_event_across_streams() {
    let mut window = RollingWindow::new(Duration::from_secs(1));
    window.intervals.push_back(interval(1000, 10));
    window.frames.push_back(frame(1500, 17.0));
    window
        .diagnoses
        .push_back(diagnosis(2300, StutterCause::CpuPressureCandidate));

    window.retain_latest_window();

    assert!(window.intervals.is_empty());
    assert_eq!(
        window
            .frames
            .iter()
            .map(|frame| frame.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![1500]
    );
    assert_eq!(
        window
            .diagnoses
            .iter()
            .map(|diagnosis| diagnosis.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![2300]
    );
}

#[test]
fn push_intervals_prunes_once_using_latest_inserted_elapsed_ms() {
    let mut window = RollingWindow::new(Duration::from_secs(2));

    window.push_intervals(vec![
        interval(1000, 1),
        interval(2000, 2),
        interval(3501, 3),
    ]);

    assert_eq!(
        window
            .intervals
            .iter()
            .map(|record| record.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![2000, 3501]
    );
    assert_eq!(window.scored_samples(), 5);
}

#[test]
fn push_intervals_sorts_out_of_order_batches_before_pruning() {
    let mut window = RollingWindow::new(Duration::from_secs(2));

    window.push_intervals(vec![
        interval(4501, 4),
        interval(1000, 1),
        interval(2500, 2),
    ]);

    assert_eq!(
        window
            .intervals
            .iter()
            .map(|record| record.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![4501]
    );
    assert_eq!(window.scored_samples(), 4);
}

#[test]
fn push_intervals_prunes_old_records_even_when_batch_arrives_after_newer_record() {
    let mut window = RollingWindow::new(Duration::from_secs(2));
    window.push_interval(interval(6000, 6));

    window.push_intervals(vec![interval(2500, 1), interval(4500, 4)]);

    assert_eq!(
        window
            .intervals
            .iter()
            .map(|record| record.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![4500, 6000]
    );
    assert_eq!(window.latest_elapsed_ms(), Some(6000));
    assert_eq!(window.scored_samples(), 10);
}

#[test]
fn push_intervals_preserves_same_tick_batch_order() {
    let mut window = RollingWindow::new(Duration::from_secs(2));
    let mut first = interval(1000, 1);
    first.task = 1;
    let mut second = interval(1000, 2);
    second.task = 2;

    window.push_intervals(vec![first, second]);

    assert_eq!(
        window
            .intervals
            .iter()
            .map(|record| record.task)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn push_intervals_empty_batch_is_noop() {
    let mut window = RollingWindow::new(Duration::from_secs(2));
    window.push_interval(interval(1000, 1));

    window.push_intervals(Vec::new());

    assert_eq!(
        window
            .intervals
            .iter()
            .map(|record| record.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![1000]
    );
    assert_eq!(window.scored_samples(), 1);
}

#[test]
fn untimestamped_irq_events_are_timestamped_at_ingestion() {
    let mut window = RollingWindow::new(Duration::from_secs(1));
    window.push_interval(interval(5_000, 10));
    let mut event = irq_event(0, 3_000_000);
    event.elapsed_ms = None;

    window.push_irq_event(event);

    assert_eq!(
        window
            .irq_events
            .iter()
            .map(|event| event.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![Some(5_000)]
    );
}

#[test]
fn untimestamped_irq_events_without_time_anchor_are_dropped() {
    let mut window = RollingWindow::new(Duration::from_secs(1));

    for _ in 0..256 {
        let mut event = irq_event(0, 3_000_000);
        event.elapsed_ms = None;
        window.push_irq_event(event);
    }

    assert!(window.irq_events.is_empty());
    assert_eq!(window.latest_elapsed_ms(), None);
}

#[test]
fn later_timestamped_irq_events_prune_ingestion_timestamped_events() {
    let mut window = RollingWindow::new(Duration::from_secs(1));
    window.push_interval(interval(5_000, 10));
    let mut untimestamped = irq_event(0, 3_000_000);
    untimestamped.elapsed_ms = None;
    window.push_irq_event(untimestamped);

    window.push_irq_event(irq_event(6_500, 1_000_000));

    assert_eq!(
        window
            .irq_events
            .iter()
            .map(|event| event.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![Some(6_500)]
    );
}

#[test]
fn prune_to_drops_legacy_untimestamped_irq_events() {
    let mut window = RollingWindow::new(Duration::from_secs(1));
    let mut event = irq_event(0, 3_000_000);
    event.elapsed_ms = None;
    window.irq_events.push_back(event);

    window.prune_to(2_000);

    assert!(window.irq_events.is_empty());
}

#[test]
fn latest_elapsed_ms_uses_max_across_streams() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.intervals.push_back(interval(1000, 10));
    window.frames.push_back(frame(4000, 16.0));
    window
        .diagnoses
        .push_back(diagnosis(2500, StutterCause::Unknown));

    assert_eq!(window.latest_elapsed_ms(), Some(4000));
}

#[test]
fn frame_stats_ignore_non_finite_negative_and_zero_values() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_frame(frame(1000, 0.0));
    window.push_frame(frame(1050, 16.0));
    window.push_frame(frame(1100, f64::NAN));
    window.push_frame(frame(1200, -1.0));
    window.push_frame(frame(1300, 33.0));

    assert_eq!(window.frame_count(), 2);
    assert_eq!(window.dropped_invalid_frame_count(), 3);
    assert_eq!(window.frame_max_ms(), 33.0);
    assert_eq!(window.frame_p99_ms(), 33.0);
}

#[test]
fn frame_stats_return_zero_when_every_frame_time_is_invalid() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_frame(frame(1000, 0.0));
    window.push_frame(frame(1100, -1.0));
    window.push_frame(frame(1200, f64::NAN));

    assert_eq!(window.frame_count(), 0);
    assert_eq!(window.dropped_invalid_frame_count(), 3);
    assert_eq!(window.frame_max_ms(), 0.0);
    assert_eq!(window.frame_p99_ms(), 0.0);
}

#[test]
fn window_score_aggregates_latency_frames_and_samples() {
    let mut window = RollingWindow::new(Duration::from_secs(5));
    window.push_interval(IntervalRecord {
        elapsed_ms: 1000,
        task: 42,
        samples: 30,
        over_1ms: 3,
        over_2ms: 2,
        over_5ms: 1,
        max_ns: 6_000_000,
        ..Default::default()
    });
    window.push_interval(IntervalRecord {
        elapsed_ms: 2000,
        task: 43,
        samples: 70,
        over_1ms: 4,
        over_2ms: 1,
        over_5ms: 0,
        max_ns: 4_000_000,
        ..Default::default()
    });
    window.push_frame(frame(1500, 16.0));
    window.push_frame(frame(1600, 33.0));

    let score = window.score();

    assert_eq!(score.duration_ms, 5000);
    assert_eq!(score.interval_count, 2);
    assert_eq!(score.scored_samples, 100);
    assert_eq!(score.over_1ms, 7);
    assert_eq!(score.over_2ms, 3);
    assert_eq!(score.over_5ms, 1);
    assert_eq!(score.diagnostic_score_total, 167);
    assert_eq!(score.max_latency_ns, 6_000_000);
    assert_eq!(score.frame_count, 2);
    assert_eq!(score.frame_p99_ms, 33.0);
    assert_eq!(score.frame_max_ms, 33.0);
    assert_eq!(score.dropped_invalid_frames, 0);
}

#[test]
fn window_score_quality_is_high_when_online_quality_gates_pass() {
    let mut window = RollingWindow::new(Duration::from_secs(10));

    for elapsed_ms in [1000, 2000, 3000, 4000, 5000] {
        window.push_interval(IntervalRecord {
            elapsed_ms,
            task: 42,
            samples: 20,
            over_1ms: 1,
            max_ns: 2_000_000,
            ..Default::default()
        });
    }

    let score = window.score();

    assert_eq!(score.interval_count, 5);
    assert_eq!(score.scored_samples, 100);
    assert_eq!(score.data_quality, OnlineDataQuality::High);
}

#[test]
fn window_score_default_quality_policy_does_not_require_frames() {
    let mut window = RollingWindow::new(Duration::from_secs(10));

    for elapsed_ms in [1000, 2000, 3000, 4000, 5000] {
        window.push_interval(IntervalRecord {
            elapsed_ms,
            task: 42,
            samples: 20,
            over_1ms: 1,
            max_ns: 2_000_000,
            ..Default::default()
        });
    }

    let score = window.score();

    assert_eq!(score.frame_count, 0);
    assert_eq!(score.data_quality, OnlineDataQuality::High);
}

#[test]
fn window_score_quality_is_low_when_policy_requires_frames_and_none_exist() {
    let mut window = RollingWindow::new(Duration::from_secs(10));

    for elapsed_ms in [1000, 2000, 3000, 4000, 5000] {
        window.push_interval(IntervalRecord {
            elapsed_ms,
            task: 42,
            samples: 20,
            over_1ms: 1,
            max_ns: 2_000_000,
            ..Default::default()
        });
    }

    let quality_policy = OnlineDataQualityPolicy {
        frame_data_policy: crate::autotune::quality::FrameDataPolicy::Required,
        ..OnlineDataQualityPolicy::default()
    };
    let score = window.score_with_quality_policy(&quality_policy);

    assert_eq!(score.frame_count, 0);
    assert!(score.data_quality.is_low());
    assert!(
        score
            .data_quality
            .reasons()
            .iter()
            .any(|reason| reason.contains("no frame data"))
    );
}

#[test]
fn window_score_quality_is_low_for_empty_window() {
    let window = RollingWindow::new(Duration::from_secs(10));

    let score = window.score();

    assert_eq!(score.interval_count, 0);
    assert_eq!(score.scored_samples, 0);
    assert_eq!(score.diagnostic_score_total, 0);
    assert!(score.data_quality.is_low());
    assert!(
        score
            .data_quality
            .reasons()
            .iter()
            .any(|reason| reason.contains("fewer than min_scored_intervals"))
    );
}

#[test]
fn window_score_quality_is_low_when_drop_counters_are_nonzero() {
    let mut window = RollingWindow::new(Duration::from_secs(10));

    for elapsed_ms in [1000, 2000, 3000, 4000, 5000] {
        window.push_interval(IntervalRecord {
            elapsed_ms,
            task: 42,
            samples: 20,
            drop_counters: crate::ebpf_loader::DropCountersSnapshot {
                ringbuf_reserve_failed: if elapsed_ms == 5000 { 1 } else { 0 },
                ..Default::default()
            },
            ..Default::default()
        });
    }

    let score = window.score();

    assert!(score.data_quality.is_low());
    assert!(
        score
            .data_quality
            .reasons()
            .iter()
            .any(|reason| reason.contains("drop counters above policy max"))
    );
}

#[test]
fn recent_diagnoses_vec_returns_cloned_diagnoses_in_order() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_diagnosis(diagnosis(1000, StutterCause::Unknown));
    window.push_diagnosis(diagnosis(2000, StutterCause::GpuBoundCandidate));

    let diagnoses = window.recent_diagnoses_vec();

    assert_eq!(diagnoses.len(), 2);
    assert_eq!(diagnoses[0].elapsed_ms, 1000);
    assert_eq!(diagnoses[1].elapsed_ms, 2000);
}

#[test]
fn clear_removes_all_streams() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_interval(interval(1000, 10));
    window.push_frame(frame(1000, 16.0));
    window.push_diagnosis(diagnosis(1000, StutterCause::Unknown));

    window.clear();

    assert!(window.is_empty());
    assert_eq!(window.total_event_count(), 0);
}

#[test]
fn objective_signals_mark_missing_io_and_irq_evidence_as_none() {
    let window = RollingWindow::new(Duration::from_secs(30));

    let signals = window.objective_signals();

    assert_eq!(signals.block_io_overlap_count, None);
    assert_eq!(signals.block_io_worst_latency_ns, None);
    assert_eq!(signals.irq_overlap_count, None);
    assert_eq!(signals.irq_worst_overlap_ns, None);
    assert_eq!(signals.irq_hot_irq, None);
    assert_eq!(signals.irq_hot_cpu, None);
    assert_eq!(signals.cpu_power_limited_cpu, None);
    assert_eq!(signals.gpu_busy_percent, None);
    assert_eq!(signals.gpu_clock_mhz, None);
    assert_eq!(signals.gpu_temp_millidegrees, None);
    assert_eq!(signals.dirty_writeback_events, None);
    assert_eq!(
        signals.signal_quality.memory_pressure,
        ObjectiveSignalQuality::Missing
    );
    assert_eq!(
        signals.signal_quality.block_io_overlap,
        ObjectiveSignalQuality::Missing
    );
}

#[test]
fn objective_signals_collect_io_irq_thermal_and_power_indicators() {
    let mut window = RollingWindow::new(Duration::from_secs(30));
    window.push_interval(interval(1_000, 10));
    window.push_irq_event(irq_event(1_100, 3_000_000));
    window.push_block_io_event(block_io_event(1_200, 8_000_000));
    window.push_gpu_sample(gpu_sample(1_300, 90_000));
    window.push_cpu_freq_event(CpuFreqRecord {
        elapsed_ms: 1_400,
        cpu: 0,
        freq_khz: 0,
        timestamp_ns: 123,
    });

    let signals = window.objective_signals();

    assert_eq!(signals.block_io_overlap_count, Some(1));
    assert_eq!(signals.block_io_worst_latency_ns, Some(8_000_000));
    assert_eq!(
        signals.block_io_overlap_basis.as_deref(),
        Some("dev-sector")
    );
    assert_eq!(
        signals.block_io_overlap_trust.as_deref(),
        Some("approximate")
    );
    assert_eq!(signals.irq_overlap_count, Some(1));
    assert_eq!(signals.irq_worst_overlap_ns, Some(3_000_000));
    assert_eq!(signals.irq_hot_irq, Some(44));
    assert_eq!(signals.irq_hot_cpu, Some(2));
    assert_eq!(signals.irq_overlap_basis.as_deref(), Some("irq-duration"));
    assert_eq!(signals.irq_overlap_trust.as_deref(), Some("direct"));
    assert_eq!(signals.thermal_degraded, Some(true));
    assert_eq!(signals.thermal_throttle_count, Some(1));
    assert_eq!(signals.cpu_power_limited, Some(true));
    assert_eq!(signals.cpu_power_limited_cpu, Some(0));
    assert_eq!(
        signals.cpu_power_limit_source.as_deref(),
        Some("cpu_freq_zero_khz")
    );
    assert_eq!(signals.cpu_power_limited_policy.as_deref(), Some("cpu0"));
    assert_eq!(signals.gpu_power_limited, Some(true));
    assert_eq!(
        signals.gpu_power_limit_reason.as_deref(),
        Some("busy_high_clock_low")
    );
    assert_eq!(signals.gpu_busy_percent, Some(96));
    assert_eq!(signals.gpu_clock_mhz, Some(250));
    assert_eq!(signals.gpu_temp_millidegrees, Some(90_000));
    assert_eq!(signals.memory_pressure_some_avg10_percent, Some(0.0));
    assert_eq!(signals.swap_activity_events, Some(0));
    assert_eq!(signals.dirty_writeback_events, Some(0));
    assert_eq!(
        signals.signal_quality.block_io_overlap,
        ObjectiveSignalQuality::Approximate
    );
    assert_eq!(
        signals.signal_quality.irq_overlap,
        ObjectiveSignalQuality::Direct
    );
    assert_eq!(
        signals.signal_quality.memory_pressure,
        ObjectiveSignalQuality::Direct
    );
}

#[test]
fn objective_signals_source_compile_throughput_from_progress_intervals() {
    let mut window = RollingWindow::new(Duration::from_secs(30));
    window.push_interval(IntervalRecord {
        elapsed_ms: 1_000,
        samples: 10,
        class: TaskClass::BuildJob,
        ..IntervalRecord::default()
    });
    window.push_interval(IntervalRecord {
        elapsed_ms: 1_000,
        samples: 25,
        class: TaskClass::Compiler,
        ..IntervalRecord::default()
    });
    window.push_interval(IntervalRecord {
        elapsed_ms: 2_000,
        samples: 5,
        class: TaskClass::Linker,
        ..IntervalRecord::default()
    });
    window.push_interval(IntervalRecord {
        elapsed_ms: 3_000,
        samples: 50,
        class: TaskClass::Indexer,
        ..IntervalRecord::default()
    });

    let signals = window.objective_signals();

    assert_eq!(signals.compile_progress_intervals, Some(2));
    assert_eq!(signals.compile_progress_samples, Some(40));
    assert_eq!(
        signals.compile_progress_source.as_deref(),
        Some("build-compiler-linker-intervals")
    );
    assert_eq!(
        signals.signal_quality.compile_throughput,
        ObjectiveSignalQuality::Direct
    );
}

#[test]
fn objective_signals_preserve_gpu_identity_when_sample_has_it() {
    let mut window = RollingWindow::new(Duration::from_secs(30));
    window.push_gpu_sample(GpuSample {
        elapsed_ms: 1_000,
        drm_card: Some("card1".to_owned()),
        render_node: Some("renderD129".to_owned()),
        gpu_busy_percent: Some(55),
        ..GpuSample::default()
    });

    let signals = window.objective_signals();

    assert_eq!(signals.gpu_drm_card.as_deref(), Some("card1"));
    assert_eq!(
        signals.gpu_active_render_node.as_deref(),
        Some("renderD129")
    );
    assert_eq!(
        signals.signal_quality.gpu_active_render_node,
        ObjectiveSignalQuality::Direct
    );
}

fn assert_non_decreasing<T, F>(items: &VecDeque<T>, elapsed_ms: F)
where
    F: Fn(&T) -> u64,
{
    let mut previous = None;
    for item in items {
        let current = elapsed_ms(item);
        if let Some(previous) = previous {
            assert!(previous <= current);
        }
        previous = Some(current);
    }
}

#[test]
fn push_frames_accepts_out_of_order_arrival_before_front_pop_pruning() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_frame(frame(20_000, 16.0));
    window.push_frame(frame(15_000, 16.0));
    window.push_frame(frame(26_000, 16.0));

    assert_eq!(
        window
            .frames
            .iter()
            .map(|f| f.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![20_000, 26_000]
    );
    assert_non_decreasing(&window.frames, |f| f.elapsed_ms);
}

#[test]
fn push_diagnoses_accepts_out_of_order_arrival_before_front_pop_pruning() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_diagnosis(diagnosis(20_000, StutterCause::Unknown));
    window.push_diagnosis(diagnosis(15_000, StutterCause::Unknown));
    window.push_diagnosis(diagnosis(26_000, StutterCause::Unknown));

    assert_eq!(
        window
            .diagnoses
            .iter()
            .map(|d| d.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![20_000, 26_000]
    );
    assert_non_decreasing(&window.diagnoses, |d| d.elapsed_ms);
}

#[test]
fn push_irq_events_accept_out_of_order_arrival_before_front_pop_pruning() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_irq_event(irq_event(20_000, 100));
    window.push_irq_event(irq_event(15_000, 100));
    window.push_irq_event(irq_event(26_000, 100));

    assert_eq!(
        window
            .irq_events
            .iter()
            .map(|e| e.elapsed_ms.unwrap())
            .collect::<Vec<_>>(),
        vec![20_000, 26_000]
    );
    assert_non_decreasing(&window.irq_events, |e| e.elapsed_ms.unwrap_or(0));
}

#[test]
fn push_block_io_events_accept_out_of_order_arrival_before_front_pop_pruning() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_block_io_event(block_io_event(20_000, 100));
    window.push_block_io_event(block_io_event(15_000, 100));
    window.push_block_io_event(block_io_event(26_000, 100));

    assert_eq!(
        window
            .block_io_events
            .iter()
            .map(|e| e.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![20_000, 26_000]
    );
    assert_non_decreasing(&window.block_io_events, |e| e.elapsed_ms);
}

#[test]
fn push_gpu_samples_accept_out_of_order_arrival_before_front_pop_pruning() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_gpu_sample(gpu_sample(20_000, 50));
    window.push_gpu_sample(gpu_sample(15_000, 50));
    window.push_gpu_sample(gpu_sample(26_000, 50));

    assert_eq!(
        window
            .gpu_samples
            .iter()
            .map(|e| e.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![20_000, 26_000]
    );
    assert_non_decreasing(&window.gpu_samples, |e| e.elapsed_ms);
}

#[test]
fn push_cpu_freq_events_accept_out_of_order_arrival_before_front_pop_pruning() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_cpu_freq_event(CpuFreqRecord {
        elapsed_ms: 20_000,
        cpu: 0,
        freq_khz: 1000,
        timestamp_ns: 0,
    });
    window.push_cpu_freq_event(CpuFreqRecord {
        elapsed_ms: 15_000,
        cpu: 0,
        freq_khz: 1000,
        timestamp_ns: 0,
    });
    window.push_cpu_freq_event(CpuFreqRecord {
        elapsed_ms: 26_000,
        cpu: 0,
        freq_khz: 1000,
        timestamp_ns: 0,
    });

    assert_eq!(
        window
            .cpu_freq_events
            .iter()
            .map(|e| e.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![20_000, 26_000]
    );
    assert_non_decreasing(&window.cpu_freq_events, |e| e.elapsed_ms);
}

#[test]
fn push_foreground_events_accept_out_of_order_arrival_before_front_pop_pruning() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    window.push_foreground_event(ForegroundEvent {
        elapsed_ms: 20_000,
        ..Default::default()
    });
    window.push_foreground_event(ForegroundEvent {
        elapsed_ms: 15_000,
        ..Default::default()
    });
    window.push_foreground_event(ForegroundEvent {
        elapsed_ms: 26_000,
        ..Default::default()
    });

    assert_eq!(
        window
            .foreground_events
            .iter()
            .map(|e| e.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![20_000, 26_000]
    );
    assert_non_decreasing(&window.foreground_events, |e| e.elapsed_ms);
}

#[test]
fn push_sorted_by_elapsed_preserves_same_tick_insertion_order() {
    let mut window = RollingWindow::new(Duration::from_secs(10));
    let mut first = frame(10_000, 16.0);
    first.frametime_ms = 10.0;
    let mut second = frame(10_000, 16.0);
    second.frametime_ms = 20.0;

    window.push_frame(first);
    window.push_frame(second);

    assert_eq!(
        window
            .frames
            .iter()
            .map(|f| f.frametime_ms)
            .collect::<Vec<_>>(),
        vec![10.0, 20.0]
    );
}
