use std::{cmp::Ordering, collections::BTreeMap};

mod model;

use model::SCHED_DELAY_SIGNIFICANT_NS;
pub use model::{
    CandidateRejection, ClusterAnchor, ClusterAnchorKind, Confidence, Diagnosis,
    DiagnosisCandidate, DiagnosisConfig, DiagnosisThresholdDoc, EvidenceItem, EvidenceKind,
    FrameDiagnosis, LiveDiagnosisEntry, StutterCause,
};

use crate::{
    metrics::format_latency,
    process_tree::TaskClass,
    recorder::{
        BlockIoRecord, CpuFreqRecord, GpuSample, IntervalRecord, IrqEventRecord,
        MigrationEventRecord,
    },
    session_io::RunArtifacts,
    spike::{SpikeCluster, SpikePoint},
};

fn is_compositor_point(p: &SpikePoint) -> bool {
    matches!(p.class, TaskClass::Compositor | TaskClass::GameScope)
}

fn is_game_point(p: &SpikePoint) -> bool {
    matches!(
        p.class,
        TaskClass::Game | TaskClass::GameHelper | TaskClass::WineServer
    ) || matches!(p.comm.as_str(), "Main" | "RenderThread")
}

fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

fn confidence_from_score(score: f32) -> Confidence {
    if score >= 0.75 {
        Confidence::High
    } else if score >= 0.40 {
        Confidence::Medium
    } else {
        Confidence::Low
    }
}

fn push_candidate(
    candidates: &mut Vec<DiagnosisCandidate>,
    cause: StutterCause,
    raw_score: f32,
    mut evidence: EvidenceItem,
) {
    let score = clamp01(raw_score);
    evidence.strength = clamp01(evidence.strength);

    if let Some(existing) = candidates.iter_mut().find(|c| c.cause == cause) {
        existing.score = existing.score.max(score);
        existing.confidence = confidence_from_score(existing.score);
        existing.evidence.push(evidence);
    } else {
        candidates.push(DiagnosisCandidate {
            cause,
            score,
            confidence: confidence_from_score(score),
            evidence: vec![evidence],
        });
    }
}

fn push_supporting_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    mut evidence: EvidenceItem,
) {
    evidence.strength = clamp01(evidence.strength);
    if let Some(candidate) = candidates
        .iter_mut()
        .find(|candidate| candidate.cause == cause)
    {
        candidate.evidence.push(evidence);
    }
}

fn scheduler_candidate_cause(candidates: &[DiagnosisCandidate]) -> Option<StutterCause> {
    candidates
        .iter()
        .filter(|candidate| {
            matches!(
                candidate.cause,
                StutterCause::GameThreadSchedulerDelay | StutterCause::CompositorSchedulerDelay
            )
        })
        .max_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| b.cause.priority().cmp(&a.cause.priority()))
        })
        .map(|candidate| candidate.cause)
}

#[derive(Default, Clone, Debug)]
struct DiagnosisContextSummary {
    saw_scheduler_delay: bool,
    saw_significant_irq: bool,
    saw_high_gpu: bool,
    saw_significant_block_io: bool,
    saw_high_cpu_psi: bool,
}

fn scheduler_delay_score(latency_ns: u64, config: DiagnosisConfig) -> f32 {
    if latency_ns < config.sched_delay_significant_ns {
        return 0.0;
    }

    let latency_ms = latency_ns as f32 / 1_000_000.0;
    let score = if latency_ms < 4.0 {
        0.40 + ((latency_ms - 2.0).max(0.0) / 2.0) * 0.15
    } else if latency_ms < 8.0 {
        0.55 + ((latency_ms - 4.0) / 4.0) * 0.20
    } else if latency_ms < 16.0 {
        0.75 + ((latency_ms - 8.0) / 8.0) * 0.25
    } else {
        1.0
    };
    score.clamp(0.40, 1.0)
}

fn is_scheduler_cause(cause: StutterCause) -> bool {
    matches!(
        cause,
        StutterCause::GameThreadSchedulerDelay | StutterCause::CompositorSchedulerDelay
    )
}

