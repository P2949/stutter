use serde::Serialize;

use crate::{
    metrics::format_latency,
    process_tree::TaskClass,
    recorder::{BlockIoRecord, FrameEvent, GpuSample, IntervalRecord, IrqEventRecord},
    report::{SpikeCluster, SpikePoint},
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
    irq_events: &[IrqEventRecord],
    gpu_samples: &[GpuSample],
    _frame_events: &[FrameEvent],
    io_events: &[BlockIoRecord],
    interval_records: &[IntervalRecord],
) -> Diagnosis {
    let mut evidence = Vec::new();
    let mut causes = Vec::new();

    // 1. Compositor scheduler delay
    // compositor/gamescope >2ms near frame spike => CompositorSchedulerDelay
    let compositor_points: Vec<_> = cluster
        .points
        .iter()
        .filter(|p| is_compositor_point(p) && p.latency_ns > SCHED_DELAY_SIGNIFICANT_NS)
        .collect();

    if !compositor_points.is_empty() {
        let p = compositor_points
            .iter()
            .max_by_key(|p| p.latency_ns)
            .unwrap();
        causes.push((StutterCause::CompositorSchedulerDelay, Confidence::High));
        evidence.push(format!(
            "compositor thread '{}' delayed by {}",
            p.comm,
            format_latency(p.latency_ns)
        ));
    }

    // 2. Game thread scheduler delay
    // Game/Main/RenderThread >2ms near frame spike => GameThreadSchedulerDelay
    let game_points: Vec<_> = cluster
        .points
        .iter()
        .filter(|p| is_game_point(p) && p.latency_ns > SCHED_DELAY_SIGNIFICANT_NS)
        .collect();

    if !game_points.is_empty() {
        let p = game_points.iter().max_by_key(|p| p.latency_ns).unwrap();
        causes.push((StutterCause::GameThreadSchedulerDelay, Confidence::High));
        evidence.push(format!(
            "game thread '{}' delayed by {}",
            p.comm,
            format_latency(p.latency_ns)
        ));
    }

    // 3. IRQ overlap
    let max_irq = irq_events
        .iter()
        .filter(|e| e.duration_ns >= IRQ_SIGNIFICANT_NS)
        .max_by_key(|e| e.duration_ns);

    if let Some(irq) = max_irq {
        causes.push((StutterCause::IrqDelayCandidate, Confidence::Medium));
        evidence.push(format!(
            "IRQ {} active during spike (max duration {})",
            irq.irq,
            format_latency(irq.duration_ns)
        ));
    }

    // 4. GPU bound
    let high_gpu = gpu_samples.iter().find(|s| {
        s.gpu_busy_percent
            .is_some_and(|p| p >= GPU_BUSY_BOUND_PERCENT)
    });
    if let Some(s) = high_gpu {
        causes.push((StutterCause::GpuBoundCandidate, Confidence::Medium));
        evidence.push(format!("GPU busy at {}%", s.gpu_busy_percent.unwrap()));
    }

    // 5. Block I/O
    let max_io = io_events
        .iter()
        .filter(|e| e.duration_ns >= BLOCK_IO_SIGNIFICANT_NS)
        .max_by_key(|e| e.duration_ns);

    if let Some(io) = max_io {
        causes.push((StutterCause::BlockIoCandidate, Confidence::Medium));
        evidence.push(format!(
            "block I/O active (max duration {})",
            format_latency(io.duration_ns)
        ));
    }

    // 6. CPU Pressure (PSI)
    let high_psi = interval_records.iter().find(|r| r.cpu_psi_some > 50.0);
    if let Some(r) = high_psi {
        causes.push((StutterCause::CpuPressureCandidate, Confidence::Medium));
        evidence.push(format!("high CPU PSI detected ({:.1}%)", r.cpu_psi_some));
    }

    if causes.is_empty() {
        return Diagnosis {
            cause: StutterCause::Unknown,
            confidence: Confidence::Low,
            secondary_causes: Vec::new(),
            evidence: vec!["no strong correlation found".to_owned()],
        };
    }

    // Sort causes by priority
    causes.sort_by_key(|(c, _)| c.priority());

    let (primary_cause, confidence) = causes[0];
    let secondary_causes: Vec<_> = causes[1..].iter().map(|(c, _)| *c).collect();

    Diagnosis {
        cause: primary_cause,
        confidence,
        secondary_causes,
        evidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{process_tree::TaskClass, report::SpikePoint};

    #[test]
    fn classifies_sway_compositor_delay_by_task_class_not_comm_name() {
        let p = SpikePoint {
            task: 123,
            class: TaskClass::Compositor,
            process_pid: Some(123),
            comm: "sway".to_owned(),
            cpu: 0,
            wakeup_target_cpu: 0,
            latency_ns: 3_000_000,
            wakeup_ns: 0,
            switch_ns: 0,
            target_pending_wakeups: 0,
            elapsed_ms: Some(100),
            scx_ops: None,
            scx_state: None,
        };
        let cluster = SpikeCluster {
            points: vec![p],
            distinct_tasks: 1,
            min_switch_ns: 0,
            max_switch_ns: 0,
            max_latency_ns: 3_000_000,
            diagnosis: None,
            anchor_task: None,
            anchor_class: None,
            anchor_comm: None,
            anchor_kind: None,
        };
        let d = diagnose_cluster(&cluster, &[], &[], &[], &[], &[]);
        assert_eq!(d.cause, StutterCause::CompositorSchedulerDelay);
    }

    #[test]
    fn chooses_compositor_as_primary_even_with_other_delays() {
        let p_compositor = SpikePoint {
            task: 123,
            class: TaskClass::Compositor,
            process_pid: Some(123),
            comm: "sway".to_owned(),
            cpu: 0,
            wakeup_target_cpu: 0,
            latency_ns: 3_000_000,
            wakeup_ns: 0,
            switch_ns: 0,
            target_pending_wakeups: 0,
            elapsed_ms: Some(100),
            scx_ops: None,
            scx_state: None,
        };
        let p_game = SpikePoint {
            task: 456,
            class: TaskClass::Game,
            process_pid: Some(456),
            comm: "RenderThread".to_owned(),
            cpu: 1,
            wakeup_target_cpu: 1,
            latency_ns: 10_000_000,
            wakeup_ns: 0,
            switch_ns: 0,
            target_pending_wakeups: 0,
            elapsed_ms: Some(100),
            scx_ops: None,
            scx_state: None,
        };
        let cluster = SpikeCluster {
            points: vec![p_compositor, p_game],
            distinct_tasks: 2,
            min_switch_ns: 0,
            max_switch_ns: 0,
            max_latency_ns: 10_000_000,
            diagnosis: None,
            anchor_task: None,
            anchor_class: None,
            anchor_comm: None,
            anchor_kind: None,
        };
        let d = diagnose_cluster(&cluster, &[], &[], &[], &[], &[]);
        assert_eq!(d.cause, StutterCause::CompositorSchedulerDelay);
        assert!(
            d.secondary_causes
                .contains(&StutterCause::GameThreadSchedulerDelay)
        );
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
    fn ignores_tiny_irq_events() {
        let p = SpikePoint {
            task: 123,
            class: TaskClass::Unknown,
            process_pid: Some(123),
            comm: "other".to_owned(),
            cpu: 0,
            wakeup_target_cpu: 0,
            latency_ns: 3_000_000,
            wakeup_ns: 0,
            switch_ns: 0,
            target_pending_wakeups: 0,
            elapsed_ms: Some(100),
            scx_ops: None,
            scx_state: None,
        };
        let cluster = SpikeCluster {
            points: vec![p],
            distinct_tasks: 1,
            min_switch_ns: 0,
            max_switch_ns: 0,
            max_latency_ns: 3_000_000,
            diagnosis: None,
            anchor_task: None,
            anchor_class: None,
            anchor_comm: None,
            anchor_kind: None,
        };

        // 10us IRQ should be ignored (threshold 250us)
        let irq = IrqEventRecord {
            elapsed_ms: None,
            irq: 137,
            cpu: 0,
            enter_ns: 0,
            exit_ns: 10_000,
            duration_ns: 10_000,
        };

        let d = diagnose_cluster(&cluster, &[irq], &[], &[], &[], &[]);
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
        let d2 = diagnose_cluster(&cluster, &[irq_big], &[], &[], &[], &[]);
        assert!(
            d2.secondary_causes
                .contains(&StutterCause::IrqDelayCandidate)
                || d2.cause == StutterCause::IrqDelayCandidate
        );
    }
}
