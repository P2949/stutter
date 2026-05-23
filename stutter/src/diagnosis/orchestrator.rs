//! Diagnosis orchestration; this module owns artifact windowing and candidate assembly.

use std::cmp::Ordering;

use super::{
    Diagnosis, DiagnosisConfig, EvidenceItem, EvidenceKind, StutterCause,
    anchor::{is_compositor_point, is_game_point},
    candidate::{
        DiagnosisContextSummary, finalize_diagnosis, push_candidate, scheduler_candidate_cause,
        scheduler_delay_score,
    },
    evidence::{
        push_cpu_frequency_evidence, push_cpu_perf_evidence, push_migration_evidence,
        push_page_fault_evidence, push_runnable_depth_evidence,
        push_runtime_slice_context_candidates, push_runtime_slice_supporting_evidence,
        push_scx_evidence,
    },
};
use crate::{
    metrics::format_latency,
    recorder::{
        BlockIoRecord, CpuFreqRecord, GpuSample, IntervalRecord, IrqEventRecord,
        MigrationEventRecord,
    },
    session_io::RunArtifacts,
    spike::SpikeCluster,
};

pub(crate) fn diagnose_cluster(
    cluster: &SpikeCluster,
    artifacts: &RunArtifacts,
    window_ns: u64,
) -> Diagnosis {
    diagnose_cluster_with_config(cluster, artifacts, window_ns, DiagnosisConfig::default())
}

