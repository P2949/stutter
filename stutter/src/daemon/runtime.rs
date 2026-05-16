use std::path::PathBuf;

use crate::{
    autotune::runtime::{AutotuneDecisionStreamEntry, AutotuneRuntimeConfig},
    config::model::MonitorConfig,
    daemon::{
        DAEMON_STATE_SCHEMA_VERSION, DaemonConfig, DaemonDecisionState, DaemonDegradedStatus,
        DaemonFaultState, DaemonLifecycleAction, DaemonLifecycleEvent, DaemonLifecycleInputs,
        DaemonLifecyclePolicy, DaemonLifecycleTransition, DaemonPhase, DaemonState,
        DaemonTargetState, SystemHealthInputs, SystemHealthThresholds, daemon_decision_state,
        evaluate_daemon_lifecycle_event, evaluate_system_health,
    },
    session_events::MonitorEvent,
};

#[derive(Clone, Debug)]
pub struct DaemonRuntimeConfig {
    pub daemon_config: DaemonConfig,
    pub monitor_config: MonitorConfig,
    pub autotune_config: Option<AutotuneRuntimeConfig>,
    pub state_snapshot_path: PathBuf,
    pub rollback_on_stop: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DaemonHealthModel {
    transition_count: usize,
    last_transition_unix_nanos: Option<u128>,
}

#[derive(Debug)]
pub struct DaemonRuntime {
    #[allow(dead_code)]
    config: DaemonRuntimeConfig,
    phase: DaemonPhase,
    state: DaemonState,
    health: DaemonHealthModel,
}

#[derive(Debug)]
pub enum DaemonRuntimeEvent {
    Initialized,
    RecoveryCompleted,
    MonitorEvent(MonitorEvent),
    Decision(Box<AutotuneDecisionStreamEntry>),
    Lifecycle(DaemonLifecycleEvent),
    ShutdownRequested,
    Fault(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DaemonTransition {
    pub from: DaemonPhase,
    pub to: DaemonPhase,
    pub reason: String,
    pub unix_nanos: u128,
}

impl DaemonRuntime {
    pub fn new(config: DaemonRuntimeConfig) -> Self {
        let phase = DaemonPhase::Init;
        let state = DaemonState {
            schema_version: DAEMON_STATE_SCHEMA_VERSION,
            mode: config.daemon_config.mode,
            phase,
            ..DaemonState::default()
        };

        Self {
            config,
            phase,
            state,
            health: DaemonHealthModel::default(),
        }
    }

    pub fn phase(&self) -> DaemonPhase {
        self.phase
    }

    pub fn state(&self) -> &DaemonState {
        &self.state
    }

    pub fn transition_to(
        &mut self,
        next: DaemonPhase,
        reason: impl Into<String>,
    ) -> DaemonTransition {
        let from = self.phase;
        let reason = reason.into();
        let unix_nanos = crate::audit::unix_nanos_now();

        self.phase = next;
        self.state.phase = next;
        self.state.last_decision = Some(DaemonDecisionState {
            decision: "daemon_transition".to_owned(),
            reason: reason.clone(),
            unix_nanos: Some(unix_nanos),
            score_total: None,
            candidate_count: None,
            top_denied_reason: None,
            planner: None,
            situation: None,
            focus_kind: None,
        });
        self.state.faulted = if next == DaemonPhase::Faulted {
            Some(DaemonFaultState {
                reason: reason.clone(),
                manual_restore_command: Some("stutter autotune restore".to_owned()),
            })
        } else {
            None
        };
        self.health.transition_count = self.health.transition_count.saturating_add(1);
        self.health.last_transition_unix_nanos = Some(unix_nanos);

        log::info!(
            "daemon_transition from_phase={} to_phase={} reason={} unix_nanos={}",
            from.lifecycle_label(),
            next.lifecycle_label(),
            reason,
            unix_nanos
        );

        DaemonTransition {
            from,
            to: next,
            reason,
            unix_nanos,
        }
    }

    pub fn handle_lifecycle_event(
        &mut self,
        event: DaemonLifecycleEvent,
    ) -> DaemonLifecycleTransition {
        let transition = evaluate_daemon_lifecycle_event(
            DaemonLifecycleInputs {
                has_active_experiment: self.state.active_experiment.is_some(),
                active_experiment_has_rollback: self
                    .state
                    .active_rollback
                    .as_ref()
                    .is_some_and(|rollback| rollback.rollback_available),
                event,
            },
            &DaemonLifecyclePolicy::default(),
        );
        self.apply_lifecycle_transition(&transition);
        transition
    }

    fn apply_lifecycle_transition(&mut self, transition: &DaemonLifecycleTransition) {
        let next_phase = lifecycle_phase_for_actions(&transition.actions, self.phase);
        let unix_nanos = crate::audit::unix_nanos_now();

        self.phase = next_phase;
        self.state.phase = next_phase;
        self.state.last_decision = Some(DaemonDecisionState {
            decision: "daemon_lifecycle".to_owned(),
            reason: transition.reason.clone(),
            unix_nanos: Some(unix_nanos),
            score_total: None,
            candidate_count: None,
            top_denied_reason: None,
            planner: None,
            situation: None,
            focus_kind: None,
        });
        replace_degraded_status(
            &mut self.state.degraded,
            DaemonDegradedStatus {
                category: "lifecycle".to_owned(),
                message: format!(
                    "{}; actions={}",
                    transition.reason,
                    lifecycle_actions_label(&transition.actions)
                ),
            },
        );

        if transition.health_suspended_or_resumed {
            self.state.health = evaluate_system_health(
                SystemHealthInputs {
                    suspended_or_resumed: true,
                    ..SystemHealthInputs::default()
                },
                &SystemHealthThresholds::default(),
            );
        }

        if transition
            .actions
            .contains(&DaemonLifecycleAction::EnterObserveOnly)
        {
            self.state.mode = crate::daemon::DaemonMode::Observe;
            self.state.active_experiment = None;
            self.state.active_rollback = None;
        }

        self.health.transition_count = self.health.transition_count.saturating_add(1);
        self.health.last_transition_unix_nanos = Some(unix_nanos);

        log::info!(
            "daemon_lifecycle event={} phase={} actions={} reason={} unix_nanos={}",
            transition.event.as_str(),
            next_phase.lifecycle_label(),
            lifecycle_actions_label(&transition.actions),
            transition.reason,
            unix_nanos
        );
    }

    pub fn handle_event(&mut self, event: DaemonRuntimeEvent) -> Option<DaemonTransition> {
        match event {
            DaemonRuntimeEvent::Initialized => {
                Some(self.transition_to(DaemonPhase::Recover, "daemon initialized"))
            }
            DaemonRuntimeEvent::RecoveryCompleted => Some(self.transition_to(
                recovered_phase_for_mode(self.state.mode),
                "startup recovery complete",
            )),
            DaemonRuntimeEvent::MonitorEvent(event) => self.handle_monitor_event(event),
            DaemonRuntimeEvent::Decision(decision) => self.handle_decision_event(*decision),
            DaemonRuntimeEvent::Lifecycle(event) => {
                let from = self.phase;
                let transition = self.handle_lifecycle_event(event);
                (self.phase != from).then(|| DaemonTransition {
                    from,
                    to: self.phase,
                    reason: transition.reason,
                    unix_nanos: crate::audit::unix_nanos_now(),
                })
            }
            DaemonRuntimeEvent::ShutdownRequested => {
                Some(self.transition_to(DaemonPhase::Shutdown, "shutdown requested"))
            }
            DaemonRuntimeEvent::Fault(reason) => {
                Some(self.transition_to(DaemonPhase::Faulted, reason))
            }
        }
    }

    fn handle_monitor_event(&mut self, event: MonitorEvent) -> Option<DaemonTransition> {
        match event {
            MonitorEvent::TargetSnapshot {
                active_targets,
                removed_targets,
                ..
            } => {
                let had_active_experiment = self.state.active_experiment.is_some();
                self.state.active_target = daemon_target_from_snapshot(&active_targets);
                if self.state.active_target.is_none() && had_active_experiment {
                    return Some(self.transition_to(
                        DaemonPhase::Rollback,
                        format!(
                            "target snapshot lost active workload; removed_targets={}",
                            removed_targets.len()
                        ),
                    ));
                }
                None
            }
            MonitorEvent::FocusChanged {
                root_pids,
                member_pids,
                new_kind,
                confidence,
                ..
            } => {
                self.state.active_target = Some(DaemonTargetState {
                    root_pid: root_pids.first().copied(),
                    active_targets: member_pids.len().max(root_pids.len()),
                    comm: Some(format!("{new_kind:?}")),
                });

                if self.state.active_experiment.is_some() {
                    return Some(self.transition_to(
                        DaemonPhase::Rollback,
                        format!(
                            "focus changed during active experiment; new_kind={new_kind:?} confidence={confidence:.2}"
                        ),
                    ));
                }
                None
            }
            MonitorEvent::FocusCleared { reason, .. } => {
                self.state.active_target = None;
                if self.state.active_experiment.is_some() {
                    return Some(self.transition_to(
                        DaemonPhase::Rollback,
                        format!("focus cleared during active experiment: {reason}"),
                    ));
                }
                self.state.last_decision = Some(daemon_decision_state("focus_cleared", reason));
                None
            }
            MonitorEvent::DataQualityWarning { message } => {
                replace_degraded_status(
                    &mut self.state.degraded,
                    DaemonDegradedStatus {
                        category: "data_quality".to_owned(),
                        message: message.clone(),
                    },
                );
                self.state.last_decision =
                    Some(daemon_decision_state("data_quality_warning", message));
                None
            }
            MonitorEvent::Finished { reason } => {
                Some(self.transition_to(DaemonPhase::Shutdown, reason))
            }
            _ => None,
        }
    }

    fn handle_decision_event(
        &mut self,
        decision: AutotuneDecisionStreamEntry,
    ) -> Option<DaemonTransition> {
        let from = self.phase;
        let next_phase = daemon_phase_from_decision_label(&decision.phase).unwrap_or(self.phase);
        let unix_nanos = decision.unix_nanos;

        self.phase = next_phase;
        self.state.phase = next_phase;
        if let Some(mode) = daemon_mode_from_decision_label(&decision.mode) {
            self.state.mode = mode;
        }
        self.state.active_target = daemon_target_from_decision(&decision);
        self.state.last_decision = Some(DaemonDecisionState {
            decision: decision.decision.clone(),
            reason: decision.reason.clone(),
            unix_nanos: Some(unix_nanos),
            score_total: Some(decision.score_total),
            candidate_count: Some(decision.candidate_count),
            top_denied_reason: decision.top_denied_reason.clone(),
            planner: decision.planner.clone(),
            situation: Some(decision.situation.clone()),
            focus_kind: decision.focus_kind.clone(),
        });

        if next_phase == DaemonPhase::Faulted {
            self.state.faulted = Some(DaemonFaultState {
                reason: decision.reason.clone(),
                manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
            });
        } else {
            self.state.faulted = None;
        }

        if next_phase != from {
            self.health.transition_count = self.health.transition_count.saturating_add(1);
            self.health.last_transition_unix_nanos = Some(unix_nanos);
            return Some(DaemonTransition {
                from,
                to: next_phase,
                reason: format!(
                    "autotune decision={} reason={}",
                    decision.decision, decision.reason
                ),
                unix_nanos,
            });
        }

        None
    }
}

fn recovered_phase_for_mode(mode: crate::daemon::DaemonMode) -> DaemonPhase {
    match mode {
        crate::daemon::DaemonMode::Observe => DaemonPhase::Observe,
        crate::daemon::DaemonMode::Suggest => DaemonPhase::Decide,
        crate::daemon::DaemonMode::ApplyLowRisk
        | crate::daemon::DaemonMode::ApplyMediumRisk
        | crate::daemon::DaemonMode::ApplyHighRisk => DaemonPhase::Observe,
    }
}

fn daemon_target_from_snapshot(
    active_targets: &std::collections::BTreeMap<u32, crate::process_tree::TaskInfo>,
) -> Option<DaemonTargetState> {
    let root_pid = active_targets.values().map(|task| task.process_pid).min();
    let comm = active_targets
        .values()
        .next()
        .map(|task| task.process_comm.to_string());

    root_pid.map(|root_pid| DaemonTargetState {
        root_pid: Some(root_pid),
        active_targets: active_targets.len(),
        comm,
    })
}

fn daemon_target_from_decision(
    decision: &AutotuneDecisionStreamEntry,
) -> Option<DaemonTargetState> {
    if decision.target_root_pid.is_none() && decision.active_target_count == 0 {
        return None;
    }

    Some(DaemonTargetState {
        root_pid: decision.target_root_pid,
        active_targets: decision.active_target_count,
        comm: decision.focus_kind.clone(),
    })
}

fn daemon_phase_from_decision_label(value: &str) -> Option<DaemonPhase> {
    match value {
        "Disabled" | "disabled" => Some(DaemonPhase::Disabled),
        "Observing" | "observing" | "observe" => Some(DaemonPhase::Observe),
        "Planning" | "planning" | "decide" => Some(DaemonPhase::Decide),
        "Applying" | "applying" | "apply" => Some(DaemonPhase::Apply),
        "Measuring" | "measuring" | "measure" => Some(DaemonPhase::Measure),
        "Keeping" | "keeping" | "keep" => Some(DaemonPhase::Keep),
        "Reverting" | "reverting" | "rollback" => Some(DaemonPhase::Rollback),
        "Cooldown" | "cooldown" => Some(DaemonPhase::Cooldown),
        "Faulted" | "faulted" => Some(DaemonPhase::Faulted),
        _ => None,
    }
}

fn daemon_mode_from_decision_label(value: &str) -> Option<crate::daemon::DaemonMode> {
    match value {
        "Observe" | "observe" => Some(crate::daemon::DaemonMode::Observe),
        "Suggest" | "suggest" => Some(crate::daemon::DaemonMode::Suggest),
        "ApplyLowRisk" | "apply-low-risk" => Some(crate::daemon::DaemonMode::ApplyLowRisk),
        "ApplyMediumRisk" | "apply-medium-risk" => Some(crate::daemon::DaemonMode::ApplyMediumRisk),
        "ApplyHighRisk" | "apply-high-risk" => Some(crate::daemon::DaemonMode::ApplyHighRisk),
        _ => None,
    }
}

fn lifecycle_phase_for_actions(
    actions: &[DaemonLifecycleAction],
    current: DaemonPhase,
) -> DaemonPhase {
    if actions.contains(&DaemonLifecycleAction::RollbackActiveExperiment) {
        DaemonPhase::Rollback
    } else if actions.contains(&DaemonLifecycleAction::EnterObserveOnly) {
        DaemonPhase::Observe
    } else if actions.contains(&DaemonLifecycleAction::PauseExperiments)
        || actions.contains(&DaemonLifecycleAction::WaitStabilization)
    {
        DaemonPhase::Paused
    } else {
        current
    }
}

fn lifecycle_actions_label(actions: &[DaemonLifecycleAction]) -> String {
    if actions.is_empty() {
        return "none".to_owned();
    }

    actions
        .iter()
        .map(|action| action.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

fn replace_degraded_status(statuses: &mut Vec<DaemonDegradedStatus>, status: DaemonDegradedStatus) {
    statuses.retain(|existing| existing.category != status.category);
    statuses.push(status);
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Arc};

    use super::*;
    use crate::{
        actions::SafetyClass,
        autotune::runtime::AutotuneDecisionStreamEntry,
        daemon::{
            ActionSource, DaemonExperimentState, DaemonMode, DaemonRollbackState, SystemHealthState,
        },
        process_tree::{TaskClass, TaskInfo},
    };

    fn runtime_config() -> DaemonRuntimeConfig {
        DaemonRuntimeConfig {
            daemon_config: DaemonConfig {
                mode: DaemonMode::ApplyLowRisk,
                source: ActionSource::Cli,
                ..DaemonConfig::default()
            },
            monitor_config: MonitorConfig::default(),
            autotune_config: None,
            state_snapshot_path: PathBuf::from("/tmp/stutter-daemon-runtime-state.json"),
            rollback_on_stop: true,
        }
    }

    #[test]
    fn daemon_runtime_new_starts_in_init_phase_without_active_state() {
        let runtime = DaemonRuntime::new(runtime_config());

        assert_eq!(runtime.phase(), DaemonPhase::Init);
        assert_eq!(runtime.state().phase, DaemonPhase::Init);
        assert_eq!(runtime.state().mode, DaemonMode::ApplyLowRisk);
        assert_eq!(runtime.state().schema_version, DAEMON_STATE_SCHEMA_VERSION);
        assert!(runtime.state().last_decision.is_none());
        assert!(runtime.state().faulted.is_none());
        assert!(runtime.state().active_target.is_none());
        assert!(runtime.state().active_experiment.is_none());
        assert!(runtime.state().active_rollback.is_none());
        assert!(runtime.config.rollback_on_stop);
        assert_eq!(runtime.config.monitor_config, MonitorConfig::default());
        assert!(runtime.config.autotune_config.is_none());
    }

    #[test]
    fn daemon_runtime_transition_updates_phase_state_and_last_decision() {
        let mut runtime = DaemonRuntime::new(runtime_config());

        let transition = runtime.transition_to(DaemonPhase::Recover, "startup recovery begins");

        assert_eq!(transition.from, DaemonPhase::Init);
        assert_eq!(transition.to, DaemonPhase::Recover);
        assert_eq!(transition.reason, "startup recovery begins");
        assert_eq!(runtime.phase(), DaemonPhase::Recover);
        assert_eq!(runtime.state().phase, DaemonPhase::Recover);

        let last_decision = runtime.state().last_decision.as_ref().unwrap();
        assert_eq!(last_decision.decision, "daemon_transition");
        assert_eq!(last_decision.reason, "startup recovery begins");
        assert_eq!(last_decision.unix_nanos, Some(transition.unix_nanos));
        assert_eq!(last_decision.score_total, None);
        assert!(runtime.state().faulted.is_none());
    }

    #[test]
    fn daemon_runtime_transitions_return_explicit_order() {
        let mut runtime = DaemonRuntime::new(runtime_config());

        let first = runtime.transition_to(DaemonPhase::Recover, "recover controller journal");
        let second = runtime.transition_to(DaemonPhase::Observe, "recovery complete");

        assert_eq!(first.from, DaemonPhase::Init);
        assert_eq!(first.to, DaemonPhase::Recover);
        assert_eq!(second.from, DaemonPhase::Recover);
        assert_eq!(second.to, DaemonPhase::Observe);
        assert_eq!(runtime.phase(), DaemonPhase::Observe);
        assert_eq!(runtime.state().phase, DaemonPhase::Observe);
        assert_eq!(runtime.health.transition_count, 2);
        assert_eq!(
            runtime.health.last_transition_unix_nanos,
            Some(second.unix_nanos)
        );
    }

    #[test]
    fn daemon_runtime_fault_state_is_set_only_when_transitioning_to_faulted() {
        let mut runtime = DaemonRuntime::new(runtime_config());

        runtime.transition_to(DaemonPhase::Recover, "recover controller journal");
        assert!(runtime.state().faulted.is_none());

        runtime.transition_to(DaemonPhase::Faulted, "startup recovery failed");

        let faulted = runtime.state().faulted.as_ref().unwrap();
        assert_eq!(runtime.phase(), DaemonPhase::Faulted);
        assert_eq!(runtime.state().phase, DaemonPhase::Faulted);
        assert_eq!(faulted.reason, "startup recovery failed");
        assert_eq!(
            faulted.manual_restore_command.as_deref(),
            Some("stutter autotune restore")
        );

        runtime.transition_to(DaemonPhase::Shutdown, "operator requested shutdown");

        assert_eq!(runtime.phase(), DaemonPhase::Shutdown);
        assert_eq!(runtime.state().phase, DaemonPhase::Shutdown);
        assert!(runtime.state().faulted.is_none());
    }

    #[test]
    fn daemon_runtime_lifecycle_suspend_schedules_rollback_for_active_experiment() {
        let mut runtime = DaemonRuntime::new(runtime_config());
        runtime.transition_to(DaemonPhase::Measure, "candidate measurement running");
        runtime.state.active_experiment = Some(active_experiment());
        runtime.state.active_rollback = Some(active_rollback(true));

        let transition = runtime.handle_lifecycle_event(DaemonLifecycleEvent::SuspendDetected);

        assert_eq!(runtime.phase(), DaemonPhase::Rollback);
        assert_eq!(runtime.state().phase, DaemonPhase::Rollback);
        assert!(
            transition
                .actions
                .contains(&DaemonLifecycleAction::RollbackActiveExperiment)
        );
        assert_eq!(
            runtime
                .state()
                .last_decision
                .as_ref()
                .map(|decision| decision.decision.as_str()),
            Some("daemon_lifecycle")
        );
        assert_eq!(
            runtime.state().health.reason_code.as_deref(),
            Some("suspend_resume_stabilizing")
        );
    }

    #[test]
    fn daemon_runtime_lifecycle_resume_pauses_and_marks_health_stabilizing() {
        let mut runtime = DaemonRuntime::new(runtime_config());
        runtime.transition_to(DaemonPhase::Measure, "candidate measurement running");

        let transition = runtime
            .handle_lifecycle_event(DaemonLifecycleEvent::ResumeDetected { slept_ms: 8_000 });

        assert_eq!(runtime.phase(), DaemonPhase::Paused);
        assert_eq!(runtime.state().phase, DaemonPhase::Paused);
        assert_eq!(
            runtime.state().health.state,
            SystemHealthState::SuspendedResumed
        );
        assert!(!runtime.state().health.ok_for_apply);
        assert_eq!(transition.stabilization_ms, Some(10_000));
        assert!(
            runtime
                .state()
                .degraded
                .iter()
                .any(|status| status.category == "lifecycle"
                    && status.message.contains("wait_stabilization"))
        );
    }

    #[test]
    fn daemon_runtime_lifecycle_target_exit_without_rollback_enters_observe_only() {
        let mut runtime = DaemonRuntime::new(runtime_config());
        runtime.transition_to(DaemonPhase::Measure, "candidate measurement running");
        runtime.state.active_experiment = Some(active_experiment());
        runtime.state.active_rollback = None;

        let transition =
            runtime.handle_lifecycle_event(DaemonLifecycleEvent::TargetExited { pid: 1234 });

        assert_eq!(runtime.phase(), DaemonPhase::Observe);
        assert_eq!(runtime.state().phase, DaemonPhase::Observe);
        assert_eq!(runtime.state().mode, DaemonMode::Observe);
        assert!(runtime.state().active_experiment.is_none());
        assert!(runtime.state().active_rollback.is_none());
        assert!(
            transition
                .actions
                .contains(&DaemonLifecycleAction::EnterObserveOnly)
        );
    }

    #[test]
    fn daemon_runtime_handle_event_drives_startup_transitions() {
        let mut runtime = DaemonRuntime::new(runtime_config());

        let init = runtime
            .handle_event(DaemonRuntimeEvent::Initialized)
            .unwrap();
        assert_eq!(init.from, DaemonPhase::Init);
        assert_eq!(init.to, DaemonPhase::Recover);

        let recovered = runtime
            .handle_event(DaemonRuntimeEvent::RecoveryCompleted)
            .unwrap();
        assert_eq!(recovered.from, DaemonPhase::Recover);
        assert_eq!(recovered.to, DaemonPhase::Observe);
        assert_eq!(runtime.state().phase, DaemonPhase::Observe);
    }

    #[test]
    fn daemon_runtime_monitor_target_snapshot_updates_status_cache() {
        let mut runtime = DaemonRuntime::new(runtime_config());
        runtime.handle_event(DaemonRuntimeEvent::RecoveryCompleted);
        let mut targets = BTreeMap::new();
        targets.insert(11, task_info(11, 10, "game-main"));
        targets.insert(12, task_info(12, 10, "game-render"));

        let transition = runtime.handle_event(DaemonRuntimeEvent::MonitorEvent(
            MonitorEvent::TargetSnapshot {
                elapsed_ms: 100,
                active_targets: targets,
                removed_targets: Vec::new(),
            },
        ));

        assert!(transition.is_none());
        let target = runtime.state().active_target.as_ref().unwrap();
        assert_eq!(target.root_pid, Some(10));
        assert_eq!(target.active_targets, 2);
        assert_eq!(target.comm.as_deref(), Some("game-main"));
    }

    #[test]
    fn daemon_runtime_empty_target_snapshot_rolls_back_active_experiment() {
        let mut runtime = DaemonRuntime::new(runtime_config());
        runtime.transition_to(DaemonPhase::Measure, "candidate measurement running");
        runtime.state.active_experiment = Some(active_experiment());
        runtime.state.active_rollback = Some(active_rollback(true));

        let transition = runtime
            .handle_event(DaemonRuntimeEvent::MonitorEvent(
                MonitorEvent::TargetSnapshot {
                    elapsed_ms: 200,
                    active_targets: BTreeMap::new(),
                    removed_targets: vec![1234],
                },
            ))
            .unwrap();

        assert_eq!(transition.from, DaemonPhase::Measure);
        assert_eq!(transition.to, DaemonPhase::Rollback);
        assert_eq!(runtime.state().phase, DaemonPhase::Rollback);
        assert!(runtime.state().active_target.is_none());
    }

    #[test]
    fn daemon_runtime_decision_event_updates_phase_target_and_last_decision() {
        let mut runtime = DaemonRuntime::new(runtime_config());

        let transition = runtime
            .handle_event(DaemonRuntimeEvent::Decision(Box::new(
                AutotuneDecisionStreamEntry {
                    unix_nanos: 42,
                    phase: "Measuring".to_owned(),
                    mode: "ApplyLowRisk".to_owned(),
                    focus_kind: Some("Game".to_owned()),
                    focus_confidence: 0.9,
                    target_root_pid: Some(1234),
                    active_target_count: 3,
                    situation: "GameCpuSchedulerPressure".to_owned(),
                    situation_confidence: 0.9,
                    situation_evidence: Vec::new(),
                    situation_blockers: Vec::new(),
                    protected_tasks_count: 0,
                    candidate_count: 0,
                    top_denied_reason: None,
                    planner: None,
                    score_total: 900,
                    data_quality: "High".to_owned(),
                    data_quality_reason_codes: Vec::new(),
                    decision: "start_experiment".to_owned(),
                    reason: "candidate evidence is strong".to_owned(),
                },
            )))
            .unwrap();

        assert_eq!(transition.from, DaemonPhase::Init);
        assert_eq!(transition.to, DaemonPhase::Measure);
        assert_eq!(runtime.state().mode, DaemonMode::ApplyLowRisk);
        assert_eq!(
            runtime
                .state()
                .active_target
                .as_ref()
                .map(|target| (target.root_pid, target.active_targets)),
            Some((Some(1234), 3))
        );
        let last_decision = runtime.state().last_decision.as_ref().unwrap();
        assert_eq!(last_decision.decision, "start_experiment");
        assert_eq!(last_decision.score_total, Some(900));
        assert_eq!(last_decision.unix_nanos, Some(42));
    }

    fn active_experiment() -> DaemonExperimentState {
        DaemonExperimentState {
            experiment_id: "experiment-1".to_owned(),
            action_id: "action-1".to_owned(),
            candidate_name: Some("game-candidate".to_owned()),
            mode: DaemonMode::ApplyLowRisk,
            safety_class: SafetyClass::ReversibleLowRisk,
            started_unix_nanos: Some(100),
        }
    }

    fn active_rollback(rollback_available: bool) -> DaemonRollbackState {
        DaemonRollbackState {
            action_id: "action-1".to_owned(),
            mode: DaemonMode::ApplyLowRisk,
            safety_class: SafetyClass::ReversibleLowRisk,
            rollback_available,
            token: None,
            manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
        }
    }

    fn task_info(tid: u32, process_pid: u32, comm: &str) -> TaskInfo {
        TaskInfo {
            tid,
            process_pid,
            process_ppid: 1,
            comm: comm.to_owned(),
            process_comm: Arc::<str>::from(comm),
            process_starttime_ticks: Some(100),
            task_starttime_ticks: Some(100 + u64::from(tid)),
            exe_dev: None,
            exe_ino: None,
            class: TaskClass::Game,
            sched_policy: None,
            from_cgroup: false,
        }
    }
}
