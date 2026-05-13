use crate::{
    autotune::runtime::{AutotuneDecisionStreamEntry, AutotuneRuntime, AutotuneRuntimeConfig},
    daemon::DaemonState,
    session_events::MonitorEvent,
};

pub struct AutotuneSubsystem {
    runtime: AutotuneRuntime,
}

#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub enum AutotuneSubsystemEvent {
    Decision(AutotuneDecisionStreamEntry),
    StateSnapshot(DaemonState),
}

impl AutotuneSubsystem {
    pub fn new(config: AutotuneRuntimeConfig) -> Self {
        Self {
            runtime: AutotuneRuntime::new(config),
        }
    }

    pub fn on_monitor_event(
        &mut self,
        event: MonitorEvent,
    ) -> anyhow::Result<Vec<AutotuneSubsystemEvent>> {
        let decision = self.runtime.on_event(event)?;
        let mut events = Vec::new();

        if let Some(decision) = decision {
            events.push(AutotuneSubsystemEvent::Decision(decision));
        }

        events.push(AutotuneSubsystemEvent::StateSnapshot(
            self.runtime.daemon_state_snapshot(),
        ));

        Ok(events)
    }

    pub fn daemon_state_snapshot(&self) -> DaemonState {
        self.runtime.daemon_state_snapshot()
    }

    pub fn has_active_experiment(&self) -> bool {
        self.runtime.has_active_experiment()
    }

    pub fn rollback_on_stop(&mut self, reason: &str) -> anyhow::Result<Option<DaemonState>> {
        self.runtime.rollback_on_stop(reason)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::daemon::DaemonPhase;

    fn subsystem_config() -> AutotuneRuntimeConfig {
        let mut config = AutotuneRuntimeConfig::observe(None, Some(1234), None);
        config.history_log = None;
        config
    }

    #[test]
    fn autotune_subsystem_starts_without_active_experiment() {
        let subsystem = AutotuneSubsystem::new(subsystem_config());

        assert!(!subsystem.has_active_experiment());
        assert_eq!(
            subsystem.daemon_state_snapshot().phase,
            DaemonPhase::Observe
        );
    }

    #[test]
    fn autotune_subsystem_target_snapshot_emits_state_snapshot_without_decision() {
        let mut subsystem = AutotuneSubsystem::new(subsystem_config());

        let events = subsystem
            .on_monitor_event(MonitorEvent::TargetSnapshot {
                elapsed_ms: 1,
                active_targets: BTreeMap::new(),
                removed_targets: Vec::new(),
            })
            .unwrap();

        assert_eq!(events.len(), 1);
        match &events[0] {
            AutotuneSubsystemEvent::StateSnapshot(snapshot) => {
                assert_eq!(snapshot.phase, DaemonPhase::Observe);
                assert!(snapshot.last_decision.is_none());
            }
            other => panic!("expected state snapshot event, got {other:?}"),
        }
    }

    #[test]
    fn autotune_subsystem_decision_event_is_followed_by_state_snapshot() {
        let mut subsystem = AutotuneSubsystem::new(subsystem_config());

        let events = subsystem
            .on_monitor_event(MonitorEvent::Finished {
                reason: "unit test finished".to_owned(),
            })
            .unwrap();

        assert_eq!(events.len(), 2);
        match &events[0] {
            AutotuneSubsystemEvent::Decision(decision) => {
                assert_eq!(decision.reason, "unit test finished");
            }
            other => panic!("expected decision event, got {other:?}"),
        }

        match &events[1] {
            AutotuneSubsystemEvent::StateSnapshot(snapshot) => {
                assert_eq!(snapshot.phase, DaemonPhase::Observe);
                assert_eq!(
                    snapshot
                        .last_decision
                        .as_ref()
                        .map(|decision| decision.reason.as_str()),
                    Some("unit test finished")
                );
            }
            other => panic!("expected state snapshot event, got {other:?}"),
        }
    }

    #[test]
    fn autotune_subsystem_rollback_on_stop_noops_without_active_experiment() {
        let mut subsystem = AutotuneSubsystem::new(subsystem_config());

        let snapshot = subsystem.rollback_on_stop("daemon stop").unwrap();

        assert!(snapshot.is_none());
        assert!(!subsystem.has_active_experiment());
    }
}
