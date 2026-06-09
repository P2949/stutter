use std::path::PathBuf;

use crate::{
    autotune::runtime::{AutotuneDecisionStreamEntry, AutotuneRuntimeConfig},
    config::model::MonitorConfig,
    daemon::{
        DaemonConfig,
        health::{SystemHealthInputs, SystemHealthThresholds, evaluate_system_health},
        lifecycle::{
            DaemonLifecycleAction, DaemonLifecycleEvent, DaemonLifecycleInputs,
            DaemonLifecyclePolicy, DaemonLifecycleTransition, evaluate_daemon_lifecycle_event,
        },
        state::{
            DAEMON_STATE_SCHEMA_VERSION, DaemonDecisionState, DaemonDegradedStatus,
            DaemonFaultState, DaemonPhase, DaemonState, DaemonTargetState,
        },
        state_builders::daemon_decision_state,
    },
    process_tree::TaskMap,
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
    pub(super) transition_count: usize,
    pub(super) last_transition_unix_nanos: Option<u128>,
}

#[derive(Debug)]
pub struct DaemonRuntime {
    pub(super) config: DaemonRuntimeConfig,
    pub(super) phase: DaemonPhase,
    pub(super) state: DaemonState,
    pub(super) health: DaemonHealthModel,
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
            diagnostic_current_raw_score_total: None,
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
            diagnostic_current_raw_score_total: None,
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
            self.state.mode = crate::daemon::policy::DaemonMode::Observe;
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
            DaemonRuntimeEvent::ShutdownRequested => Some(self.transition_to(
                DaemonPhase::Shutdown,
                if self.config.rollback_on_stop {
                    "shutdown requested; rollback_on_stop enabled"
                } else {
                    "shutdown requested; rollback_on_stop disabled"
                },
            )),
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
            diagnostic_current_raw_score_total: Some(decision.diagnostic_raw_score_total),
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

pub(super) fn recovered_phase_for_mode(mode: crate::daemon::policy::DaemonMode) -> DaemonPhase {
    match mode {
        crate::daemon::policy::DaemonMode::Observe => DaemonPhase::Observe,
        crate::daemon::policy::DaemonMode::Suggest => DaemonPhase::Decide,
        crate::daemon::policy::DaemonMode::ApplyLowRisk
        | crate::daemon::policy::DaemonMode::ApplyMediumRisk
        | crate::daemon::policy::DaemonMode::ApplyHighRisk => DaemonPhase::Observe,
    }
}

pub(super) fn daemon_target_from_snapshot(active_targets: &TaskMap) -> Option<DaemonTargetState> {
    let root_pid = active_targets
        .values()
        .map(|task| task.process_id().as_u32())
        .min();
    let comm = active_targets
        .values()
        .next()
        .map(|task| task.process_comm.clone());

    root_pid.map(|root_pid| DaemonTargetState {
        root_pid: Some(root_pid),
        active_targets: active_targets.len(),
        comm,
    })
}

pub(super) fn daemon_target_from_decision(
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

pub(super) fn daemon_phase_from_decision_label(value: &str) -> Option<DaemonPhase> {
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

pub(super) fn daemon_mode_from_decision_label(
    value: &str,
) -> Option<crate::daemon::policy::DaemonMode> {
    match value {
        "Observe" | "observe" => Some(crate::daemon::policy::DaemonMode::Observe),
        "Suggest" | "suggest" => Some(crate::daemon::policy::DaemonMode::Suggest),
        "ApplyLowRisk" | "apply-low-risk" => Some(crate::daemon::policy::DaemonMode::ApplyLowRisk),
        "ApplyMediumRisk" | "apply-medium-risk" => {
            Some(crate::daemon::policy::DaemonMode::ApplyMediumRisk)
        }
        "ApplyHighRisk" | "apply-high-risk" => {
            Some(crate::daemon::policy::DaemonMode::ApplyHighRisk)
        }
        _ => None,
    }
}

pub(super) fn lifecycle_phase_for_actions(
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

pub(super) fn lifecycle_actions_label(actions: &[DaemonLifecycleAction]) -> String {
    if actions.is_empty() {
        return "none".to_owned();
    }

    actions
        .iter()
        .map(|action| action.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

pub(super) fn replace_degraded_status(
    statuses: &mut Vec<DaemonDegradedStatus>,
    status: DaemonDegradedStatus,
) {
    statuses.retain(|existing| existing.category != status.category);
    statuses.push(status);
}
