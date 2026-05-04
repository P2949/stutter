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

#[derive(Debug, Clone, Serialize)]
pub struct Diagnosis {
    pub cause: StutterCause,
    pub confidence: Confidence,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LiveDiagnosisEntry {
    pub timestamp_ms: u128,
    pub spike: crate::recorder::SpikeEvent,
    pub diagnosis: Diagnosis,
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
    let compositor_delay = cluster
        .points
        .iter()
        .filter(|p| is_compositor_point(p))
        .filter(|p| p.latency_ns > 2_000_000)
        .max_by_key(|p| p.latency_ns);

    if let Some(p) = compositor_delay {
        causes.push((StutterCause::CompositorSchedulerDelay, Confidence::High));
        evidence.push(format!(
            "compositor thread '{}' delayed by {}",
            p.comm,
            format_latency(p.latency_ns)
        ));
    }

    // 2. Game thread scheduler delay
    // Game/Main/RenderThread >2ms near frame spike => GameThreadSchedulerDelay
    let game_delay = cluster
        .points
        .iter()
        .filter(|p| is_game_point(p))
        .filter(|p| p.latency_ns > 2_000_000)
        .max_by_key(|p| p.latency_ns);

    if let Some(p) = game_delay {
        causes.push((StutterCause::GameThreadSchedulerDelay, Confidence::High));
        evidence.push(format!(
            "game thread '{}' delayed by {}",
            p.comm,
            format_latency(p.latency_ns)
        ));
    }

    // 3. IRQ overlap
    // IRQ overlap near spike => IrqDelayCandidate
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
    // GPU busy >95% near spike => GpuBoundCandidate
    let high_gpu = gpu_samples
        .iter()
        .find(|s| s.gpu_busy_percent.is_some_and(|p| p > 95));
    if let Some(s) = high_gpu {
        causes.push((StutterCause::GpuBoundCandidate, Confidence::Medium));
        evidence.push(format!("GPU busy at {}%", s.gpu_busy_percent.unwrap()));
    }

    // 5. Block I/O
    // block I/O overlap near spike => BlockIoCandidate
    if !io_events.is_empty() {
        let max_io = io_events.iter().max_by_key(|e| e.duration_ns).unwrap();
        causes.push((StutterCause::BlockIoCandidate, Confidence::Medium));
        evidence.push(format!(
            "block I/O active (max duration {})",
            format_latency(max_io.duration_ns)
        ));
    }

    // 6. CPU Pressure (PSI)
    // PSI CPU high => CpuPressureCandidate
    // We check interval records near the cluster
    let high_psi = interval_records.iter().find(|r| r.cpu_psi_some > 50.0);
    if let Some(r) = high_psi {
        causes.push((StutterCause::CpuPressureCandidate, Confidence::Medium));
        evidence.push(format!("high CPU PSI detected ({:.1}%)", r.cpu_psi_some));
    }

    if causes.is_empty() {
        return Diagnosis {
            cause: StutterCause::Unknown,
            confidence: Confidence::Low,
            evidence: vec!["no strong correlation found".to_owned()],
        };
    }

    if causes.len() > 1 {
        return Diagnosis {
            cause: StutterCause::Mixed,
            confidence: Confidence::Medium,
            evidence,
        };
    }

    let (cause, confidence) = causes[0];
    Diagnosis {
        cause,
        confidence,
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
        };
        let d = diagnose_cluster(&cluster, &[], &[], &[], &[], &[]);
        assert_eq!(d.cause, StutterCause::CompositorSchedulerDelay);
    }

    #[test]
    fn classifies_gamescope_as_compositor_path() {
        let p = SpikePoint {
            task: 456,
            class: TaskClass::GameScope,
            process_pid: Some(456),
            comm: "gamescope".to_owned(),
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
        };
        let d = diagnose_cluster(&cluster, &[], &[], &[], &[], &[]);
        assert_eq!(d.cause, StutterCause::CompositorSchedulerDelay);
    }
}