fn scheduler_latency_for_candidate(candidate: &DiagnosisCandidate) -> u64 {
    candidate
        .evidence
        .iter()
        .filter(|evidence| evidence.kind == EvidenceKind::SchedulerDelay)
        .filter_map(|evidence| Some(evidence.end_ns?.saturating_sub(evidence.start_ns?)))
        .max()
        .unwrap_or(0)
}

fn primary_rejection_reasons(
    candidate: &DiagnosisCandidate,
    config: DiagnosisConfig,
) -> Vec<String> {
    let mut reasons = Vec::new();

    if candidate.score < config.min_primary_score {
        reasons.push(format!(
            "candidate score below minimum primary score: {:.2} < {:.2}",
            candidate.score, config.min_primary_score
        ));
    }

    if candidate.confidence < config.min_primary_confidence {
        reasons.push(format!(
            "candidate confidence below minimum primary confidence: {:?} < {:?}",
            candidate.confidence, config.min_primary_confidence
        ));
    }

    if candidate.evidence.len() < config.min_primary_evidence_items {
        reasons.push(format!(
            "candidate has too few evidence items: {} < {}",
            candidate.evidence.len(),
            config.min_primary_evidence_items
        ));
    }

    if is_scheduler_cause(candidate.cause) {
        let scheduler_latency_ns = scheduler_latency_for_candidate(candidate);
        if scheduler_latency_ns < config.min_scheduler_latency_for_primary_ns {
            reasons.push(format!(
                "scheduler delay below primary threshold: {}ns < {}ns",
                scheduler_latency_ns, config.min_scheduler_latency_for_primary_ns
            ));
        }
    } else if config.min_non_scheduler_score_for_primary > config.min_primary_score
        && candidate.score < config.min_non_scheduler_score_for_primary
    {
        reasons.push(format!(
            "non-scheduler candidate score below minimum primary score: {:.2} < {:.2}",
            candidate.score, config.min_non_scheduler_score_for_primary
        ));
    }

    reasons
}

fn candidate_is_sufficient_primary(
    candidate: &DiagnosisCandidate,
    config: DiagnosisConfig,
) -> Result<(), String> {
    let reasons = primary_rejection_reasons(candidate, config);
    if let Some(reason) = reasons.into_iter().next() {
        Err(reason)
    } else {
        Ok(())
    }
}

fn push_unique_missing(missing: &mut Vec<String>, message: impl Into<String>) {
    let message = message.into();
    if !missing.iter().any(|existing| existing == &message) {
        missing.push(message);
    }
}

fn missing_evidence_for_context(
    candidates: &[DiagnosisCandidate],
    rejected_primary: Option<&DiagnosisCandidate>,
    config: DiagnosisConfig,
    context: &DiagnosisContextSummary,
) -> Vec<String> {
    let mut missing = Vec::new();

    if candidates.is_empty() {
        push_unique_missing(
            &mut missing,
            "no candidate reached the minimum evidence threshold",
        );
    }

    if let Some(candidate) = rejected_primary {
        for reason in primary_rejection_reasons(candidate, config) {
            push_unique_missing(&mut missing, reason);
        }
    }

    if !context.saw_scheduler_delay {
        push_unique_missing(&mut missing, "scheduler delay below primary threshold");
    }
    if !context.saw_significant_irq {
        push_unique_missing(
            &mut missing,
            "no IRQ event above significant-duration threshold",
        );
    }
    if !context.saw_high_gpu {
        push_unique_missing(&mut missing, "no GPU sample at or above busy threshold");
    }
    if !context.saw_significant_block_io {
        push_unique_missing(
            &mut missing,
            "no block I/O event above significant-duration threshold",
        );
    }
    if !context.saw_high_cpu_psi {
        push_unique_missing(&mut missing, "no CPU PSI sample above pressure threshold");
    }

    missing
}

