use serde::{Deserialize, Serialize};

use crate::daemon::{DaemonPhase, DaemonState, SystemHealthSnapshot, SystemHealthState};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonWatchdogConfig {
    pub max_drop_counters_total: u64,
    pub max_action_phase_elapsed_ms: u64,
    pub max_experiment_elapsed_ms: u64,
    pub max_shutdown_rollback_elapsed_ms: u64,
}

impl Default for DaemonWatchdogConfig {
    fn default() -> Self {
        Self {
            max_drop_counters_total: 10_000,
            max_action_phase_elapsed_ms: 30_000,
            max_experiment_elapsed_ms: 10 * 60 * 1_000,
            max_shutdown_rollback_elapsed_ms: 30_000,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonWatchdogInputs {
    pub phase: DaemonPhase,
    pub health: SystemHealthSnapshot,
    pub ringbuf_alive: bool,
    pub event_rate_per_minute: Option<u64>,
    pub drop_counters_total: u64,
    pub target_refresh_ok: bool,
    pub recorder_writable: bool,
    pub rollback_journal_clean: bool,
    pub has_active_experiment: bool,
    pub has_active_rollback: bool,
    pub action_phase_elapsed_ms: Option<u64>,
    pub experiment_elapsed_ms: Option<u64>,
    pub shutdown_rollback_elapsed_ms: Option<u64>,
}

impl DaemonWatchdogInputs {
    pub fn from_state_and_health(state: &DaemonState, health: &SystemHealthSnapshot) -> Self {
        Self {
            phase: state.phase,
            health: health.clone(),
            ringbuf_alive: health.state != SystemHealthState::InstrumentationBroken,
            event_rate_per_minute: None,
            drop_counters_total: health.inputs.ebpf_dropped_events,
            target_refresh_ok: state.active_target.is_some()
                || state.active_experiment.is_none()
                || matches!(
                    state.phase,
                    DaemonPhase::Disabled
                        | DaemonPhase::Init
                        | DaemonPhase::Recover
                        | DaemonPhase::Paused
                        | DaemonPhase::Shutdown
                ),
            recorder_writable: !matches!(health.state, SystemHealthState::LowDisk),
            rollback_journal_clean: state.active_rollback.is_none()
                || state.active_experiment.is_some(),
            has_active_experiment: state.active_experiment.is_some(),
            has_active_rollback: state.active_rollback.is_some(),
            action_phase_elapsed_ms: None,
            experiment_elapsed_ms: None,
            shutdown_rollback_elapsed_ms: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DaemonSelfHealingAction {
    RestartMonitorSubsystem,
    ClearStaleTarget,
    PauseAutotune,
    RollbackActiveExperiment,
    EnterObserveOnly,
    FaultHard,
}

impl DaemonSelfHealingAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RestartMonitorSubsystem => "restart_monitor_subsystem",
            Self::ClearStaleTarget => "clear_stale_target",
            Self::PauseAutotune => "pause_autotune",
            Self::RollbackActiveExperiment => "rollback_active_experiment",
            Self::EnterObserveOnly => "enter_observe_only",
            Self::FaultHard => "fault_hard",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonWatchdogIssue {
    pub reason_code: String,
    pub message: String,
    pub recommended_actions: Vec<DaemonSelfHealingAction>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonWatchdogReport {
    pub ok: bool,
    pub phase: DaemonPhase,
    pub issues: Vec<DaemonWatchdogIssue>,
    pub recommended_actions: Vec<DaemonSelfHealingAction>,
}

pub fn evaluate_daemon_watchdog(
    inputs: DaemonWatchdogInputs,
    config: &DaemonWatchdogConfig,
) -> DaemonWatchdogReport {
    let mut issues = Vec::new();

    if !inputs.ringbuf_alive {
        issues.push(issue(
            "ringbuf_not_alive",
            "event ring buffer is not alive",
            &[
                DaemonSelfHealingAction::RestartMonitorSubsystem,
                DaemonSelfHealingAction::PauseAutotune,
            ],
        ));
    }

    if inputs.event_rate_per_minute == Some(0) && !inputs.phase.is_terminal() {
        issues.push(issue(
            "event_rate_zero",
            "daemon is active but no monitor events are arriving",
            &[
                DaemonSelfHealingAction::RestartMonitorSubsystem,
                DaemonSelfHealingAction::PauseAutotune,
            ],
        ));
    }

    if inputs.drop_counters_total > config.max_drop_counters_total {
        issues.push(issue(
            "drop_counters_high",
            "event drop counters exceed watchdog budget",
            &[
                DaemonSelfHealingAction::PauseAutotune,
                DaemonSelfHealingAction::EnterObserveOnly,
            ],
        ));
    }

    if !inputs.target_refresh_ok {
        issues.push(issue(
            "target_refresh_stalled",
            "target refresh is stale or failed",
            &[
                DaemonSelfHealingAction::ClearStaleTarget,
                DaemonSelfHealingAction::PauseAutotune,
            ],
        ));
    }

    if !inputs.recorder_writable {
        issues.push(issue(
            "recorder_unwritable",
            "recorder or audit storage is not writable",
            &[
                DaemonSelfHealingAction::PauseAutotune,
                DaemonSelfHealingAction::EnterObserveOnly,
            ],
        ));
    }

    if !inputs.rollback_journal_clean {
        issues.push(issue(
            "rollback_journal_dirty",
            "rollback journal is dirty outside an active experiment",
            &[
                DaemonSelfHealingAction::RollbackActiveExperiment,
                DaemonSelfHealingAction::FaultHard,
            ],
        ));
    }

    if !inputs.health.ok_for_apply {
        let mut actions = vec![DaemonSelfHealingAction::EnterObserveOnly];
        if inputs.has_active_rollback {
            actions.insert(0, DaemonSelfHealingAction::RollbackActiveExperiment);
        }
        issues.push(issue(
            inputs
                .health
                .reason_code
                .as_deref()
                .unwrap_or("health_blocks_apply"),
            "system health blocks apply mode",
            &actions,
        ));
    }

    if inputs
        .action_phase_elapsed_ms
        .is_some_and(|elapsed| elapsed > config.max_action_phase_elapsed_ms)
    {
        let mut actions = Vec::new();
        if inputs.has_active_rollback {
            actions.push(DaemonSelfHealingAction::RollbackActiveExperiment);
        }
        actions.push(DaemonSelfHealingAction::FaultHard);
        issues.push(issue(
            "action_phase_timeout",
            "action runner phase exceeded watchdog timeout",
            &actions,
        ));
    }

    if inputs
        .experiment_elapsed_ms
        .is_some_and(|elapsed| elapsed > config.max_experiment_elapsed_ms)
    {
        issues.push(issue(
            "experiment_timeout",
            "active experiment exceeded watchdog timeout",
            &[
                DaemonSelfHealingAction::RollbackActiveExperiment,
                DaemonSelfHealingAction::PauseAutotune,
            ],
        ));
    }

    if inputs
        .shutdown_rollback_elapsed_ms
        .is_some_and(|elapsed| elapsed > config.max_shutdown_rollback_elapsed_ms)
    {
        issues.push(issue(
            "shutdown_rollback_timeout",
            "shutdown rollback exceeded watchdog timeout",
            &[DaemonSelfHealingAction::FaultHard],
        ));
    }

    let mut recommended_actions = Vec::new();
    for action in issues
        .iter()
        .flat_map(|issue| issue.recommended_actions.iter().copied())
    {
        if !recommended_actions.contains(&action) {
            recommended_actions.push(action);
        }
    }

    DaemonWatchdogReport {
        ok: issues.is_empty(),
        phase: inputs.phase,
        issues,
        recommended_actions,
    }
}

fn issue(
    reason_code: impl Into<String>,
    message: impl Into<String>,
    actions: &[DaemonSelfHealingAction],
) -> DaemonWatchdogIssue {
    DaemonWatchdogIssue {
        reason_code: reason_code.into(),
        message: message.into(),
        recommended_actions: actions.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::{DaemonExperimentState, DaemonMode, DaemonRollbackState};

    fn healthy_inputs() -> DaemonWatchdogInputs {
        DaemonWatchdogInputs {
            phase: DaemonPhase::Observe,
            health: SystemHealthSnapshot::default(),
            ringbuf_alive: true,
            event_rate_per_minute: Some(60),
            drop_counters_total: 0,
            target_refresh_ok: true,
            recorder_writable: true,
            rollback_journal_clean: true,
            has_active_experiment: false,
            has_active_rollback: false,
            action_phase_elapsed_ms: None,
            experiment_elapsed_ms: None,
            shutdown_rollback_elapsed_ms: None,
        }
    }

    #[test]
    fn watchdog_is_ok_for_healthy_inputs() {
        let report = evaluate_daemon_watchdog(healthy_inputs(), &DaemonWatchdogConfig::default());

        assert!(report.ok);
        assert!(report.issues.is_empty());
        assert!(report.recommended_actions.is_empty());
    }

    #[test]
    fn watchdog_restarts_monitor_when_ringbuf_is_dead() {
        let mut inputs = healthy_inputs();
        inputs.ringbuf_alive = false;

        let report = evaluate_daemon_watchdog(inputs, &DaemonWatchdogConfig::default());

        assert!(!report.ok);
        assert_eq!(report.issues[0].reason_code, "ringbuf_not_alive");
        assert!(
            report
                .recommended_actions
                .contains(&DaemonSelfHealingAction::RestartMonitorSubsystem)
        );
        assert!(
            report
                .recommended_actions
                .contains(&DaemonSelfHealingAction::PauseAutotune)
        );
    }

    #[test]
    fn watchdog_pauses_when_drop_counters_are_high() {
        let mut inputs = healthy_inputs();
        inputs.drop_counters_total = 10_001;

        let report = evaluate_daemon_watchdog(inputs, &DaemonWatchdogConfig::default());

        assert_eq!(report.issues[0].reason_code, "drop_counters_high");
        assert!(
            report
                .recommended_actions
                .contains(&DaemonSelfHealingAction::EnterObserveOnly)
        );
    }

    #[test]
    fn watchdog_rolls_back_when_health_degrades_during_active_rollback() {
        let mut inputs = healthy_inputs();
        inputs.has_active_rollback = true;
        inputs.health.ok_for_apply = false;
        inputs.health.state = SystemHealthState::Overheated;
        inputs.health.reason_code = Some("cpu_overheated".to_owned());

        let report = evaluate_daemon_watchdog(inputs, &DaemonWatchdogConfig::default());

        assert_eq!(report.issues[0].reason_code, "cpu_overheated");
        assert_eq!(
            report.recommended_actions[0],
            DaemonSelfHealingAction::RollbackActiveExperiment
        );
    }

    #[test]
    fn watchdog_faults_on_stuck_action_phase() {
        let mut inputs = healthy_inputs();
        inputs.has_active_rollback = true;
        inputs.action_phase_elapsed_ms = Some(30_001);

        let report = evaluate_daemon_watchdog(inputs, &DaemonWatchdogConfig::default());

        assert!(
            report
                .recommended_actions
                .contains(&DaemonSelfHealingAction::FaultHard)
        );
        assert!(
            report
                .recommended_actions
                .contains(&DaemonSelfHealingAction::RollbackActiveExperiment)
        );
    }

    #[test]
    fn watchdog_inputs_from_state_carries_health_and_rollback_context() {
        let health = SystemHealthSnapshot::default();
        let state = DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            phase: DaemonPhase::Apply,
            active_experiment: Some(DaemonExperimentState {
                experiment_id: "experiment-1".to_owned(),
                action_id: "action-1".to_owned(),
                candidate_name: Some("candidate".to_owned()),
                mode: DaemonMode::ApplyLowRisk,
                safety_class: crate::actions::SafetyClass::ReversibleLowRisk,
                started_unix_nanos: None,
            }),
            active_rollback: Some(DaemonRollbackState {
                action_id: "action-1".to_owned(),
                mode: DaemonMode::ApplyLowRisk,
                safety_class: crate::actions::SafetyClass::ReversibleLowRisk,
                rollback_available: true,
                token: None,
                manual_restore_command: Some("stutter daemon restore".to_owned()),
            }),
            ..DaemonState::default()
        };

        let inputs = DaemonWatchdogInputs::from_state_and_health(&state, &health);

        assert_eq!(inputs.phase, DaemonPhase::Apply);
        assert!(inputs.has_active_experiment);
        assert!(inputs.has_active_rollback);
        assert!(inputs.rollback_journal_clean);
    }
}
