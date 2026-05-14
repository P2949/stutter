use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LifecycleClockSample {
    pub realtime_ms: u128,
    pub monotonic_ms: u128,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum DaemonLifecycleEvent {
    SuspendDetected,
    ResumeDetected { slept_ms: u128 },
    GpuReset,
    CompositorRestart,
    TargetPidRestart { old_pid: u32, new_pid: u32 },
    TargetExited { pid: u32 },
    CgroupMoved,
    CpuTopologyChanged,
}

impl DaemonLifecycleEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::SuspendDetected => "suspend_detected",
            Self::ResumeDetected { .. } => "resume_detected",
            Self::GpuReset => "gpu_reset",
            Self::CompositorRestart => "compositor_restart",
            Self::TargetPidRestart { .. } => "target_pid_restart",
            Self::TargetExited { .. } => "target_exited",
            Self::CgroupMoved => "cgroup_moved",
            Self::CpuTopologyChanged => "cpu_topology_changed",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DaemonLifecycleAction {
    PauseExperiments,
    RollbackActiveExperiment,
    FlushJournal,
    RefreshTopology,
    RefreshTargetIdentity,
    RefreshEbpfMaps,
    ClearMeasurementWindows,
    WaitStabilization,
    EnterObserveOnly,
}

impl DaemonLifecycleAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PauseExperiments => "pause_experiments",
            Self::RollbackActiveExperiment => "rollback_active_experiment",
            Self::FlushJournal => "flush_journal",
            Self::RefreshTopology => "refresh_topology",
            Self::RefreshTargetIdentity => "refresh_target_identity",
            Self::RefreshEbpfMaps => "refresh_ebpf_maps",
            Self::ClearMeasurementWindows => "clear_measurement_windows",
            Self::WaitStabilization => "wait_stabilization",
            Self::EnterObserveOnly => "enter_observe_only",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonLifecyclePolicy {
    pub suspend_resume_gap_threshold_ms: u128,
    pub resume_stabilization_ms: u64,
    pub rollback_active_experiment_on_suspend: bool,
    pub rollback_active_experiment_on_target_change: bool,
}

impl Default for DaemonLifecyclePolicy {
    fn default() -> Self {
        Self {
            suspend_resume_gap_threshold_ms: 5_000,
            resume_stabilization_ms: 10_000,
            rollback_active_experiment_on_suspend: true,
            rollback_active_experiment_on_target_change: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonLifecycleInputs {
    pub has_active_experiment: bool,
    pub active_experiment_has_rollback: bool,
    pub event: DaemonLifecycleEvent,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonLifecycleTransition {
    pub event: DaemonLifecycleEvent,
    pub actions: Vec<DaemonLifecycleAction>,
    pub reason_code: String,
    pub reason: String,
    pub stabilization_ms: Option<u64>,
    pub health_suspended_or_resumed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuspendResumeDetector {
    previous: Option<LifecycleClockSample>,
    gap_threshold_ms: u128,
}

impl SuspendResumeDetector {
    pub fn new(gap_threshold: Duration) -> Self {
        Self {
            previous: None,
            gap_threshold_ms: gap_threshold.as_millis(),
        }
    }

    pub fn with_policy(policy: &DaemonLifecyclePolicy) -> Self {
        Self {
            previous: None,
            gap_threshold_ms: policy.suspend_resume_gap_threshold_ms,
        }
    }

    pub fn observe(&mut self, sample: LifecycleClockSample) -> Option<DaemonLifecycleEvent> {
        let previous = self.previous.replace(sample)?;
        let realtime_delta = sample.realtime_ms.saturating_sub(previous.realtime_ms);
        let monotonic_delta = sample.monotonic_ms.saturating_sub(previous.monotonic_ms);
        let gap = realtime_delta.saturating_sub(monotonic_delta);

        (gap >= self.gap_threshold_ms)
            .then_some(DaemonLifecycleEvent::ResumeDetected { slept_ms: gap })
    }
}

pub fn evaluate_daemon_lifecycle_event(
    inputs: DaemonLifecycleInputs,
    policy: &DaemonLifecyclePolicy,
) -> DaemonLifecycleTransition {
    let mut actions = Vec::new();
    let mut stabilization_ms = None;
    let mut health_suspended_or_resumed = false;

    match &inputs.event {
        DaemonLifecycleEvent::SuspendDetected => {
            actions.push(DaemonLifecycleAction::PauseExperiments);
            actions.push(DaemonLifecycleAction::FlushJournal);
            if policy.rollback_active_experiment_on_suspend
                && inputs.has_active_experiment
                && inputs.active_experiment_has_rollback
            {
                actions.push(DaemonLifecycleAction::RollbackActiveExperiment);
            }
            health_suspended_or_resumed = true;
        }
        DaemonLifecycleEvent::ResumeDetected { .. } => {
            actions.push(DaemonLifecycleAction::RefreshTopology);
            actions.push(DaemonLifecycleAction::RefreshTargetIdentity);
            actions.push(DaemonLifecycleAction::RefreshEbpfMaps);
            actions.push(DaemonLifecycleAction::ClearMeasurementWindows);
            actions.push(DaemonLifecycleAction::WaitStabilization);
            stabilization_ms = Some(policy.resume_stabilization_ms);
            health_suspended_or_resumed = true;
        }
        DaemonLifecycleEvent::GpuReset | DaemonLifecycleEvent::CompositorRestart => {
            actions.push(DaemonLifecycleAction::PauseExperiments);
            actions.push(DaemonLifecycleAction::ClearMeasurementWindows);
            actions.push(DaemonLifecycleAction::WaitStabilization);
            stabilization_ms = Some(policy.resume_stabilization_ms);
        }
        DaemonLifecycleEvent::TargetPidRestart { .. }
        | DaemonLifecycleEvent::TargetExited { .. }
        | DaemonLifecycleEvent::CgroupMoved => {
            actions.push(DaemonLifecycleAction::RefreshTargetIdentity);
            actions.push(DaemonLifecycleAction::ClearMeasurementWindows);
            if policy.rollback_active_experiment_on_target_change && inputs.has_active_experiment {
                if inputs.active_experiment_has_rollback {
                    actions.push(DaemonLifecycleAction::RollbackActiveExperiment);
                } else {
                    actions.push(DaemonLifecycleAction::EnterObserveOnly);
                }
            }
        }
        DaemonLifecycleEvent::CpuTopologyChanged => {
            actions.push(DaemonLifecycleAction::RefreshTopology);
            actions.push(DaemonLifecycleAction::ClearMeasurementWindows);
            actions.push(DaemonLifecycleAction::EnterObserveOnly);
        }
    }

    DaemonLifecycleTransition {
        reason_code: inputs.event.as_str().to_owned(),
        reason: lifecycle_reason(&inputs.event),
        event: inputs.event,
        actions,
        stabilization_ms,
        health_suspended_or_resumed,
    }
}

fn lifecycle_reason(event: &DaemonLifecycleEvent) -> String {
    match event {
        DaemonLifecycleEvent::SuspendDetected => {
            "system suspend detected; experiments must pause and journal state must flush"
                .to_owned()
        }
        DaemonLifecycleEvent::ResumeDetected { slept_ms } => {
            format!(
                "system resume detected after approximately {slept_ms}ms; measurements must stabilize"
            )
        }
        DaemonLifecycleEvent::GpuReset => {
            "GPU reset detected; current measurement windows are no longer comparable".to_owned()
        }
        DaemonLifecycleEvent::CompositorRestart => {
            "compositor restart detected; foreground and frame context must be refreshed".to_owned()
        }
        DaemonLifecycleEvent::TargetPidRestart { old_pid, new_pid } => {
            format!("target process restarted from pid {old_pid} to pid {new_pid}")
        }
        DaemonLifecycleEvent::TargetExited { pid } => {
            format!("target process pid {pid} exited")
        }
        DaemonLifecycleEvent::CgroupMoved => {
            "target cgroup changed; workload identity must be refreshed".to_owned()
        }
        DaemonLifecycleEvent::CpuTopologyChanged => {
            "CPU topology changed; candidates must be revalidated".to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn active_inputs(event: DaemonLifecycleEvent) -> DaemonLifecycleInputs {
        DaemonLifecycleInputs {
            has_active_experiment: true,
            active_experiment_has_rollback: true,
            event,
        }
    }

    #[test]
    fn suspend_resume_detector_reports_large_realtime_gap() {
        let mut detector = SuspendResumeDetector::new(Duration::from_secs(5));

        assert_eq!(
            detector.observe(LifecycleClockSample {
                realtime_ms: 1_000,
                monotonic_ms: 1_000,
            }),
            None
        );
        assert_eq!(
            detector.observe(LifecycleClockSample {
                realtime_ms: 20_000,
                monotonic_ms: 2_000,
            }),
            Some(DaemonLifecycleEvent::ResumeDetected { slept_ms: 18_000 })
        );
    }

    #[test]
    fn suspend_transition_pauses_flushes_and_rolls_back_active_experiment() {
        let transition = evaluate_daemon_lifecycle_event(
            active_inputs(DaemonLifecycleEvent::SuspendDetected),
            &DaemonLifecyclePolicy::default(),
        );

        assert_eq!(transition.reason_code, "suspend_detected");
        assert!(transition.health_suspended_or_resumed);
        assert!(
            transition
                .actions
                .contains(&DaemonLifecycleAction::PauseExperiments)
        );
        assert!(
            transition
                .actions
                .contains(&DaemonLifecycleAction::FlushJournal)
        );
        assert!(
            transition
                .actions
                .contains(&DaemonLifecycleAction::RollbackActiveExperiment)
        );
    }

    #[test]
    fn resume_transition_refreshes_state_and_waits_for_stabilization() {
        let transition = evaluate_daemon_lifecycle_event(
            active_inputs(DaemonLifecycleEvent::ResumeDetected { slept_ms: 8_000 }),
            &DaemonLifecyclePolicy::default(),
        );

        assert_eq!(transition.reason_code, "resume_detected");
        assert_eq!(transition.stabilization_ms, Some(10_000));
        assert!(transition.health_suspended_or_resumed);
        assert!(
            transition
                .actions
                .contains(&DaemonLifecycleAction::RefreshTopology)
        );
        assert!(
            transition
                .actions
                .contains(&DaemonLifecycleAction::RefreshTargetIdentity)
        );
        assert!(
            transition
                .actions
                .contains(&DaemonLifecycleAction::RefreshEbpfMaps)
        );
        assert!(
            transition
                .actions
                .contains(&DaemonLifecycleAction::ClearMeasurementWindows)
        );
        assert!(
            transition
                .actions
                .contains(&DaemonLifecycleAction::WaitStabilization)
        );
    }

    #[test]
    fn target_restart_rolls_back_when_active_experiment_has_rollback() {
        let transition = evaluate_daemon_lifecycle_event(
            active_inputs(DaemonLifecycleEvent::TargetPidRestart {
                old_pid: 100,
                new_pid: 200,
            }),
            &DaemonLifecyclePolicy::default(),
        );

        assert_eq!(transition.reason_code, "target_pid_restart");
        assert!(
            transition
                .actions
                .contains(&DaemonLifecycleAction::RollbackActiveExperiment)
        );
        assert!(
            transition
                .actions
                .contains(&DaemonLifecycleAction::ClearMeasurementWindows)
        );
    }

    #[test]
    fn target_exit_without_rollback_enters_observe_only() {
        let transition = evaluate_daemon_lifecycle_event(
            DaemonLifecycleInputs {
                has_active_experiment: true,
                active_experiment_has_rollback: false,
                event: DaemonLifecycleEvent::TargetExited { pid: 100 },
            },
            &DaemonLifecyclePolicy::default(),
        );

        assert!(
            transition
                .actions
                .contains(&DaemonLifecycleAction::EnterObserveOnly)
        );
        assert!(
            !transition
                .actions
                .contains(&DaemonLifecycleAction::RollbackActiveExperiment)
        );
    }

    #[test]
    fn cpu_topology_change_forces_revalidation() {
        let transition = evaluate_daemon_lifecycle_event(
            DaemonLifecycleInputs {
                has_active_experiment: false,
                active_experiment_has_rollback: false,
                event: DaemonLifecycleEvent::CpuTopologyChanged,
            },
            &DaemonLifecyclePolicy::default(),
        );

        assert_eq!(transition.reason_code, "cpu_topology_changed");
        assert!(
            transition
                .actions
                .contains(&DaemonLifecycleAction::RefreshTopology)
        );
        assert!(
            transition
                .actions
                .contains(&DaemonLifecycleAction::EnterObserveOnly)
        );
    }
}