fn normalize_and_sort_candidates(candidates: &mut [DiagnosisCandidate]) {
    for candidate in candidates {
        candidate.score = clamp01(candidate.score);
        candidate.confidence = confidence_from_score(candidate.score);
        for evidence in &mut candidate.evidence {
            evidence.strength = clamp01(evidence.strength);
        }
    }
}

fn sort_candidates(candidates: &mut [DiagnosisCandidate]) {
    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.cause.priority().cmp(&b.cause.priority()))
    });
}

fn unknown_diagnosis_with_candidates(
    candidates: Vec<DiagnosisCandidate>,
    missing_evidence: Vec<String>,
    summary: String,
) -> Diagnosis {
    let secondary_causes = candidates.iter().map(|c| c.cause).collect::<Vec<_>>();
    let evidence = candidates
        .iter()
        .flat_map(|c| c.evidence.iter().map(|e| e.message.clone()))
        .collect::<Vec<_>>();

    Diagnosis {
        cause: StutterCause::Unknown,
        confidence: Confidence::Low,
        secondary_causes,
        evidence,
        missing_evidence,
        primary: None,
        candidates,
        candidate_rejections: Vec::new(),
        summary,
    }
}

fn finalize_diagnosis(
    mut candidates: Vec<DiagnosisCandidate>,
    config: DiagnosisConfig,
    context: DiagnosisContextSummary,
) -> Diagnosis {
    if candidates.is_empty() {
        let missing_evidence = missing_evidence_for_context(&[], None, config, &context);
        return Diagnosis {
            cause: StutterCause::Unknown,
            confidence: Confidence::Low,
            secondary_causes: Vec::new(),
            evidence: vec!["no strong correlation found".to_owned()],
            missing_evidence,
            primary: None,
            candidates: Vec::new(),
            candidate_rejections: Vec::new(),
            summary: "insufficient evidence: no candidate reached diagnosis thresholds".to_owned(),
        };
    }

    normalize_and_sort_candidates(&mut candidates);
    sort_candidates(&mut candidates);

    let primary = candidates[0].clone();
    if candidate_is_sufficient_primary(&primary, config).is_err() {
        let candidate_rejections = candidate_rejections(&candidates, config);
        let missing_evidence =
            missing_evidence_for_context(&candidates, Some(&primary), config, &context);
        let mut diagnosis = unknown_diagnosis_with_candidates(
            candidates,
            missing_evidence,
            format!(
                "insufficient evidence: best_candidate={:?} confidence={:?} score={:.2}",
                primary.cause, primary.confidence, primary.score
            ),
        );
        diagnosis.candidate_rejections = candidate_rejections;
        return diagnosis;
    }

    let secondary_causes = candidates
        .iter()
        .skip(1)
        .map(|c| c.cause)
        .collect::<Vec<_>>();

    let evidence = candidates
        .iter()
        .flat_map(|c| c.evidence.iter().map(|e| e.message.clone()))
        .collect::<Vec<_>>();

    let missing_evidence = missing_evidence_for_context(&candidates, None, config, &context);
    let summary = format!(
        "primary={:?} confidence={:?} score={:.2}",
        primary.cause, primary.confidence, primary.score
    );
    let candidate_rejections = candidate_rejections(&candidates, config);

    Diagnosis {
        cause: primary.cause,
        confidence: primary.confidence,
        secondary_causes,
        evidence,
        missing_evidence,
        primary: Some(primary),
        candidates,
        candidate_rejections,
        summary,
    }
}

fn candidate_rejections(
    candidates: &[DiagnosisCandidate],
    config: DiagnosisConfig,
) -> Vec<CandidateRejection> {
    candidates
        .iter()
        .filter_map(|candidate| {
            let reasons = primary_rejection_reasons(candidate, config);
            (!reasons.is_empty()).then_some(CandidateRejection {
                cause: candidate.cause,
                score: candidate.score,
                confidence: candidate.confidence,
                reasons,
            })
        })
        .collect()
}

