//! Diagnosis orchestration; this module owns artifact windowing and candidate assembly.

use std::cmp::Ordering;

use super::{
    Diagnosis, DiagnosisConfig, EvidenceItem, EvidenceKind, StutterCause,
    anchor::{is_compositor_point, is_game_point},
    candidate::{
        DiagnosisContextSummary, finalize_diagnosis, push_candidate, push_supporting_evidence,
        scheduler_candidate_cause, scheduler_delay_score,
    },
    evidence::{
        push_cpu_frequency_evidence, push_cpu_perf_evidence, push_migration_evidence,
        push_page_fault_evidence, push_runnable_depth_evidence,
        push_runtime_slice_context_candidates, push_runtime_slice_supporting_evidence,
        push_scx_evidence,
    },
    evidence_chain::{EvidenceChainInputs, attach_evidence_chains},
};
use crate::{
    irq_inspect::{IrqDeviceClass, IrqLine, classify_irq_device},
    metrics::format_latency,
    recorder::{
        BlockIoRecord, CpuFreqRecord, DrmFenceEventRecord, FrameEvent, GpuSample, IntervalRecord,
        IrqEventRecord, MigrationEventRecord,
    },
    session_io::RunArtifacts,
    spike::SpikeCluster,
};

fn irq_inventory_from_artifacts(artifacts: &RunArtifacts) -> &[IrqLine] {
    artifacts
        .metadata
        .as_ref()
        .map(|metadata| metadata.core.metadata.irq_lines.as_slice())
        .unwrap_or(&[])
}

fn irq_line_for_event(irq_lines: &[IrqLine], irq: u32) -> Option<&IrqLine> {
    let irq = irq.to_string();
    irq_lines.iter().find(|line| line.irq == irq)
}

fn irq_device_label(line: Option<&IrqLine>) -> String {
    let Some(line) = line else {
        return "unknown device".to_owned();
    };

    let class = classify_irq_device(line);
    let name = if line.name.trim().is_empty() {
        "unnamed IRQ device"
    } else {
        line.name.trim()
    };

    format!("{name}, class={class:?}")
}

fn irq_class(line: Option<&IrqLine>) -> IrqDeviceClass {
    line.map(classify_irq_device)
        .unwrap_or(IrqDeviceClass::Unknown)
}

fn cluster_elapsed_midpoint(cluster: &SpikeCluster) -> Option<u64> {
    let mut values = cluster.points.iter().filter_map(|point| point.elapsed_ms);
    let first = values.next()?;
    let mut min = first;
    let mut max = first;

    for value in values {
        min = min.min(value);
        max = max.max(value);
    }

    Some(min + (max.saturating_sub(min) / 2))
}

fn coincident_frame_spike<'a>(
    cluster: &SpikeCluster,
    frames: &'a [FrameEvent],
    config: DiagnosisConfig,
) -> Option<&'a FrameEvent> {
    let elapsed = cluster_elapsed_midpoint(cluster)?;

    frames
        .iter()
        .filter(|frame| frame.frametime_ms.is_finite())
        .filter(|frame| frame.frametime_ms >= config.frame_spike_frametime_ms)
        .filter(|frame| frame.elapsed_ms.abs_diff(elapsed) <= config.frame_coincidence_window_ms)
        .max_by(|a, b| a.frametime_ms.total_cmp(&b.frametime_ms))
}

fn frame_spike_strength(frame: &FrameEvent, config: DiagnosisConfig) -> f32 {
    let excess_ms = (frame.frametime_ms - config.frame_spike_frametime_ms).max(0.0);
    (0.20 + (excess_ms / 50.0) as f32).clamp(0.20, 0.60)
}

fn max_drm_fence_wait(events: &[DrmFenceEventRecord]) -> Option<&DrmFenceEventRecord> {
    events
        .iter()
        .filter(|event| event.duration_ns.unwrap_or(0) >= 1_000_000)
        .max_by_key(|event| event.duration_ns.unwrap_or(0))
}

