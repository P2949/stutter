use serde::Serialize;

use crate::{
    metrics::format_latency,
    process_tree::TaskClass,
    recorder::{BlockIoRecord, FrameEvent, GpuSample, IntervalRecord, IrqEventRecord},
    report::{SpikeCluster, SpikePoint},
};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub enum StutterCause {
    GameThreadSchedulerDelay,
    CompositorSchedulerDelay,
    IrqDelayCandidate,
    GpuBoundCandidate,
    BlockIoCandidate,
    CpuPressureCandidate,
    Mixed,
    Unknown,
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
    pub cause: StutterCause, // Legacy field for compatibility
    pub primary_cause: StutterCause,
    pub confidence: Confidence,
    pub secondary_causes: Vec<StutterCause>,
    pub evidence: Vec<String>,
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
        .filter(|p| is_compositor_point(p) && p.latency_ns > 2_000_000)
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
        .filter(|p| is_game_point(p) && p.latency_ns > 2_000_000)
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

    let anchor = select_anchor(cluster);

    // 1. Compositor scheduler delay
    // compositor/gamescope >2ms near frame spike => CompositorSchedulerDelay
    let compositor_points: Vec<_> = cluster
        .points
        .iter()
        .filter(|p| is_compositor_point(p) && p.latency_ns > 2_000_000)
        .collect();

    if !compositor_points.is_empty() {
        let p = compositor_points.iter().max_by_key(|p| p.latency_ns).unwrap();
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
        .filter(|p| is_game_point(p) && p.latency_ns > 2_000_000)
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
    if !irq_events.is_empty() {
        let max_irq = irq_events.iter().max_by_key(|e| e.duration_ns).unwrap();
        causes.push((StutterCause::IrqDelayCandidate, Confidence::Medium));
        evidence.push(format!(
            "IRQ {} active during spike (max duration {})",
            max_irq.irq,
            format_latency(max_irq.duration_ns)
        ));
    }

    // 4. GPU bound
    let high_gpu = gpu_samples
        .iter()
        .find(|s| s.gpu_busy_percent.is_some_and(|p| p > 95));
    if let Some(s) = high_gpu {
        causes.push((StutterCause::GpuBoundCandidate, Confidence::Medium));
        evidence.push(format!("GPU busy at {}%", s.gpu_busy_percent.unwrap()));
    }

    // 5. Block I/O
    if !io_events.is_empty() {
        let max_io = io_events.iter().max_by_key(|e| e.duration_ns).unwrap();
        causes.push((StutterCause::BlockIoCandidate, Confidence::Medium));
        evidence.push(format!(
            "block I/O active (max duration {})",
            format_latency(max_io.duration_ns)
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
            primary_cause: StutterCause::Unknown,
            confidence: Confidence::Low,
            secondary_causes: Vec::new(),
            evidence: vec!["no strong correlation found".to_owned()],
        };
    }

    // Determine primary cause based on anchor
    let primary_cause = match anchor.kind {
        ClusterAnchorKind::Compositor => StutterCause::CompositorSchedulerDelay,
        ClusterAnchorKind::Game => StutterCause::GameThreadSchedulerDelay,
        ClusterAnchorKind::Other => causes[0].0,
    };

    let confidence = causes
        .iter()
        .find(|(c, _)| *c == primary_cause)
        .map(|(_, conf)| *conf)
        .unwrap_or(Confidence::Low);

    let secondary_causes: Vec<_> = causes
        .into_iter()
        .map(|(c, _)| c)
        .filter(|c| *c != primary_cause)
        .collect();

    Diagnosis {
        cause: if secondary_causes.is_empty() {
            primary_cause
        } else {
            StutterCause::Mixed
        },
        primary_cause,
        confidence,
        secondary_causes,
        evidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_tree::TaskClass;
    use crate::report::SpikePoint;

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
        assert_eq!(d.primary_cause, StutterCause::CompositorSchedulerDelay);
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
        assert_eq!(d.primary_cause, StutterCause::CompositorSchedulerDelay);
        assert!(d.secondary_causes.contains(&StutterCause::GameThreadSchedulerDelay));
        assert!(d.evidence.iter().any(|e| e.contains("compositor thread 'sway' delayed by 3.000ms")));
        assert!(d.evidence.iter().any(|e| e.contains("game thread 'RenderThread' delayed by 10.000ms")));
    }
}