pub(crate) fn select_anchor_for_diagnosis(
    cluster: &SpikeCluster,
    diagnosis: &Diagnosis,
) -> ClusterAnchor {
    if diagnosis.cause == StutterCause::GameThreadSchedulerDelay {
        let game_anchor = cluster
            .points
            .iter()
            .filter(|p| is_game_point(p) && p.latency_ns > SCHED_DELAY_SIGNIFICANT_NS)
            .max_by_key(|p| p.latency_ns);

        if let Some(p) = game_anchor {
            return ClusterAnchor {
                task: p.task,
                class: p.class,
                comm: p.comm.clone(),
                latency_ns: p.latency_ns,
                kind: ClusterAnchorKind::Game,
            };
        }
    }

    if diagnosis.cause == StutterCause::CompositorSchedulerDelay {
        let compositor_anchor = cluster
            .points
            .iter()
            .filter(|p| is_compositor_point(p) && p.latency_ns > SCHED_DELAY_SIGNIFICANT_NS)
            .max_by_key(|p| p.latency_ns);

        if let Some(p) = compositor_anchor {
            return ClusterAnchor {
                task: p.task,
                class: p.class,
                comm: p.comm.clone(),
                latency_ns: p.latency_ns,
                kind: ClusterAnchorKind::Compositor,
            };
        }
    }

    select_anchor(cluster)
}

pub(crate) fn select_anchor(cluster: &SpikeCluster) -> ClusterAnchor {
    // 1. Prefer highest-latency TaskClass::Compositor or TaskClass::GameScope point above 2ms.
    let compositor_anchor = cluster
        .points
        .iter()
        .filter(|p| is_compositor_point(p) && p.latency_ns > SCHED_DELAY_SIGNIFICANT_NS)
        .max_by_key(|p| p.latency_ns);

    if let Some(p) = compositor_anchor {
        return ClusterAnchor {
            task: p.task,
            class: p.class,
            comm: p.comm.clone(),
            latency_ns: p.latency_ns,
            kind: ClusterAnchorKind::Compositor,
        };
    }

    // 2. Else prefer highest-latency Game/RenderThread/Main point above 2ms.
    let game_anchor = cluster
        .points
        .iter()
        .filter(|p| is_game_point(p) && p.latency_ns > SCHED_DELAY_SIGNIFICANT_NS)
        .max_by_key(|p| p.latency_ns);

    if let Some(p) = game_anchor {
        return ClusterAnchor {
            task: p.task,
            class: p.class,
            comm: p.comm.clone(),
            latency_ns: p.latency_ns,
            kind: ClusterAnchorKind::Game,
        };
    }

    // 3. Else use highest-latency point.
    let fallback = cluster
        .points
        .iter()
        .max_by_key(|p| p.latency_ns)
        .expect("cluster must have points");

    ClusterAnchor {
        task: fallback.task,
        class: fallback.class,
        comm: fallback.comm.clone(),
        latency_ns: fallback.latency_ns,
        kind: ClusterAnchorKind::Other,
    }
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

fn push_runnable_depth_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    point: &SpikePoint,
) {
    if point.observed_runnable_depth >= 4 {
        push_supporting_evidence(
            candidates,
            cause,
            EvidenceItem {
                kind: EvidenceKind::SchedulerDelay,
                strength: 0.40,
                message: format!(
                    "high monitored runnable depth ({} target tasks) suggests target-local CPU contention",
                    point.observed_runnable_depth
                ),
                timestamp_ms: point.elapsed_ms,
                start_ns: Some(point.wakeup_ns),
                end_ns: Some(point.switch_ns),
            },
        );
    }

    if point.target_pending_wakeups >= 4 {
        push_supporting_evidence(
            candidates,
            cause,
            EvidenceItem {
                kind: EvidenceKind::SchedulerDelay,
                strength: 0.20,
                message: format!(
                    "monitored wakeup backlog ({} tasks) (diagnostic-only)",
                    point.target_pending_wakeups
                ),
                timestamp_ms: point.elapsed_ms,
                start_ns: Some(point.wakeup_ns),
                end_ns: Some(point.switch_ns),
            },
        );
    }
}

