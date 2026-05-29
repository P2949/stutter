use super::*;

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
