//! Tests for diagnosis scoring, evidence, candidate ranking, and anchor selection.
//!
//! Owns diagnosis regression tests and test-only fixtures. Does not own production diagnosis
//! configuration, models, evidence builders, or orchestration.

use std::collections::BTreeSet;

use super::*;
use crate::{
    process_tree::TaskClass,
    recorder::{BlockIoRecord, GpuSample, IntervalRecord, IrqEventRecord},
    session_io::RunArtifacts,
    spike::{SpikeCluster, SpikePoint},
};

fn spike_point(task: u32, class: TaskClass, comm: &str, latency_ns: u64) -> SpikePoint {
    let switch_ns = 100_000_000 + u64::from(task);
    SpikePoint {
        task,
        class,
        process_pid: Some(task),
        comm: comm.to_owned(),
        latency_ns,
        wakeup_ns: switch_ns.saturating_sub(latency_ns),
        switch_ns,
        elapsed_ms: Some(100),
        ..Default::default()
    }
}

fn spike_cluster(points: Vec<SpikePoint>) -> SpikeCluster {
    let distinct_tasks = points
        .iter()
        .map(|point| point.task)
        .collect::<BTreeSet<_>>()
        .len();
    let min_switch_ns = points.iter().map(|p| p.switch_ns).min().unwrap_or(0);
    let max_switch_ns = points.iter().map(|p| p.switch_ns).max().unwrap_or(0);
    let max_latency_ns = points.iter().map(|p| p.latency_ns).max().unwrap_or(0);

    SpikeCluster {
        points,
        distinct_tasks,
        min_switch_ns,
        max_switch_ns,
        max_latency_ns,
        ..Default::default()
    }
}

fn irq_event(duration_ns: u64) -> IrqEventRecord {
    IrqEventRecord {
        elapsed_ms: Some(100),
        irq: 137,
        cpu: 0,
        enter_ns: 100_000_000,
        exit_ns: 100_000_000 + duration_ns,
        duration_ns,
    }
}

fn block_io_event(duration_ns: u64) -> BlockIoRecord {
    BlockIoRecord {
        elapsed_ms: 100,
        tid: 100,
        dev: 1,
        nr_sector: 8,
        sector: 2048,
        duration_ns,
        timestamp_ns: 100_500_000,
        rwbs: "R".to_owned(),
        ..Default::default()
    }
}

fn cpu_psi_interval(cpu_psi_some: f64) -> IntervalRecord {
    IntervalRecord {
        elapsed_ms: 100,
        task: 100,
        active: true,
        class: TaskClass::Unknown,
        comm: "worker-a".to_owned(),
        process_pid: Some(100),
        process_comm: "worker-a".into(),
        samples: 1,
        stored_samples: 1,
        cpu_psi_some,
        ..Default::default()
    }
}

fn candidate(diagnosis: &Diagnosis, cause: StutterCause) -> &DiagnosisCandidate {
    diagnosis
        .candidates
        .iter()
        .find(|candidate| candidate.cause == cause)
        .unwrap()
}

fn candidate_index(diagnosis: &Diagnosis, cause: StutterCause) -> Option<usize> {
    diagnosis
        .candidates
        .iter()
        .position(|candidate| candidate.cause == cause)
}

fn assert_no_candidate(diagnosis: &Diagnosis, cause: StutterCause) {
    assert!(
        !diagnosis
            .candidates
            .iter()
            .any(|candidate| candidate.cause == cause),
        "unexpected candidate {:?}: {:#?}",
        cause,
        diagnosis
    );
}

fn assert_missing_contains(diagnosis: &Diagnosis, needle: &str) {
    assert!(
        diagnosis
            .missing_evidence
            .iter()
            .any(|message| message.contains(needle)),
        "missing_evidence did not contain {:?}: {:?}",
        needle,
        diagnosis.missing_evidence
    );
}

#[test]
fn confidence_words_are_cautious() {
    assert_eq!(Confidence::High.as_report_word(), "strong candidate");
    assert_eq!(Confidence::Medium.as_report_word(), "candidate");
    assert_eq!(Confidence::Low.as_report_word(), "weak candidate");

    assert!(Confidence::High.caution_text().contains("inference"));
    assert!(Confidence::Medium.caution_text().contains("mixed"));
    assert!(Confidence::Low.caution_text().contains("weak"));
}