fn push_scx_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    point: &SpikePoint,
) {
    if point.scx_ops.is_none() && point.scx_state.is_none() {
        return;
    }

    push_supporting_evidence(
        candidates,
        cause,
        EvidenceItem {
            kind: EvidenceKind::ScxState,
            strength: 0.25,
            message: format!(
                "SCX state near spike: ops={} state={}",
                point.scx_ops.as_deref().unwrap_or("-"),
                point.scx_state.as_deref().unwrap_or("-")
            ),
            timestamp_ms: point.elapsed_ms,
            start_ns: Some(point.wakeup_ns),
            end_ns: Some(point.switch_ns),
        },
    );
}

fn push_cpu_frequency_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    events: &[CpuFreqRecord],
    config: DiagnosisConfig,
) {
    let mut by_cpu: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    for event in events {
        let entry = by_cpu
            .entry(event.cpu)
            .or_insert((event.freq_khz, event.freq_khz));
        entry.0 = entry.0.min(event.freq_khz);
        entry.1 = entry.1.max(event.freq_khz);
    }

    let Some((cpu, (min_freq, max_freq))) = by_cpu
        .into_iter()
        .filter(|(_, (_, max_freq))| *max_freq > 0)
        .map(|(cpu, (min_freq, max_freq))| {
            let drop_percent =
                ((max_freq.saturating_sub(min_freq)) as f64 / max_freq as f64) * 100.0;
            (cpu, (min_freq, max_freq, drop_percent))
        })
        .filter(|(_, (_, _, drop_percent))| *drop_percent >= config.cpu_freq_drop_percent)
        .max_by(|a, b| a.1.2.partial_cmp(&b.1.2).unwrap_or(Ordering::Equal))
        .map(|(cpu, (min_freq, max_freq, _))| (cpu, (min_freq, max_freq)))
    else {
        return;
    };

    push_supporting_evidence(
        candidates,
        cause,
        EvidenceItem {
            kind: EvidenceKind::CpuFrequency,
            strength: 0.25,
            message: format!(
                "CPU frequency drop near spike: cpu={} min={}kHz max={}kHz",
                cpu, min_freq, max_freq
            ),
            timestamp_ms: None,
            start_ns: None,
            end_ns: None,
        },
    );
}

fn push_migration_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    events: &[MigrationEventRecord],
    cluster_elapsed_range: Option<(u64, u64)>,
    config: DiagnosisConfig,
) {
    let Some(event) = events.iter().find(|event| {
        cluster_elapsed_range.is_none_or(|(min, max)| {
            event.elapsed_ms >= min.saturating_sub(config.migration_window_ms)
                && event.elapsed_ms <= max.saturating_add(config.migration_window_ms)
        })
    }) else {
        return;
    };

    push_supporting_evidence(
        candidates,
        cause,
        EvidenceItem {
            kind: EvidenceKind::Migration,
            strength: 0.20,
            message: format!(
                "task migration event near spike: tid={} cpu {}->{}",
                event.tid, event.from_cpu, event.to_cpu
            ),
            timestamp_ms: Some(event.elapsed_ms),
            start_ns: Some(event.timestamp_ns),
            end_ns: Some(event.timestamp_ns),
        },
    );
}

fn push_page_fault_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    intervals: &[IntervalRecord],
    config: DiagnosisConfig,
) {
    let Some(record) = intervals.iter().find(|record| {
        record.major_faults.saturating_add(record.minor_faults) >= config.page_fault_delta_threshold
    }) else {
        return;
    };

    push_supporting_evidence(
        candidates,
        cause,
        EvidenceItem {
            kind: EvidenceKind::PageFaults,
            strength: 0.20,
            message: "page fault activity near spike; treat as supporting evidence only".to_owned(),
            timestamp_ms: Some(record.elapsed_ms),
            start_ns: None,
            end_ns: None,
        },
    );
}