pub(crate) fn diagnose_cluster_with_config(
    cluster: &SpikeCluster,
    artifacts: &RunArtifacts,
    window_ns: u64,
    config: DiagnosisConfig,
) -> Diagnosis {
    // Compute cluster time window
    let start_ns = cluster.min_switch_ns.saturating_sub(window_ns);
    let end_ns = cluster.max_switch_ns.saturating_add(window_ns);

    // Helper: cluster elapsed and range based on spike point elapsed_ms
    let cluster_elapsed_opt = cluster.points.iter().filter_map(|p| p.elapsed_ms).min();
    let cluster_elapsed_range = {
        let mut it = cluster.points.iter().filter_map(|p| p.elapsed_ms);
        if let Some(first) = it.next() {
            let mut min = first;
            let mut max = first;
            for v in it {
                min = min.min(v);
                max = max.max(v);
            }
            Some((min, max))
        } else {
            None
        }
    };

    // Filter artifacts to the cluster window / elapsed range
    let irq_events: Vec<IrqEventRecord> = artifacts
        .irq_events
        .iter()
        .filter(|e| e.exit_ns >= start_ns && e.enter_ns <= end_ns)
        .cloned()
        .collect();

    let gpu_samples: Vec<GpuSample> = if let Some(elapsed) = cluster_elapsed_opt {
        artifacts
            .gpu_samples
            .iter()
            .filter(|s| s.elapsed_ms.abs_diff(elapsed) <= 50)
            .cloned()
            .collect()
    } else {
        Vec::new()
    };

    // frame events are not currently used in diagnosis, but could be considered
    // for future enhancements.

    let io_events: Vec<BlockIoRecord> = artifacts
        .block_io_events
        .iter()
        .filter(|e| {
            e.timestamp_ns >= start_ns && e.timestamp_ns.saturating_sub(e.duration_ns) <= end_ns
        })
        .cloned()
        .collect();

    let cpu_freq_events: Vec<CpuFreqRecord> = artifacts
        .cpu_freq_events
        .iter()
        .filter(|e| e.timestamp_ns >= start_ns && e.timestamp_ns <= end_ns)
        .cloned()
        .collect();

    let migration_events: Vec<MigrationEventRecord> = artifacts
        .migration_events
        .iter()
        .filter(|e| e.timestamp_ns >= start_ns && e.timestamp_ns <= end_ns)
        .cloned()
        .collect();

    let interval_records: Vec<IntervalRecord> = if let Some((min, max)) = cluster_elapsed_range {
        artifacts
            .intervals
            .iter()
            .filter(|r| {
                r.elapsed_ms >= min.saturating_sub(1000) && r.elapsed_ms <= max.saturating_add(1000)
            })
            .cloned()
            .collect()
    } else {
        Vec::new()
    };

    let runtime_slices = if let Some((min, max)) = cluster_elapsed_range {
        artifacts
            .runtime_slices
            .iter()
            .filter(|r| {
                r.valid
                    && r.elapsed_ms >= min.saturating_sub(1000)
                    && r.elapsed_ms <= max.saturating_add(1000)
            })
            .cloned()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let mut candidates = Vec::new();
    let mut context = DiagnosisContextSummary::default();

    // 1. Compositor scheduler delay
    // compositor/gamescope >2ms near frame spike => CompositorSchedulerDelay
    let compositor_point = cluster
        .points
        .iter()
        .filter(|p| is_compositor_point(p) && p.latency_ns > config.sched_delay_significant_ns)
        .max_by_key(|p| p.latency_ns);

    if let Some(p) = compositor_point {
        context.saw_scheduler_delay = true;
        let score = scheduler_delay_score(p.latency_ns, config);
        let message = format!(
            "compositor thread '{}' delayed by {}",
            p.comm,
            format_latency(p.latency_ns)
        );
        push_candidate(
            &mut candidates,
            StutterCause::CompositorSchedulerDelay,
            score,
            EvidenceItem {
                kind: EvidenceKind::SchedulerDelay,
                strength: score,
                message,
                timestamp_ms: p.elapsed_ms,
                start_ns: Some(p.wakeup_ns),
                end_ns: Some(p.switch_ns),
            },
        );
        push_scx_evidence(&mut candidates, StutterCause::CompositorSchedulerDelay, p);
        push_runnable_depth_evidence(&mut candidates, StutterCause::CompositorSchedulerDelay, p);
    }

    // 2. Game thread scheduler delay
    // Game/Main/RenderThread >2ms near frame spike => GameThreadSchedulerDelay
    let game_point = cluster
        .points
        .iter()
        .filter(|p| is_game_point(p) && p.latency_ns > config.sched_delay_significant_ns)
        .max_by_key(|p| p.latency_ns);

    if let Some(p) = game_point {
        context.saw_scheduler_delay = true;
        let score = scheduler_delay_score(p.latency_ns, config);
        let message = format!(
            "game thread '{}' delayed by {}",
            p.comm,
            format_latency(p.latency_ns)
        );
        push_candidate(
            &mut candidates,
            StutterCause::GameThreadSchedulerDelay,
            score,
            EvidenceItem {
                kind: EvidenceKind::SchedulerDelay,
                strength: score,
                message,
                timestamp_ms: p.elapsed_ms,
                start_ns: Some(p.wakeup_ns),
                end_ns: Some(p.switch_ns),
            },
        );
        push_scx_evidence(&mut candidates, StutterCause::GameThreadSchedulerDelay, p);
        push_runnable_depth_evidence(&mut candidates, StutterCause::GameThreadSchedulerDelay, p);
    }

    // 3. IRQ overlap
    let max_irq = irq_events
        .iter()
        .filter(|e| e.duration_ns >= config.irq_significant_ns)
        .max_by_key(|e| e.duration_ns);

    if let Some(irq) = max_irq {
        context.saw_significant_irq = true;
        let mut score = irq.duration_ns as f32 / 2_000_000.0;
        if cluster.max_latency_ns > 0
            && (irq.duration_ns as f32 / cluster.max_latency_ns as f32) < 0.10
        {
            score = score.min(0.35);
        }
        let message = format!(
            "IRQ {} active during spike (max duration {})",
            irq.irq,
            format_latency(irq.duration_ns)
        );
        push_candidate(
            &mut candidates,
            StutterCause::IrqDelayCandidate,
            score,
            EvidenceItem {
                kind: EvidenceKind::IrqOverlap,
                strength: score,
                message,
                timestamp_ms: irq.elapsed_ms,
                start_ns: Some(irq.enter_ns),
                end_ns: Some(irq.exit_ns),
            },
        );
    }

    // 4. GPU bound
    let high_gpu = gpu_samples
        .iter()
        .filter_map(|s| {
            s.gpu_busy_percent
                .filter(|busy| *busy >= config.gpu_busy_bound_percent)
                .map(|busy| (s, busy))
        })
        .max_by_key(|(_, busy)| *busy);
    if let Some((sample, busy_percent)) = high_gpu {
        context.saw_high_gpu = true;
        let score = (busy_percent.saturating_sub(90)) as f32 / 10.0;
        let message = format!("GPU busy at {}%", busy_percent);
        push_candidate(
            &mut candidates,
            StutterCause::GpuBoundCandidate,
            score,
            EvidenceItem {
                kind: EvidenceKind::GpuBusy,
                strength: score,
                message,
                timestamp_ms: Some(sample.elapsed_ms),
                start_ns: None,
                end_ns: None,
            },
        );
    }

    // 5. Block I/O
    let max_io = io_events
        .iter()
        .filter(|e| e.duration_ns >= config.block_io_significant_ns)
        .max_by_key(|e| e.duration_ns);

    if let Some(io) = max_io {
        context.saw_significant_block_io = true;
        let score = (io.duration_ns as f32 / 8_000_000.0).max(0.40);
        let message = format!(
            "block I/O active (max duration {})",
            format_latency(io.duration_ns)
        );
        push_candidate(
            &mut candidates,
            StutterCause::BlockIoCandidate,
            score,
            EvidenceItem {
                kind: EvidenceKind::BlockIo,
                strength: score,
                message,
                timestamp_ms: Some(io.elapsed_ms),
                start_ns: Some(io.timestamp_ns.saturating_sub(io.duration_ns)),
                end_ns: Some(io.timestamp_ns),
            },
        );
    }

    // 6. CPU Pressure (PSI)
    let high_psi = interval_records
        .iter()
        .filter(|r| r.cpu_psi_some > config.cpu_psi_some_significant)
        .max_by(|a, b| {
            a.cpu_psi_some
                .partial_cmp(&b.cpu_psi_some)
                .unwrap_or(Ordering::Equal)
        });
    if let Some(r) = high_psi {
        context.saw_high_cpu_psi = true;
        let score = (r.cpu_psi_some as f32 / 100.0).max(0.40);
        let message = format!("high CPU PSI detected ({:.1}%)", r.cpu_psi_some);
        push_candidate(
            &mut candidates,
            StutterCause::CpuPressureCandidate,
            score,
            EvidenceItem {
                kind: EvidenceKind::CpuPressure,
                strength: score,
                message,
                timestamp_ms: Some(r.elapsed_ms),
                start_ns: None,
                end_ns: None,
            },
        );
    }

    if let Some(cause) = scheduler_candidate_cause(&candidates) {
        push_cpu_frequency_evidence(&mut candidates, cause, &cpu_freq_events, config);
        push_migration_evidence(
            &mut candidates,
            cause,
            &migration_events,
            cluster_elapsed_range,
            config,
        );
        push_page_fault_evidence(&mut candidates, cause, &interval_records, config);
        push_cpu_perf_evidence(&mut candidates, cause, &interval_records, cluster, config);
        push_runtime_slice_supporting_evidence(
            &mut candidates,
            cause,
            &runtime_slices,
            cluster,
            config,
        );
    }

    push_runtime_slice_context_candidates(&mut candidates, &runtime_slices, cluster, config);

    finalize_diagnosis(candidates, config, context)
}
