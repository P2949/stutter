use std::path::PathBuf;

use crate::{
    autotune::runtime::{AutotuneDecisionStreamEntry, AutotuneRuntimeConfig},
    config::model::MonitorConfig,
    daemon::{
        DAEMON_STATE_SCHEMA_VERSION, DaemonConfig, DaemonDecisionState, DaemonFaultState,
        DaemonPhase, DaemonState,
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
    Decision(AutotuneDecisionStreamEntry),
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::{ActionSource, DaemonMode};

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
}