fn push_cpu_perf_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    intervals: &[IntervalRecord],
    cluster: &SpikeCluster,
    config: DiagnosisConfig,
) {
    let anchor = select_anchor(cluster);
    let anchor_process_pid = cluster
        .points
        .iter()
        .find(|point| point.task == anchor.task)
        .and_then(|point| point.process_pid);

    let mut matching = intervals
        .iter()
        .filter(|record| record.cpu_perf.is_some() && record.task == anchor.task)
        .collect::<Vec<_>>();

    if matching.is_empty()
        && let Some(process_pid) = anchor_process_pid
    {
        matching = intervals
            .iter()
            .filter(|record| record.cpu_perf.is_some() && record.process_pid == Some(process_pid))
            .collect();
    }

    if matching.is_empty() && is_cpu_perf_class(anchor.class) {
        matching = intervals
            .iter()
            .filter(|record| record.cpu_perf.is_some() && record.class == anchor.class)
            .collect();
    }

    let Some(record) = matching.into_iter().max_by(|a, b| {
        cpu_perf_evidence_score(a, config)
            .partial_cmp(&cpu_perf_evidence_score(b, config))
            .unwrap_or(Ordering::Equal)
    }) else {
        return;
    };

    let score = cpu_perf_evidence_score(record, config);
    if score <= 0.0 {
        return;
    }

    let Some(perf) = &record.cpu_perf else {
        return;
    };

    push_supporting_evidence(
        candidates,
        cause,
        EvidenceItem {
            kind: EvidenceKind::CpuPerf,
            strength: 0.25,
            message: format!(
                "CPU perf near spike: task={} ipc={} cache_mpki={} cache_miss_rate={}",
                record.task,
                format_optional_float(perf.ipc, 2),
                format_optional_float(perf.cache_mpki, 1),
                format_optional_percent(perf.cache_miss_rate),
            ),
            timestamp_ms: Some(record.elapsed_ms),
            start_ns: None,
            end_ns: None,
        },
    );
}

fn push_runtime_slice_supporting_evidence(
    candidates: &mut [DiagnosisCandidate],
    cause: StutterCause,
    runtime_slices: &[crate::metrics::RuntimeSliceRecord],
    cluster: &SpikeCluster,
    config: DiagnosisConfig,
) {
    let anchor = select_anchor(cluster);
    let Some(anchor_record) = runtime_slices
        .iter()
        .filter(|record| record.task == anchor.task)
        .max_by(|a, b| {
            a.wait_ratio
                .unwrap_or(0.0)
                .partial_cmp(&b.wait_ratio.unwrap_or(0.0))
                .unwrap_or(Ordering::Equal)
        })
    else {
        return;
    };

    if let Some(wait_ratio) = anchor_record.wait_ratio
        && wait_ratio >= config.runtime_wait_high_ratio
    {
        push_supporting_evidence(
            candidates,
            cause,
            EvidenceItem {
                kind: EvidenceKind::RuntimeRunqueueWait,
                strength: 0.35,
                message: format!(
                    "runtime slice context: anchor task={} waited on runqueue for {:.1}% of sample interval",
                    anchor_record.task,
                    wait_ratio * 100.0
                ),
                timestamp_ms: Some(anchor_record.elapsed_ms),
                start_ns: None,
                end_ns: None,
            },
        );
    }

    if let Some(runtime_ratio) = anchor_record.runtime_ratio
        && runtime_ratio >= config.runtime_high_ratio
    {
        push_supporting_evidence(
            candidates,
            cause,
            EvidenceItem {
                kind: EvidenceKind::RuntimeCpuUse,
                strength: 0.20,
                message: format!(
                    "runtime slice context: anchor task={} was running for {:.1}% of sample interval; this weakens a pure waiting-only explanation",
                    anchor_record.task,
                    runtime_ratio * 100.0
                ),
                timestamp_ms: Some(anchor_record.elapsed_ms),
                start_ns: None,
                end_ns: None,
            },
        );
    }
}

