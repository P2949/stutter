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

#[test]
fn irq_candidate_message_includes_device_identity_when_metadata_has_irq_line() {
    let cluster = spike_cluster(vec![spike_point(
        100,
        TaskClass::Unknown,
        "worker",
        3_000_000,
    )]);

    let artifacts = RunArtifacts {
        metadata: Some(metadata_with_irq_line(137, "524288-edge amdgpu")),
        irq_events: vec![IrqEventRecord {
            elapsed_ms: Some(100),
            irq: 137,
            cpu: 1,
            enter_ns: 100_000_000,
            exit_ns: 104_000_000,
            duration_ns: 4_000_000,
        }],
        ..Default::default()
    };

    let diagnosis = diagnose_cluster(&cluster, &artifacts, 0);
    let irq = candidate(&diagnosis, StutterCause::IrqDelayCandidate);

    assert!(
        irq.evidence.iter().any(|evidence| {
            evidence.message.contains("IRQ 137")
                && evidence.message.contains("amdgpu")
                && evidence.message.contains("class=Gpu")
                && evidence.message.contains("cpu=1")
        }),
        "IRQ evidence should include device identity: {irq:#?}"
    );
    assert!(
        irq.evidence
            .iter()
            .any(|evidence| evidence.message.contains("classified as a GPU interrupt")),
        "GPU IRQ should get class-specific supporting evidence: {irq:#?}"
    );
}

#[test]
fn gpu_candidate_includes_power_limit_reason_when_available() {
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
            power_limit_reason: Some("power_cap".to_owned()),
            ..Default::default()
        }],
        ..Default::default()
    };

    let diagnosis = diagnose_cluster_with_config(&cluster, &artifacts, 0, config);
    let gpu = candidate(&diagnosis, StutterCause::GpuBoundCandidate);

    assert!(
        gpu.evidence.iter().any(|evidence| {
            evidence.message.contains("power limit active")
                && evidence.message.contains("power_cap")
        }),
        "GPU candidate should mention power limit reason: {gpu:#?}"
    );
}

#[test]
fn gpu_candidate_includes_drm_fence_wait_when_available() {
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
        drm_fence_events: vec![DrmFenceEventRecord {
            elapsed_ms: 100,
            timestamp_ns: 100_500_000,
            driver: Some("amdgpu".to_owned()),
            gpu_role: Some("render".to_owned()),
            comm: Some("Game.exe".to_owned()),
            wait_start_ns: Some(100_000_000),
            wait_done_ns: Some(103_000_000),
            duration_ns: Some(3_000_000),
            ..Default::default()
        }],
        ..Default::default()
    };

    let diagnosis = diagnose_cluster_with_config(&cluster, &artifacts, 5_000_000, config);
    let gpu = candidate(&diagnosis, StutterCause::GpuBoundCandidate);

    assert!(
        gpu.evidence.iter().any(|evidence| {
            evidence.kind == EvidenceKind::DrmFenceWait
                && evidence.message.contains("DRM fence wait")
                && evidence.message.contains("render")
                && evidence.message.contains("3.000ms")
        }),
        "GPU candidate should include DRM fence wait evidence: {gpu:#?}"
    );
}

#[test]
fn irq_candidate_records_explicit_evidence_chain() {
    let cluster = spike_cluster(vec![spike_point(
        100,
        TaskClass::Unknown,
        "worker",
        3_000_000,
    )]);

    let artifacts = RunArtifacts {
        metadata: Some(metadata_with_irq_line(137, "524288-edge amdgpu")),
        frame_events: vec![FrameEvent {
            elapsed_ms: 100,
            frametime_ms: 40.0,
        }],
        irq_events: vec![IrqEventRecord {
            elapsed_ms: Some(100),
            irq: 137,
            cpu: 1,
            enter_ns: 100_000_000,
            exit_ns: 104_000_000,
            duration_ns: 4_000_000,
        }],
        ..Default::default()
    };

    let diagnosis = diagnose_cluster(&cluster, &artifacts, 0);

    let chain = diagnosis
        .evidence_chains
        .iter()
        .find(|chain| chain.kind == EvidenceChainKind::Irq)
        .expect("IRQ candidate should include an explicit evidence chain");
    assert!(chain.explicit);
    assert!(chain.summary.contains("explicit IRQ chain"));
    assert_eq!(chain.nodes[0].kind, EvidenceChainNodeKind::Frame);
    assert_eq!(chain.nodes[1].kind, EvidenceChainNodeKind::Cluster);
    assert_eq!(chain.nodes[2].kind, EvidenceChainNodeKind::Event);
    assert_eq!(chain.nodes[3].kind, EvidenceChainNodeKind::Device);
    assert_eq!(chain.nodes[4].kind, EvidenceChainNodeKind::Recommendation);
    assert_eq!(chain.nodes[2].delta_from_previous_ms, Some(0));
    assert!(chain.nodes[3].label.contains("amdgpu"));
}

#[test]
fn gpu_fence_candidate_records_explicit_evidence_chains() {
    let config = DiagnosisConfig::default();
    let cluster = spike_cluster(vec![spike_point(
        456,
        TaskClass::Game,
        "RenderThread",
        config.sched_delay_significant_ns - 1,
    )]);

    let artifacts = RunArtifacts {
        frame_events: vec![FrameEvent {
            elapsed_ms: 100,
            frametime_ms: 42.0,
        }],
        gpu_samples: vec![GpuSample {
            elapsed_ms: 100,
            gpu_busy_percent: Some(99),
            drm_card: Some("card0".to_owned()),
            render_node: Some("renderD128".to_owned()),
            ..Default::default()
        }],
        drm_fence_events: vec![DrmFenceEventRecord {
            elapsed_ms: 100,
            timestamp_ns: 100_500_000,
            driver: Some("amdgpu".to_owned()),
            card: Some("card0".to_owned()),
            gpu_role: Some("render".to_owned()),
            comm: Some("Game.exe".to_owned()),
            wait_start_ns: Some(100_000_000),
            wait_done_ns: Some(103_000_000),
            duration_ns: Some(3_000_000),
            confidence: "high".to_owned(),
            ..Default::default()
        }],
        ..Default::default()
    };

    let diagnosis = diagnose_cluster_with_config(&cluster, &artifacts, 5_000_000, config);

    assert!(
        diagnosis
            .evidence_chains
            .iter()
            .any(|chain| chain.kind == EvidenceChainKind::Gpu
                && chain
                    .nodes
                    .iter()
                    .any(|node| node.kind == EvidenceChainNodeKind::Recommendation)),
        "GPU candidate should include a GPU evidence chain: {diagnosis:#?}"
    );
    assert!(
        diagnosis
            .evidence_chains
            .iter()
            .any(|chain| chain.kind == EvidenceChainKind::DrmFence
                && chain
                    .nodes
                    .iter()
                    .any(|node| node.label == "DRM fence wait")),
        "GPU candidate should include a DRM fence evidence chain: {diagnosis:#?}"
    );
}
