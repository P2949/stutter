use super::{
    policy::DaemonMode,
    state::{
        DAEMON_STATE_SCHEMA_VERSION, DaemonDecisionState, DaemonDegradedStatus,
        DaemonExperimentState, DaemonFaultState, DaemonPhase, DaemonRollbackState, DaemonState,
        DaemonTargetState,
    },
};
use crate::{
    actions::{RollbackToken, SafetyClass},
    autotune::startup_recovery::StartupRecoveryOutcome,
    remote::RemoteMonitorRequest,
};

pub struct StartupRecoveryDaemonStateInput<'a> {
    pub phase: DaemonPhase,
    pub decision: &'a str,
    pub reason: String,
    pub experiment_id: &'a str,
    pub action_id: &'a str,
    pub rollback_token: Option<&'a RollbackToken>,
    pub rollback_available: bool,
    pub include_active_experiment: bool,
    pub faulted: bool,
    pub manual_restore_command: Option<&'a str>,
}

pub fn daemon_decision_state(decision: &str, reason: impl Into<String>) -> DaemonDecisionState {
    DaemonDecisionState {
        decision: decision.to_owned(),
        reason: reason.into(),
        unix_nanos: Some(crate::audit::unix_nanos_now()),
        score_total: None,
    }
}

pub fn daemon_state_for_agent_fault(
    mode: DaemonMode,
    decision: &str,
    reason: impl Into<String>,
) -> DaemonState {
    let reason = reason.into();

    DaemonState {
        mode,
        phase: DaemonPhase::Faulted,
        last_decision: Some(daemon_decision_state(decision, reason.clone())),
        degraded: vec![DaemonDegradedStatus {
            category: "agent".to_owned(),
            message: reason.clone(),
        }],
        faulted: Some(DaemonFaultState {
            reason,
            manual_restore_command: Some("stutter autotune restore".to_owned()),
        }),
        ..DaemonState::default()
    }
}

pub fn daemon_state_from_startup_recovery(outcome: &StartupRecoveryOutcome) -> DaemonState {
    match outcome {
        StartupRecoveryOutcome::Clean => DaemonState::default(),
        StartupRecoveryOutcome::Recovered {
            experiment_id,
            action_id,
            affected_tasks,
            manual_restore_command,
        } => DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            phase: DaemonPhase::Cooldown,
            last_decision: Some(daemon_decision_state(
                "startup_recovery_recovered",
                format!(
                    "recovered experiment_id={experiment_id} action_id={action_id} affected_tasks={affected_tasks} manual_restore_command=\"{manual_restore_command}\""
                ),
            )),
            ..DaemonState::default()
        },
        StartupRecoveryOutcome::RollbackDisabled {
            experiment_id,
            action_id,
            manual_restore_command,
        } => DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            phase: DaemonPhase::Faulted,
            active_experiment: Some(DaemonExperimentState {
                experiment_id: experiment_id.clone(),
                action_id: action_id.clone(),
                candidate_name: None,
                safety_class: SafetyClass::ReversibleLowRisk,
                started_unix_nanos: None,
            }),
            active_rollback: Some(DaemonRollbackState {
                action_id: action_id.clone(),
                rollback_available: true,
                token: None,
                manual_restore_command: Some(manual_restore_command.clone()),
            }),
            last_decision: Some(daemon_decision_state(
                "startup_recovery_rollback_disabled",
                format!(
                    "rollback disabled for experiment_id={experiment_id} action_id={action_id} manual_restore_command=\"{manual_restore_command}\""
                ),
            )),
            degraded: vec![DaemonDegradedStatus {
                category: "startup_recovery".to_owned(),
                message: "rollback disabled".to_owned(),
            }],
            faulted: Some(DaemonFaultState {
                reason: "startup recovery rollback disabled".to_owned(),
                manual_restore_command: Some(manual_restore_command.clone()),
            }),
            ..DaemonState::default()
        },
        StartupRecoveryOutcome::ApplyingWithoutRollback {
            experiment_id,
            action_id,
        } => DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            phase: DaemonPhase::Faulted,
            active_experiment: Some(DaemonExperimentState {
                experiment_id: experiment_id.clone(),
                action_id: action_id.clone(),
                candidate_name: None,
                safety_class: SafetyClass::ReversibleLowRisk,
                started_unix_nanos: None,
            }),
            last_decision: Some(daemon_decision_state(
                "startup_recovery_applying_without_rollback",
                format!(
                    "applying journal without rollback token experiment_id={experiment_id} action_id={action_id}"
                ),
            )),
            degraded: vec![DaemonDegradedStatus {
                category: "startup_recovery".to_owned(),
                message: "applying journal without rollback token".to_owned(),
            }],
            faulted: Some(DaemonFaultState {
                reason: "startup recovery found applying journal without rollback token".to_owned(),
                manual_restore_command: Some("stutter autotune restore".to_owned()),
            }),
            ..DaemonState::default()
        },
        StartupRecoveryOutcome::Faulted {
            experiment_id,
            action_id,
            manual_restore_command,
            reason,
        } => DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            phase: DaemonPhase::Faulted,
            active_experiment: Some(DaemonExperimentState {
                experiment_id: experiment_id.clone(),
                action_id: action_id.clone(),
                candidate_name: None,
                safety_class: SafetyClass::ReversibleLowRisk,
                started_unix_nanos: None,
            }),
            last_decision: Some(daemon_decision_state(
                "startup_recovery_faulted",
                reason.clone(),
            )),
            degraded: vec![DaemonDegradedStatus {
                category: "startup_recovery".to_owned(),
                message: reason.clone(),
            }],
            faulted: Some(DaemonFaultState {
                reason: reason.clone(),
                manual_restore_command: Some(manual_restore_command.clone()),
            }),
            ..DaemonState::default()
        },
    }
}

