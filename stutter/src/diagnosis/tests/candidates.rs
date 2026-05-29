use super::*;

#[test]
fn weak_irq_overlap_does_not_create_irq_candidate() {
    let config = DiagnosisConfig::default();
    let cluster = spike_cluster(vec![spike_point(
        123,
        TaskClass::Unknown,
        "worker",
        3_000_000,
    )]);
    let artifacts = RunArtifacts {
        irq_events: vec![irq_event(config.irq_significant_ns - 1)],
        ..Default::default()
    };

    let d = diagnose_cluster_with_config(&cluster, &artifacts, 0, config);

    assert_no_candidate(&d, StutterCause::IrqDelayCandidate);
    assert_eq!(d.cause, StutterCause::Unknown);
    assert!(d.primary.is_none());
    assert_missing_contains(&d, "no IRQ event");
}

#[test]
fn weak_gpu_sample_does_not_create_gpu_candidate() {
    let config = DiagnosisConfig::default();
    let cluster = spike_cluster(vec![spike_point(
        123,
        TaskClass::Unknown,
        "worker",
        3_000_000,
    )]);
    let artifacts = RunArtifacts {
        gpu_samples: vec![GpuSample {
            elapsed_ms: 100,
            gpu_busy_percent: Some(config.gpu_busy_bound_percent - 1),
            ..Default::default()
        }],
        ..Default::default()
    };

    let d = diagnose_cluster_with_config(&cluster, &artifacts, 0, config);

    assert_no_candidate(&d, StutterCause::GpuBoundCandidate);
    assert_eq!(d.cause, StutterCause::Unknown);
    assert!(d.primary.is_none());
    assert_missing_contains(&d, "GPU");
}

#[test]
fn weak_block_io_does_not_create_block_candidate() {
    let config = DiagnosisConfig::default();
    let cluster = spike_cluster(vec![spike_point(
        123,
        TaskClass::Unknown,
        "worker",
        3_000_000,
    )]);
    let artifacts = RunArtifacts {
        block_io_events: vec![block_io_event(config.block_io_significant_ns - 1)],
        ..Default::default()
    };

    let d = diagnose_cluster_with_config(&cluster, &artifacts, 0, config);

    assert_no_candidate(&d, StutterCause::BlockIoCandidate);
    assert_eq!(d.cause, StutterCause::Unknown);
    assert!(d.primary.is_none());
    assert_missing_contains(&d, "no block I/O");
}

#[test]
fn weak_cpu_psi_does_not_create_cpu_pressure_candidate() {
    let config = DiagnosisConfig::default();
    let cluster = spike_cluster(vec![spike_point(
        123,
        TaskClass::Unknown,
        "worker",
        3_000_000,
    )]);
    let artifacts = RunArtifacts {
        intervals: vec![cpu_psi_interval(config.cpu_psi_some_significant - 1.0)],
        ..Default::default()
    };

    let d = diagnose_cluster_with_config(&cluster, &artifacts, 0, config);

    assert_no_candidate(&d, StutterCause::CpuPressureCandidate);
    assert_eq!(d.cause, StutterCause::Unknown);
    assert!(d.primary.is_none());
    assert_missing_contains(&d, "no CPU PSI");
}

#[test]
fn below_threshold_scheduler_delay_does_not_create_scheduler_candidate() {
    let config = DiagnosisConfig::default();
    let cluster = spike_cluster(vec![spike_point(
        456,
        TaskClass::Game,
        "RenderThread",
        config.sched_delay_significant_ns - 1,
    )]);

    let d = diagnose_cluster_with_config(&cluster, &RunArtifacts::default(), 0, config);

    assert_no_candidate(&d, StutterCause::GameThreadSchedulerDelay);
    assert_eq!(d.cause, StutterCause::Unknown);
    assert!(d.primary.is_none());
    assert_missing_contains(&d, "scheduler delay below");
}

#[test]
fn strong_scheduler_beats_weak_irq() {
    let cluster = spike_cluster(vec![spike_point(
        456,
        TaskClass::Game,
        "RenderThread",
        10_000_000,
    )]);
    let artifacts = RunArtifacts {
        irq_events: vec![irq_event(300_000)],
        ..Default::default()
    };

    let d = diagnose_cluster(&cluster, &artifacts, 0);

    assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
    assert!(candidate_index(&d, StutterCause::IrqDelayCandidate).is_some());
    assert!(
        candidate_index(&d, StutterCause::GameThreadSchedulerDelay).unwrap()
            < candidate_index(&d, StutterCause::IrqDelayCandidate).unwrap()
    );
}

#[test]
fn strong_irq_beats_unknown_worker_noise() {
    let cluster = spike_cluster(vec![
        spike_point(100, TaskClass::Unknown, "worker-a", 3_000_000),
        spike_point(101, TaskClass::Unknown, "worker-b", 2_500_000),
        spike_point(102, TaskClass::Unknown, "worker-c", 2_000_000),
    ]);
    let artifacts = RunArtifacts {
        irq_events: vec![irq_event(4_000_000)],
        ..Default::default()
    };

    let d = diagnose_cluster(&cluster, &artifacts, 0);

    assert_eq!(d.cause, StutterCause::IrqDelayCandidate);
    assert_eq!(d.candidates[0].cause, StutterCause::IrqDelayCandidate);
}

#[test]
fn block_io_orders_before_cpu_pressure_when_score_is_higher() {
    let cluster = spike_cluster(vec![
        spike_point(100, TaskClass::Unknown, "worker-a", 3_000_000),
        spike_point(101, TaskClass::Unknown, "worker-b", 2_500_000),
        spike_point(102, TaskClass::Unknown, "worker-c", 2_000_000),
    ]);
    let artifacts = RunArtifacts {
        block_io_events: vec![block_io_event(8_000_000)],
        intervals: vec![cpu_psi_interval(80.0)],
        ..Default::default()
    };

    let d = diagnose_cluster(&cluster, &artifacts, 0);

    assert_eq!(d.cause, StutterCause::BlockIoCandidate);
    assert_eq!(d.candidates[0].cause, StutterCause::BlockIoCandidate);
    assert!(candidate_index(&d, StutterCause::CpuPressureCandidate).is_some());
}

#[test]
fn gpu_bound_beats_below_threshold_scheduler_delay() {
    let config = DiagnosisConfig::default();
    let cluster = spike_cluster(vec![spike_point(
        456,
        TaskClass::Game,
        "RenderThread",
        config.sched_delay_significant_ns - 1,
    )]);
    let artifacts = RunArtifacts {
        gpu_samples: vec![GpuSample {
            elapsed_ms: 100,
            gpu_busy_percent: Some(99),
            ..Default::default()
        }],
        ..Default::default()
    };

    let d = diagnose_cluster_with_config(&cluster, &artifacts, 0, config);

    assert_eq!(d.cause, StutterCause::GpuBoundCandidate);
    assert_no_candidate(&d, StutterCause::GameThreadSchedulerDelay);
}