fn push_runtime_slice_context_candidates(
    candidates: &mut Vec<DiagnosisCandidate>,
    runtime_slices: &[crate::metrics::RuntimeSliceRecord],
    cluster: &SpikeCluster,
    config: DiagnosisConfig,
) {
    let cluster_tasks = cluster
        .points
        .iter()
        .map(|point| point.task)
        .collect::<std::collections::BTreeSet<_>>();

    let high_background_runtime = runtime_slices
        .iter()
        .filter(|record| !cluster_tasks.contains(&record.task))
        .filter(|record| record.runtime_ratio.unwrap_or(0.0) >= config.runtime_high_ratio)
        .max_by(|a, b| {
            a.runtime_ratio
                .unwrap_or(0.0)
                .partial_cmp(&b.runtime_ratio.unwrap_or(0.0))
                .unwrap_or(Ordering::Equal)
        });

    if let Some(record) = high_background_runtime {
        let score = (record.runtime_ratio.unwrap_or(0.0) as f32).min(0.35);
        push_candidate(
            candidates,
            StutterCause::CpuMonopolizationCandidate,
            score,
            EvidenceItem {
                kind: EvidenceKind::RuntimeCpuUse,
                strength: score,
                message: format!(
                    "runtime slice context: non-anchor task={} comm='{}' consumed {:.1}% CPU time near spike",
                    record.task,
                    record.comm,
                    record.runtime_ratio.unwrap_or(0.0) * 100.0
                ),
                timestamp_ms: Some(record.elapsed_ms),
                start_ns: None,
                end_ns: None,
            },
        );
    }

    let high_wait = runtime_slices
        .iter()
        .filter(|record| record.wait_ratio.unwrap_or(0.0) >= config.runtime_wait_high_ratio)
        .max_by(|a, b| {
            a.wait_ratio
                .unwrap_or(0.0)
                .partial_cmp(&b.wait_ratio.unwrap_or(0.0))
                .unwrap_or(Ordering::Equal)
        });

    if let Some(record) = high_wait
        && !candidates
            .iter()
            .any(|candidate| is_scheduler_cause(candidate.cause))
    {
        let score = (record.wait_ratio.unwrap_or(0.0) as f32).min(0.35);
        push_candidate(
            candidates,
            StutterCause::RuntimeWaitCandidate,
            score,
            EvidenceItem {
                kind: EvidenceKind::RuntimeRunqueueWait,
                strength: score,
                message: format!(
                    "runtime slice context: task={} comm='{}' waited on runqueue for {:.1}% of sample interval",
                    record.task,
                    record.comm,
                    record.wait_ratio.unwrap_or(0.0) * 100.0
                ),
                timestamp_ms: Some(record.elapsed_ms),
                start_ns: None,
                end_ns: None,
            },
        );
    }
}

fn cpu_perf_evidence_score(record: &IntervalRecord, config: DiagnosisConfig) -> f64 {
    let Some(perf) = &record.cpu_perf else {
        return 0.0;
    };

    let low_ipc_score = perf
        .ipc
        .filter(|ipc| *ipc < config.low_ipc_threshold)
        .map(|ipc| 10_000.0 + config.low_ipc_threshold - ipc)
        .unwrap_or(0.0);
    if low_ipc_score > 0.0 {
        return low_ipc_score;
    }

    perf.cache_mpki
        .filter(|mpki| *mpki > config.high_cache_mpki_threshold)
        .map(|mpki| mpki - config.high_cache_mpki_threshold)
        .unwrap_or(0.0)
}

fn is_cpu_perf_class(class: TaskClass) -> bool {
    matches!(
        class,
        TaskClass::Game | TaskClass::GameHelper | TaskClass::WineServer | TaskClass::GameScope
    )
}

fn format_optional_float(value: Option<f64>, decimals: usize) -> String {
    let Some(value) = value.filter(|value| value.is_finite()) else {
        return "-".to_owned();
    };
    format!("{value:.decimals$}")
}

fn format_optional_percent(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite())
        .map(|value| format!("{:.1}%", value * 100.0))
        .unwrap_or_else(|| "-".to_owned())
}

#[cfg(test)]
mod tests;