pub fn daemon_state_for_record_start(
    request: &RemoteMonitorRequest,
    run_id: &str,
    target_count: usize,
) -> DaemonState {
    DaemonState {
        mode: DaemonMode::Observe,
        phase: DaemonPhase::Observe,
        active_target: daemon_target_from_record_request(request),
        last_decision: Some(daemon_decision_state(
            "record_started",
            format!("remote recording run_id={run_id} target_count={target_count}"),
        )),
        ..DaemonState::default()
    }
}

pub fn daemon_state_for_startup_recovery_snapshot(
    input: StartupRecoveryDaemonStateInput<'_>,
) -> DaemonState {
    let manual_restore_command = input
        .manual_restore_command
        .map(str::to_owned)
        .unwrap_or_else(|| "stutter autotune restore".to_owned());
    let safety_class = input
        .rollback_token
        .map(safety_class_for_rollback_token)
        .unwrap_or(SafetyClass::ReversibleLowRisk);
    let active_experiment = if input.include_active_experiment {
        Some(DaemonExperimentState {
            experiment_id: input.experiment_id.to_owned(),
            action_id: input.action_id.to_owned(),
            candidate_name: candidate_name_from_action_id(input.action_id),
            safety_class,
            started_unix_nanos: None,
        })
    } else {
        None
    };
    let active_rollback = input.rollback_token.map(|token| DaemonRollbackState {
        action_id: input.action_id.to_owned(),
        rollback_available: input.rollback_available,
        token: Some(token.clone()),
        manual_restore_command: Some(manual_restore_command.clone()),
    });
    let degraded = if input.faulted {
        vec![DaemonDegradedStatus {
            category: "startup_recovery".to_owned(),
            message: input.reason.clone(),
        }]
    } else {
        Vec::new()
    };
    let faulted_state = if input.faulted {
        Some(DaemonFaultState {
            reason: input.reason.clone(),
            manual_restore_command: Some(manual_restore_command.clone()),
        })
    } else {
        None
    };

    DaemonState {
        schema_version: DAEMON_STATE_SCHEMA_VERSION,
        mode: DaemonMode::ApplyLowRisk,
        phase: input.phase,
        active_experiment,
        active_rollback,
        last_decision: Some(DaemonDecisionState {
            decision: input.decision.to_owned(),
            reason: input.reason,
            unix_nanos: Some(crate::audit::unix_nanos_now()),
            score_total: None,
        }),
        degraded,
        faulted: faulted_state,
        ..DaemonState::default()
    }
}

fn daemon_target_from_record_request(request: &RemoteMonitorRequest) -> Option<DaemonTargetState> {
    let root_pid = request
        .tree_pids
        .first()
        .copied()
        .or_else(|| request.target_pids.first().copied());
    let active_targets = request.target_pids.len() + request.tree_pids.len();
    let comm = request.include_comm.first().cloned();

    if root_pid.is_none() && active_targets == 0 && comm.is_none() {
        return None;
    }

    Some(DaemonTargetState {
        root_pid,
        active_targets,
        comm,
    })
}

pub fn safety_class_for_rollback_token(token: &RollbackToken) -> SafetyClass {
    match token {
        RollbackToken::CpuAffinityRestoreFile { .. } => SafetyClass::ReversibleLowRisk,
        RollbackToken::NiceRestore { .. }
        | RollbackToken::IoPrioRestore { .. }
        | RollbackToken::UclampRestore { .. }
        | RollbackToken::IrqAffinityRestore { .. } => SafetyClass::ReversibleMediumRisk,
        RollbackToken::CgroupRestore { .. }
        | RollbackToken::CpuPowerRestore { .. }
        | RollbackToken::VmKnobRestore { .. }
        | RollbackToken::GpuPowerRestore { .. }
        | RollbackToken::SysfsRestore { .. } => SafetyClass::HighRisk,
    }
}