#[test]
fn diagnosis_threshold_table_covers_all_config_fields() {
    let table = DiagnosisConfig::default().threshold_table();
    let keys = table.iter().map(|entry| entry.key).collect::<BTreeSet<_>>();
    let expected = [
        "irq_significant_ns",
        "block_io_significant_ns",
        "gpu_busy_bound_percent",
        "sched_delay_significant_ns",
        "cpu_psi_some_significant",
        "cpu_freq_drop_percent",
        "migration_window_ms",
        "page_fault_delta_threshold",
        "low_ipc_threshold",
        "high_cache_mpki_threshold",
        "min_primary_score",
        "min_primary_confidence",
        "min_primary_evidence_items",
        "min_scheduler_latency_for_primary_ns",
        "min_non_scheduler_score_for_primary",
        "runtime_high_ratio",
        "runtime_wait_high_ratio",
        "runtime_min_samples_for_primary_support",
    ];

    assert_eq!(
        table.len(),
        keys.len(),
        "duplicate threshold keys: {table:?}"
    );
    for key in expected {
        assert!(keys.contains(key), "missing threshold key {key}");
    }
    assert_eq!(table.len(), expected.len());
    for entry in table {
        assert!(entry.value > 0.0, "non-positive threshold value: {entry:?}");
        assert!(
            !entry.description.trim().is_empty(),
            "missing threshold description: {entry:?}"
        );
    }
}

#[test]
fn weak_scheduler_delay_is_not_forced_high_confidence() {
    let cluster = spike_cluster(vec![spike_point(
        456,
        TaskClass::Game,
        "RenderThread",
        2_100_000,
    )]);

    let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);

    assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
    assert_ne!(d.confidence, Confidence::High);
    assert_eq!(
        candidate(&d, StutterCause::GameThreadSchedulerDelay).confidence,
        Confidence::Medium
    );
}

#[test]
fn strict_primary_confidence_leaves_weak_scheduler_unknown() {
    let cluster = spike_cluster(vec![spike_point(
        456,
        TaskClass::Game,
        "RenderThread",
        2_100_000,
    )]);
    let config = DiagnosisConfig {
        min_primary_confidence: Confidence::High,
        ..DiagnosisConfig::default()
    };

    let d = diagnose_cluster_with_config(&cluster, &RunArtifacts::default(), 0, config);

    assert_eq!(d.cause, StutterCause::Unknown);
    assert!(d.primary.is_none());
    assert!(candidate_index(&d, StutterCause::GameThreadSchedulerDelay).is_some());
    assert_missing_contains(&d, "confidence below");
    assert!(
        d.candidate_rejections.iter().any(|rejection| {
            rejection.cause == StutterCause::GameThreadSchedulerDelay
                && rejection
                    .reasons
                    .iter()
                    .any(|reason| reason.contains("confidence below"))
        }),
        "expected rejected primary explanation, got {:?}",
        d.candidate_rejections
    );
}

#[test]
fn strong_scheduler_delay_still_reaches_high_confidence() {
    let cluster = spike_cluster(vec![spike_point(
        456,
        TaskClass::Game,
        "RenderThread",
        8_000_000,
    )]);

    let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);

    assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
    assert_eq!(d.confidence, Confidence::High);
}

#[test]
fn insufficient_evidence_returns_unknown_but_keeps_candidates() {
    let cluster = spike_cluster(vec![spike_point(
        123,
        TaskClass::Unknown,
        "worker",
        3_000_000,
    )]);
    let artifacts = RunArtifacts {
        irq_events: vec![irq_event(300_000)],
        ..Default::default()
    };

    let d = diagnose_cluster(&cluster, &artifacts, 0);

    assert_eq!(d.cause, StutterCause::Unknown);
    assert!(d.primary.is_none());
    assert!(candidate_index(&d, StutterCause::IrqDelayCandidate).is_some());
    assert!(
        d.secondary_causes
            .contains(&StutterCause::IrqDelayCandidate)
    );
    assert!(!d.evidence.is_empty());
    assert!(d.summary.starts_with("insufficient evidence"));
    assert_missing_contains(&d, "score below");
    assert_missing_contains(&d, "confidence below");
}