fn drm_fence_message(event: &DrmFenceEventRecord) -> String {
    let duration = event.duration_ns.unwrap_or_default();
    let role = event.gpu_role.as_deref().unwrap_or("unknown");
    let driver = event.driver.as_deref().unwrap_or("-");
    let comm = event.comm.as_deref().unwrap_or("-");

    format!(
        "DRM fence wait near spike: role={role} driver={driver} comm={comm} duration={}",
        format_latency(duration)
    )
}

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
    let irq_inventory = irq_inventory_from_artifacts(artifacts);

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

    let frame_events: Vec<FrameEvent> = if let Some((min, max)) = cluster_elapsed_range {
        artifacts
            .frame_events
            .iter()
            .filter(|frame| {
                frame.elapsed_ms >= min.saturating_sub(config.frame_coincidence_window_ms)
                    && frame.elapsed_ms <= max.saturating_add(config.frame_coincidence_window_ms)
            })
            .cloned()
            .collect()
    } else {
        Vec::new()
    };

    let drm_fence_events: Vec<DrmFenceEventRecord> = artifacts
        .drm_fence_events
        .iter()
        .filter(|event| {
            if event.timestamp_ns >= start_ns && event.timestamp_ns <= end_ns {
                return true;
            }

            if let (Some(wait_start), Some(wait_done)) = (event.wait_start_ns, event.wait_done_ns) {
                return wait_done >= start_ns && wait_start <= end_ns;
            }

            false
        })
        .cloned()
        .collect();

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

    let coincident_frame = coincident_frame_spike(cluster, &frame_events, config);

    if let Some(frame) = coincident_frame
        && let Some(cause) = scheduler_candidate_cause(&candidates)
    {
        let strength = frame_spike_strength(frame, config);
        push_supporting_evidence(
            &mut candidates,
            cause,
            EvidenceItem {
                kind: EvidenceKind::SchedulerDelay,
                strength,
                message: format!(
                    "frame spike ({:.1}ms) coincides with scheduling cluster",
                    frame.frametime_ms
                ),
                timestamp_ms: Some(frame.elapsed_ms),
                start_ns: None,
                end_ns: None,
            },
        );
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
        let irq_line = irq_line_for_event(irq_inventory, irq.irq);
        let irq_label = irq_device_label(irq_line);
        let irq_class = irq_class(irq_line);
        let message = format!(
            "IRQ {} ({irq_label}, cpu={}) active during spike (max duration {})",
            irq.irq,
            irq.cpu,
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

        if irq_class == IrqDeviceClass::Gpu {
            push_supporting_evidence(
                &mut candidates,
                StutterCause::IrqDelayCandidate,
                EvidenceItem {
                    kind: EvidenceKind::IrqOverlap,
                    strength: 0.30,
                    message: format!(
                        "IRQ {} is classified as a GPU interrupt; inspect IRQ affinity before changing CPU tuning",
                        irq.irq
                    ),
                    timestamp_ms: irq.elapsed_ms,
                    start_ns: Some(irq.enter_ns),
                    end_ns: Some(irq.exit_ns),
                },
            );
        }
    }

    // 4. GPU bound
    let max_fence_wait = max_drm_fence_wait(&drm_fence_events);
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

        let power_limited = sample
            .power_limit_reason
            .as_deref()
            .filter(|reason| !reason.trim().is_empty());

        let mut score = (busy_percent.saturating_sub(90)) as f32 / 10.0;
        if power_limited.is_some() {
            score = (score + 0.15).min(1.0);
        }
        if max_fence_wait.is_some() {
            score = (score + 0.10).min(1.0);
        }

        let message = match power_limited {
            Some(reason) => {
                format!("GPU busy at {busy_percent}% with power limit active ({reason})")
            }
            None => format!("GPU busy at {busy_percent}%"),
        };
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

        if let Some(reason) = power_limited {
            push_supporting_evidence(
                &mut candidates,
                StutterCause::GpuBoundCandidate,
                EvidenceItem {
                    kind: EvidenceKind::GpuBusy,
                    strength: 0.35,
                    message: format!("GPU power limit active near spike (reason: {reason})"),
                    timestamp_ms: Some(sample.elapsed_ms),
                    start_ns: None,
                    end_ns: None,
                },
            );
        }

        if let Some(fence) = max_fence_wait {
            push_supporting_evidence(
                &mut candidates,
                StutterCause::GpuBoundCandidate,
                EvidenceItem {
                    kind: EvidenceKind::DrmFenceWait,
                    strength: 0.30,
                    message: drm_fence_message(fence),
                    timestamp_ms: Some(fence.elapsed_ms),
                    start_ns: fence.wait_start_ns,
                    end_ns: fence.wait_done_ns.or(Some(fence.timestamp_ns)),
                },
            );
        }
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

    let mut diagnosis = finalize_diagnosis(candidates, config, context);
    attach_evidence_chains(
        &mut diagnosis,
        cluster,
        EvidenceChainInputs {
            frame: coincident_frame,
            irq_event: max_irq,
            irq_line: max_irq.and_then(|irq| irq_line_for_event(irq_inventory, irq.irq)),
            gpu_sample: high_gpu,
            drm_fence_event: max_fence_wait,
            block_io_event: max_io,
        },
    );
    diagnosis
}