fn candidate_name_from_action_id(action_id: &str) -> Option<String> {
    action_id
        .split_once(':')
        .map(|(_, candidate)| candidate.to_owned())
        .filter(|candidate| !candidate.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn remote_monitor_request() -> RemoteMonitorRequest {
        RemoteMonitorRequest {
            target_pids: vec![2222],
            tree_pids: vec![1111],
            exclude_tree_pids: Vec::new(),
            duration_seconds: Some(5),
            spike_us: None,
            summary_ms: None,
            include_comm: vec!["Game.exe".to_owned()],
            exclude_comm: Vec::new(),
            hwmon: false,
            cpu_freq: false,
            faults: false,
            stat_wait: false,
            block_io: false,
            runtime_slices: false,
            runtime_slices_max_tasks: None,
            irq_latency: false,
            irqs: Vec::new(),
            foreground_window: false,
            focus_source: None,
            foreground_source: None,
            foreground_poll_ms: None,
            foreground_max_stale_ms: None,
            foreground_include_title: false,
            record: false,
            run_name: None,
        }
    }

    #[test]
    fn agent_fault_builder_preserves_fault_shape() {
        let state = daemon_state_for_agent_fault(
            DaemonMode::ApplyLowRisk,
            "remote_start_rejected",
            "policy rejected request",
        );

        assert_eq!(state.mode, DaemonMode::ApplyLowRisk);
        assert_eq!(state.phase, DaemonPhase::Faulted);
        assert_eq!(
            state
                .last_decision
                .as_ref()
                .map(|decision| decision.decision.as_str()),
            Some("remote_start_rejected")
        );
        assert_eq!(
            state.faulted.as_ref().map(|fault| fault.reason.as_str()),
            Some("policy rejected request")
        );
        assert_eq!(
            state
                .faulted
                .as_ref()
                .and_then(|fault| fault.manual_restore_command.as_deref()),
            Some("stutter autotune restore")
        );
    }

    #[test]
    fn startup_recovery_outcome_builder_preserves_rollback_disabled_shape() {
        let state = daemon_state_from_startup_recovery(&StartupRecoveryOutcome::RollbackDisabled {
            experiment_id: "experiment-1".to_owned(),
            action_id: "cpu-affinity-profile:game-main".to_owned(),
            manual_restore_command: "stutter restore".to_owned(),
        });

        assert_eq!(state.mode, DaemonMode::ApplyLowRisk);
        assert_eq!(state.phase, DaemonPhase::Faulted);
        assert_eq!(
            state
                .active_experiment
                .as_ref()
                .map(|experiment| experiment.action_id.as_str()),
            Some("cpu-affinity-profile:game-main")
        );
        assert_eq!(
            state
                .active_rollback
                .as_ref()
                .map(|rollback| rollback.rollback_available),
            Some(true)
        );
        assert_eq!(
            state.faulted.as_ref().map(|fault| fault.reason.as_str()),
            Some("startup recovery rollback disabled")
        );
    }

    #[test]
    fn record_start_builder_preserves_target_identity() {
        let request = remote_monitor_request();

        let state = daemon_state_for_record_start(&request, "run-1", 3);

        assert_eq!(state.mode, DaemonMode::Observe);
        assert_eq!(state.phase, DaemonPhase::Observe);
        assert_eq!(
            state.active_target,
            Some(DaemonTargetState {
                root_pid: Some(1111),
                active_targets: 2,
                comm: Some("Game.exe".to_owned()),
            })
        );
        assert_eq!(
            state
                .last_decision
                .as_ref()
                .map(|decision| decision.reason.as_str()),
            Some("remote recording run_id=run-1 target_count=3")
        );
    }

    #[test]
    fn startup_recovery_snapshot_builder_preserves_fault_and_rollback_token() {
        let rollback_token = RollbackToken::CpuAffinityRestoreFile {
            path: PathBuf::from("/tmp/stutter-restore.json"),
            affected_tasks: 31,
        };

        let state = daemon_state_for_startup_recovery_snapshot(StartupRecoveryDaemonStateInput {
            phase: DaemonPhase::Faulted,
            decision: "faulted",
            reason: "startup crash recovery rollback failed".to_owned(),
            experiment_id: "experiment-1",
            action_id: "cpu-affinity-profile:game-main",
            rollback_token: Some(&rollback_token),
            rollback_available: true,
            include_active_experiment: true,
            faulted: true,
            manual_restore_command: Some("stutter restore"),
        });

        assert_eq!(state.phase, DaemonPhase::Faulted);
        assert_eq!(
            state
                .active_experiment
                .as_ref()
                .and_then(|experiment| experiment.candidate_name.as_deref()),
            Some("game-main")
        );
        assert_eq!(
            state
                .active_rollback
                .as_ref()
                .and_then(|rollback| rollback.token.as_ref()),
            Some(&rollback_token)
        );
        assert_eq!(
            state.faulted.as_ref().map(|fault| fault.reason.as_str()),
            Some("startup crash recovery rollback failed")
        );
    }
}