#[test]
fn missing_evidence_is_serialized() {
    let cluster = spike_cluster(vec![spike_point(
        123,
        TaskClass::Unknown,
        "worker",
        3_000_000,
    )]);
    let artifacts = RunArtifacts {
        irq_events: vec![irq_event(300_000)],
        ..Default::default()
    };
    let d = diagnose_cluster(&cluster, &artifacts, 0);

    let json = serde_json::to_value(&d).unwrap();
    let missing = json
        .get("missing_evidence")
        .and_then(|value| value.as_array())
        .expect("missing_evidence should serialize as an array");

    assert!(
        missing
            .iter()
            .filter_map(|value| value.as_str())
            .any(|message| message.contains("score below")),
        "serialized missing_evidence={missing:?}"
    );
}

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
fn diagnosis_report_summary_uses_candidate_wording() {
    let cluster = spike_cluster(vec![spike_point(
        456,
        TaskClass::Game,
        "RenderThread",
        8_000_000,
    )]);

    let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
    let summary = d.report_summary();

    assert!(summary.contains("GameThreadSchedulerDelay"));
    assert!(summary.contains("candidate"));
    assert!(summary.contains("inference"));
}

#[test]
fn scx_secondary_evidence_keeps_scheduler_primary() {
    let mut point = spike_point(456, TaskClass::Game, "RenderThread", 8_000_000);
    point.scx_ops = Some("scx_lavd".to_owned());
    point.scx_state = Some("enabled".to_owned());
    let cluster = spike_cluster(vec![point]);

    let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);

    assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
    let game = candidate(&d, StutterCause::GameThreadSchedulerDelay);
    assert!(
        game.evidence
            .iter()
            .any(|evidence| evidence.kind == EvidenceKind::ScxState)
    );
}

#[test]
fn cpu_perf_is_supporting_scheduler_evidence() {
    let cluster = spike_cluster(vec![spike_point(
        456,
        TaskClass::Game,
        "RenderThread",
        8_000_000,
    )]);
    let artifacts = RunArtifacts {
        intervals: vec![IntervalRecord {
            elapsed_ms: 100,
            task: 456,
            active: true,
            class: TaskClass::Game,
            comm: "RenderThread".to_owned(),
            process_pid: Some(456),
            process_comm: "game".into(),
            samples: 1,
            stored_samples: 1,
            cpu_perf: Some(crate::metrics::CpuPerfRecord {
                ipc: Some(0.50),
                cache_mpki: Some(45.0),
                cache_miss_rate: Some(0.10),
                ..Default::default()
            }),
            ..Default::default()
        }],
        ..Default::default()
    };

    let d = diagnose_cluster(&cluster, &artifacts, 0);

    assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
    let game = candidate(&d, StutterCause::GameThreadSchedulerDelay);
    assert!(
        game.evidence
            .iter()
            .any(|evidence| evidence.kind == EvidenceKind::CpuPerf
                && evidence.message.contains("ipc=0.50"))
    );
}

#[test]
fn classifies_sway_compositor_delay_by_task_class_not_comm_name() {
    let cluster = spike_cluster(vec![spike_point(
        123,
        TaskClass::Compositor,
        "sway",
        3_000_000,
    )]);
    let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
    assert_eq!(d.cause, StutterCause::CompositorSchedulerDelay);
}

#[test]
fn ranked_candidates_prefer_stronger_game_delay_over_weaker_compositor_delay() {
    let cluster = spike_cluster(vec![
        spike_point(123, TaskClass::Compositor, "sway", 3_000_000),
        spike_point(456, TaskClass::Game, "RenderThread", 10_000_000),
    ]);

    let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
    assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
    assert_eq!(
        d.primary.as_ref().unwrap().cause,
        StutterCause::GameThreadSchedulerDelay
    );
    assert!(
        d.secondary_causes
            .contains(&StutterCause::CompositorSchedulerDelay)
    );
    assert!(d.candidates.len() >= 2);

    let game = candidate(&d, StutterCause::GameThreadSchedulerDelay);
    let compositor = candidate(&d, StutterCause::CompositorSchedulerDelay);
    assert!(game.score > compositor.score);
    assert!(
        d.evidence
            .iter()
            .any(|e| e.contains("compositor thread 'sway' delayed by 3.000ms"))
    );
    assert!(
        d.evidence
            .iter()
            .any(|e| e.contains("game thread 'RenderThread' delayed by 10.000ms"))
    );
}

#[test]
fn compositor_wins_when_compositor_and_game_scores_are_similar() {
    let cluster = spike_cluster(vec![
        spike_point(123, TaskClass::Compositor, "sway", 6_000_000),
        spike_point(456, TaskClass::Game, "RenderThread", 5_000_000),
    ]);

    let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
    assert_eq!(d.cause, StutterCause::CompositorSchedulerDelay);
}

