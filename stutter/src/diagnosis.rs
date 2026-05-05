use std::cmp::Ordering;

use serde::Serialize;

use crate::{
    metrics::format_latency,
    process_tree::TaskClass,
    recorder::{BlockIoRecord, GpuSample, IntervalRecord, IrqEventRecord},
    report::{SpikeCluster, SpikePoint},
    session_io::RunArtifacts,
};

const IRQ_SIGNIFICANT_NS: u64 = 250_000; // start conservative
const BLOCK_IO_SIGNIFICANT_NS: u64 = 1_000_000;
const GPU_BUSY_BOUND_PERCENT: u32 = 95;
const SCHED_DELAY_SIGNIFICANT_NS: u64 = 2_000_000;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum StutterCause {
    CompositorSchedulerDelay,
    GameThreadSchedulerDelay,
    IrqDelayCandidate,
    GpuBoundCandidate,
    BlockIoCandidate,
    CpuPressureCandidate,
    Unknown,
}

impl StutterCause {
    pub fn priority(&self) -> u32 {
        match self {
            StutterCause::CompositorSchedulerDelay => 1,
            StutterCause::GameThreadSchedulerDelay => 2,
            StutterCause::IrqDelayCandidate => 3,
            StutterCause::GpuBoundCandidate => 4,
            StutterCause::BlockIoCandidate => 5,
            StutterCause::CpuPressureCandidate => 6,
            StutterCause::Unknown => 7,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[allow(dead_code)]
pub enum EvidenceKind {
    SchedulerDelay,
    IrqOverlap,
    GpuBusy,
    BlockIo,
    CpuPressure,
    ScxState,
    CpuFrequency,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvidenceItem {
    pub kind: EvidenceKind,
    pub strength: f32,
    pub message: String,
    pub timestamp_ms: Option<u128>,
    pub start_ns: Option<u64>,
    pub end_ns: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosisCandidate {
    pub cause: StutterCause,
    pub score: f32,
    pub confidence: Confidence,
    pub evidence: Vec<EvidenceItem>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum ClusterAnchorKind {
    Compositor,
    Game,
    Other,
}

#[derive(Debug, Clone, Serialize)]
pub struct ClusterAnchor {
    pub task: u32,
    pub class: TaskClass,
    pub comm: String,
    pub latency_ns: u64,
    pub kind: ClusterAnchorKind,
}

#[derive(Debug, Clone, Serialize)]
pub struct Diagnosis {
    pub cause: StutterCause, // Primary cause (for compatibility)
    pub confidence: Confidence,
    pub secondary_causes: Vec<StutterCause>,
    pub evidence: Vec<String>,
    pub primary: Option<DiagnosisCandidate>,
    pub candidates: Vec<DiagnosisCandidate>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FrameDiagnosis {
    pub frame_elapsed_ms: u128,
    pub frametime_ms: f64,
    pub diagnosis: Diagnosis,
}

#[derive(Clone, Debug)]
pub struct LiveDiagnosisEntry {
    pub elapsed_ms: u128,
    pub cause: StutterCause,
    pub confidence: Confidence,
    pub anchor_class: TaskClass,
    pub anchor_comm: String,
    pub evidence: Vec<String>,
}

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

fn finalize_diagnosis(mut candidates: Vec<DiagnosisCandidate>) -> Diagnosis {
    if candidates.is_empty() {
        return Diagnosis {
            cause: StutterCause::Unknown,
            confidence: Confidence::Low,
            secondary_causes: Vec::new(),
            evidence: vec!["no strong correlation found".to_owned()],
            primary: None,
            candidates: Vec::new(),
            summary: "no strong correlation found".to_owned(),
        };
    }

    for candidate in &mut candidates {
        candidate.score = clamp01(candidate.score);
        candidate.confidence = confidence_from_score(candidate.score);
        for evidence in &mut candidate.evidence {
            evidence.strength = clamp01(evidence.strength);
        }
    }

    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.cause.priority().cmp(&b.cause.priority()))
    });

    let primary = candidates[0].clone();

    let secondary_causes = candidates
        .iter()
        .skip(1)
        .map(|c| c.cause)
        .collect::<Vec<_>>();

    let evidence = candidates
        .iter()
        .flat_map(|c| c.evidence.iter().map(|e| e.message.clone()))
        .collect::<Vec<_>>();

    let summary = format!(
        "primary={:?} confidence={:?} score={:.2}",
        primary.cause, primary.confidence, primary.score
    );

    Diagnosis {
        cause: primary.cause,
        confidence: primary.confidence,
        secondary_causes,
        evidence,
        primary: Some(primary),
        candidates,
        summary,
    }
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

pub fn diagnose_cluster(
    cluster: &SpikeCluster,
    artifacts: &RunArtifacts,
    window_ns: u64,
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

    let mut candidates = Vec::new();

    // 1. Compositor scheduler delay
    // compositor/gamescope >2ms near frame spike => CompositorSchedulerDelay
    let compositor_point = cluster
        .points
        .iter()
        .filter(|p| is_compositor_point(p) && p.latency_ns > SCHED_DELAY_SIGNIFICANT_NS)
        .max_by_key(|p| p.latency_ns);

    if let Some(p) = compositor_point {
        let score = (p.latency_ns as f32 / 8_000_000.0).max(0.75);
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
    }

    // 2. Game thread scheduler delay
    // Game/Main/RenderThread >2ms near frame spike => GameThreadSchedulerDelay
    let game_point = cluster
        .points
        .iter()
        .filter(|p| is_game_point(p) && p.latency_ns > SCHED_DELAY_SIGNIFICANT_NS)
        .max_by_key(|p| p.latency_ns);

    if let Some(p) = game_point {
        let score = (p.latency_ns as f32 / 8_000_000.0).max(0.75);
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
    }

    // 3. IRQ overlap
    let max_irq = irq_events
        .iter()
        .filter(|e| e.duration_ns >= IRQ_SIGNIFICANT_NS)
        .max_by_key(|e| e.duration_ns);

    if let Some(irq) = max_irq {
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
                .filter(|busy| *busy >= GPU_BUSY_BOUND_PERCENT)
                .map(|busy| (s, busy))
        })
        .max_by_key(|(_, busy)| *busy);
    if let Some((sample, busy_percent)) = high_gpu {
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
        .filter(|e| e.duration_ns >= BLOCK_IO_SIGNIFICANT_NS)
        .max_by_key(|e| e.duration_ns);

    if let Some(io) = max_io {
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
        .filter(|r| r.cpu_psi_some > 50.0)
        .max_by(|a, b| {
            a.cpu_psi_some
                .partial_cmp(&b.cpu_psi_some)
                .unwrap_or(Ordering::Equal)
        });
    if let Some(r) = high_psi {
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

    finalize_diagnosis(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{process_tree::TaskClass, report::SpikePoint};

    fn spike_point(task: u32, class: TaskClass, comm: &str, latency_ns: u64) -> SpikePoint {
        SpikePoint {
            task,
            class,
            process_pid: Some(task),
            comm: comm.to_owned(),
            cpu: 0,
            wakeup_target_cpu: 0,
            latency_ns,
            wakeup_ns: 0,
            switch_ns: 0,
            target_pending_wakeups: 0,
            elapsed_ms: Some(100),
            scx_ops: None,
            scx_state: None,
        }
    }

    fn spike_cluster(points: Vec<SpikePoint>) -> SpikeCluster {
        let distinct_tasks = points.len();
        let min_switch_ns = points.iter().map(|p| p.switch_ns).min().unwrap_or(0);
        let max_switch_ns = points.iter().map(|p| p.switch_ns).max().unwrap_or(0);
        let max_latency_ns = points.iter().map(|p| p.latency_ns).max().unwrap_or(0);

        SpikeCluster {
            points,
            distinct_tasks,
            min_switch_ns,
            max_switch_ns,
            max_latency_ns,
            diagnosis: None,
            anchor_task: None,
            anchor_class: None,
            anchor_comm: None,
            anchor_kind: None,
        }
    }

    fn candidate(diagnosis: &Diagnosis, cause: StutterCause) -> &DiagnosisCandidate {
        diagnosis
            .candidates
            .iter()
            .find(|candidate| candidate.cause == cause)
            .unwrap()
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
        let irq = IrqEventRecord {
            elapsed_ms: None,
            irq: 137,
            cpu: 0,
            enter_ns: 0,
            exit_ns: 10_000,
            duration_ns: 10_000,
        };

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
        let irq_big = IrqEventRecord {
            elapsed_ms: None,
            irq: 137,
            cpu: 0,
            enter_ns: 0,
            exit_ns: 1_000_000,
            duration_ns: 1_000_000,
        };
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
        let irq = IrqEventRecord {
            elapsed_ms: None,
            irq: 137,
            cpu: 0,
            enter_ns: 0,
            exit_ns: 300_000,
            duration_ns: 300_000,
        };
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
        let irq = IrqEventRecord {
            elapsed_ms: None,
            irq: 137,
            cpu: 0,
            enter_ns: 0,
            exit_ns: 4_000_000,
            duration_ns: 4_000_000,
        };
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
}