#[test]
fn ignores_tiny_irq_events() {
    let cluster = spike_cluster(vec![spike_point(
        123,
        TaskClass::Unknown,
        "other",
        3_000_000,
    )]);

    // 10us IRQ should be ignored (threshold 250us)
    let irq = irq_event(10_000);

    let artifacts = RunArtifacts {
        irq_events: vec![irq],
        ..Default::default()
    };
    let d = diagnose_cluster(&cluster, &artifacts, 0);
    assert!(
        !d.secondary_causes
            .contains(&StutterCause::IrqDelayCandidate)
    );
    assert_ne!(d.cause, StutterCause::IrqDelayCandidate);

    // 1ms IRQ should be caught
    let irq_big = irq_event(1_000_000);
    let artifacts2 = RunArtifacts {
        irq_events: vec![irq_big],
        ..Default::default()
    };
    let d2 = diagnose_cluster(&cluster, &artifacts2, 0);
    assert!(
        d2.secondary_causes
            .contains(&StutterCause::IrqDelayCandidate)
            || d2.cause == StutterCause::IrqDelayCandidate
    );
}

#[test]
fn tiny_irq_is_low_or_absent_when_scheduler_delay_dominates() {
    let cluster = spike_cluster(vec![spike_point(
        456,
        TaskClass::Game,
        "RenderThread",
        8_000_000,
    )]);
    let irq = irq_event(300_000);
    let artifacts = RunArtifacts {
        irq_events: vec![irq],
        ..Default::default()
    };

    let d = diagnose_cluster(&cluster, &artifacts, 0);
    assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
    assert_ne!(d.cause, StutterCause::IrqDelayCandidate);

    if let Some(irq_candidate) = d
        .candidates
        .iter()
        .find(|candidate| candidate.cause == StutterCause::IrqDelayCandidate)
    {
        assert!(irq_candidate.score < 0.40 || irq_candidate.confidence == Confidence::Low);
    }
}

#[test]
fn large_irq_can_be_primary_when_no_scheduler_anchor_exists() {
    let cluster = spike_cluster(vec![spike_point(
        123,
        TaskClass::Unknown,
        "other",
        3_000_000,
    )]);
    let irq = irq_event(4_000_000);
    let artifacts = RunArtifacts {
        irq_events: vec![irq],
        ..Default::default()
    };

    let d = diagnose_cluster(&cluster, &artifacts, 0);
    assert_eq!(d.cause, StutterCause::IrqDelayCandidate);
    assert!(matches!(
        d.confidence,
        Confidence::Medium | Confidence::High
    ));
    assert_eq!(
        d.primary.as_ref().unwrap().cause,
        StutterCause::IrqDelayCandidate
    );
}

#[test]
fn diagnosis_keeps_legacy_fields_populated() {
    let cluster = spike_cluster(vec![spike_point(
        456,
        TaskClass::Game,
        "RenderThread",
        8_000_000,
    )]);

    let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
    assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);
    assert_eq!(d.confidence, Confidence::High);
    assert!(!d.evidence.is_empty());
    assert!(d.primary.is_some());
    assert!(!d.candidates.is_empty());
    assert!(d.summary.contains("primary="));
}

#[test]
fn cluster_anchor_follows_ranked_game_primary() {
    // compositor 3ms + game 10ms => diagnosis primary GameThreadSchedulerDelay and anchor_kind Game
    let cluster = spike_cluster(vec![
        spike_point(123, TaskClass::Compositor, "sway", 3_000_000),
        spike_point(456, TaskClass::Game, "RenderThread", 10_000_000),
    ]);

    let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
    assert_eq!(d.cause, StutterCause::GameThreadSchedulerDelay);

    let anchor = select_anchor_for_diagnosis(&cluster, &d);
    assert_eq!(anchor.kind, ClusterAnchorKind::Game);
    assert_eq!(anchor.task, 456);
}

#[test]
fn cluster_anchor_follows_compositor_primary() {
    // compositor 6ms + game 5ms => diagnosis primary CompositorSchedulerDelay and anchor_kind Compositor
    let cluster = spike_cluster(vec![
        spike_point(123, TaskClass::Compositor, "sway", 6_000_000),
        spike_point(456, TaskClass::Game, "RenderThread", 5_000_000),
    ]);

    let d = diagnose_cluster(&cluster, &RunArtifacts::default(), 0);
    assert_eq!(d.cause, StutterCause::CompositorSchedulerDelay);

    let anchor = select_anchor_for_diagnosis(&cluster, &d);
    assert_eq!(anchor.kind, ClusterAnchorKind::Compositor);
    assert_eq!(anchor.task, 123);
}
