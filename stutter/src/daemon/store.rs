use super::{
    policy::DaemonMode,
    runtime::DaemonTransition,
    state::{
        DaemonDecisionState, DaemonDegradedStatus, DaemonFaultState, DaemonPhase, DaemonState,
        DaemonStateSnapshotWriter,
    },
};

pub struct DaemonStateStore {
    state: DaemonState,
    writer: Option<DaemonStateSnapshotWriter>,
}

impl DaemonStateStore {
    pub fn new(initial: DaemonState, writer: Option<DaemonStateSnapshotWriter>) -> Self {
        Self {
            state: initial,
            writer,
        }
    }

    pub fn state(&self) -> &DaemonState {
        &self.state
    }

    pub fn replace(&mut self, state: DaemonState) -> anyhow::Result<()> {
        if let Some(writer) = self.writer.as_ref() {
            writer.write(&state)?;
        }

        self.state = state;
        Ok(())
    }

    pub fn transition(&mut self, transition: DaemonTransition) -> anyhow::Result<()> {
        let mut state = self.state.clone();
        state.phase = transition.to;
        state.last_decision = Some(DaemonDecisionState {
            decision: "daemon_transition".to_owned(),
            reason: transition.reason,
            unix_nanos: Some(transition.unix_nanos),
            score_total: None,
            candidate_count: None,
            top_denied_reason: None,
            planner: None,
            situation: None,
            focus_kind: None,
        });

        if !transition.to.is_faulted() {
            state.faulted = None;
        }

        self.replace(state)
    }

    pub fn mark_fault(
        &mut self,
        mode: DaemonMode,
        reason: String,
        manual_restore_command: Option<String>,
    ) -> anyhow::Result<()> {
        let mut state = self.state.clone();
        state.mode = mode;
        state.phase = DaemonPhase::Faulted;
        state.last_decision = Some(DaemonDecisionState {
            decision: "daemon_fault".to_owned(),
            reason: reason.clone(),
            unix_nanos: Some(crate::audit::unix_nanos_now()),
            score_total: None,
            candidate_count: None,
            top_denied_reason: None,
            planner: None,
            situation: None,
            focus_kind: None,
        });
        state.degraded = vec![DaemonDegradedStatus {
            category: "daemon_state_store".to_owned(),
            message: reason.clone(),
        }];
        state.faulted = Some(DaemonFaultState {
            reason,
            manual_restore_command,
        });

        self.replace(state)
    }

    pub fn update_from_autotune_snapshot(&mut self, snapshot: DaemonState) -> anyhow::Result<()> {
        self.replace(snapshot)
    }

    pub fn pause(&mut self, reason: impl Into<String>) -> anyhow::Result<()> {
        let mut state = self.state.clone();
        let reason = reason.into();
        state.phase = DaemonPhase::Paused;
        state.last_decision = Some(DaemonDecisionState {
            decision: "daemon_paused".to_owned(),
            reason,
            unix_nanos: Some(crate::audit::unix_nanos_now()),
            score_total: None,
            candidate_count: None,
            top_denied_reason: None,
            planner: None,
            situation: None,
            focus_kind: None,
        });
        state.faulted = None;

        self.replace(state)
    }

    pub fn resume(&mut self, reason: impl Into<String>) -> anyhow::Result<()> {
        if self.state.faulted.is_some() {
            anyhow::bail!("cannot resume a faulted daemon state; restore first");
        }

        let mut state = self.state.clone();
        let reason = reason.into();
        state.phase = DaemonPhase::Observe;
        state.last_decision = Some(DaemonDecisionState {
            decision: "daemon_resumed".to_owned(),
            reason,
            unix_nanos: Some(crate::audit::unix_nanos_now()),
            score_total: None,
            candidate_count: None,
            top_denied_reason: None,
            planner: None,
            situation: None,
            focus_kind: None,
        });

        self.replace(state)
    }

    pub fn mark_restored(&mut self, reason: impl Into<String>) -> anyhow::Result<()> {
        let mut state = self.state.clone();
        let reason = reason.into();
        state.phase = DaemonPhase::Observe;
        state.active_experiment = None;
        state.active_rollback = None;
        state.cooldown_until_unix_nanos = None;
        state.faulted = None;
        state.degraded.clear();
        state.last_decision = Some(DaemonDecisionState {
            decision: "daemon_restored".to_owned(),
            reason,
            unix_nanos: Some(crate::audit::unix_nanos_now()),
            score_total: None,
            candidate_count: None,
            top_denied_reason: None,
            planner: None,
            situation: None,
            focus_kind: None,
        });

        self.replace(state)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::daemon::{ActionSource, DaemonConfig, load_daemon_state};

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-daemon-state-store-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn replace_updates_memory_and_snapshot_writer() {
        let dir = temp_dir("replace");
        let path = dir.join("daemon_state.json");
        let writer = DaemonStateSnapshotWriter::new(&path);
        let mut store = DaemonStateStore::new(DaemonState::default(), Some(writer));
        let next = DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            phase: DaemonPhase::Observe,
            ..DaemonState::default()
        };

        store.replace(next.clone()).unwrap();

        assert_eq!(store.state(), &next);
        assert_eq!(load_daemon_state(&path).unwrap(), next);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn transition_updates_phase_and_last_decision() {
        let mut store = DaemonStateStore::new(DaemonState::default(), None);

        store
            .transition(DaemonTransition {
                from: DaemonPhase::Disabled,
                to: DaemonPhase::Observe,
                reason: "monitor started".to_owned(),
                unix_nanos: 123,
            })
            .unwrap();

        assert_eq!(store.state().phase, DaemonPhase::Observe);
        assert_eq!(
            store
                .state()
                .last_decision
                .as_ref()
                .map(|decision| decision.decision.as_str()),
            Some("daemon_transition")
        );
        assert_eq!(
            store
                .state()
                .last_decision
                .as_ref()
                .map(|decision| decision.reason.as_str()),
            Some("monitor started")
        );
        assert_eq!(
            store
                .state()
                .last_decision
                .as_ref()
                .and_then(|decision| decision.unix_nanos),
            Some(123)
        );
    }

    #[test]
    fn mark_fault_preserves_active_recovery_context_and_profile_memory() {
        let rollback_token = crate::actions::RollbackToken::CpuAffinityRestoreFile {
            path: std::path::PathBuf::from("/tmp/stutter-restore.json"),
            affected_tasks: 31,
        };
        let mut store = DaemonStateStore::new(
            DaemonState {
                mode: DaemonMode::ApplyLowRisk,
                phase: DaemonPhase::Apply,
                active_target: Some(crate::daemon::DaemonTargetState {
                    root_pid: Some(1234),
                    active_targets: 7,
                    comm: Some("game".to_owned()),
                }),
                active_experiment: Some(crate::daemon::DaemonExperimentState {
                    experiment_id: "experiment-1".to_owned(),
                    action_id: "cpu-affinity-profile:game-main".to_owned(),
                    candidate_name: Some("game-main".to_owned()),
                    mode: DaemonMode::ApplyLowRisk,
                    safety_class: crate::actions::SafetyClass::ReversibleLowRisk,
                    started_unix_nanos: Some(100),
                }),
                active_rollback: Some(crate::daemon::DaemonRollbackState {
                    action_id: "cpu-affinity-profile:game-main".to_owned(),
                    mode: DaemonMode::ApplyLowRisk,
                    safety_class: crate::actions::SafetyClass::ReversibleLowRisk,
                    rollback_available: true,
                    token: Some(rollback_token.clone()),
                    manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
                }),
                profile_memory: crate::daemon::state::DaemonProfileMemory {
                    profiles: vec![crate::daemon::DaemonWorkloadProfile {
                        workload_identity_hash: "workload-abc".to_owned(),
                        workload_label: Some("game".to_owned()),
                        candidate_name: "game-main".to_owned(),
                        action_id: "cpu-affinity-profile:game-main".to_owned(),
                        action_kind: "cpu_affinity_profile".to_owned(),
                        safety_class: crate::actions::SafetyClass::ReversibleLowRisk,
                        kept_unix_nanos: 200,
                        last_validated_unix_nanos: Some(200),
                        baseline_score_total: Some(1000),
                        candidate_score_total: Some(850),
                        score_delta: -150,
                        confidence_milli: 900,
                        environment: crate::daemon::DaemonProfileEnvironment::default(),
                        partition: crate::daemon::state::DaemonProfilePartition::default(),
                    }],
                },
                ..DaemonState::default()
            },
            None,
        );

        store
            .mark_fault(
                DaemonMode::ApplyLowRisk,
                "restore did not complete safely".to_owned(),
                Some("stutter daemon emergency-restore --dry-run".to_owned()),
            )
            .unwrap();

        assert_eq!(store.state().phase, DaemonPhase::Faulted);
        assert_eq!(
            store
                .state()
                .faulted
                .as_ref()
                .map(|fault| fault.reason.as_str()),
            Some("restore did not complete safely")
        );
        assert_eq!(
            store
                .state()
                .active_target
                .as_ref()
                .and_then(|target| target.root_pid),
            Some(1234)
        );
        assert_eq!(
            store
                .state()
                .active_experiment
                .as_ref()
                .map(|experiment| experiment.experiment_id.as_str()),
            Some("experiment-1")
        );
        assert_eq!(
            store
                .state()
                .active_rollback
                .as_ref()
                .and_then(|rollback| rollback.token.as_ref()),
            Some(&rollback_token)
        );
        assert_eq!(store.state().profile_memory.profiles.len(), 1);
        assert_eq!(
            store.state().profile_memory.profiles[0].workload_identity_hash,
            "workload-abc"
        );
    }

    #[test]
    fn mark_fault_marks_state_with_faulted_snapshot() {
        let mut store = DaemonStateStore::new(DaemonState::default(), None);

        store
            .mark_fault(
                DaemonMode::ApplyLowRisk,
                "fatal daemon error".to_owned(),
                Some("stutter autotune restore".to_owned()),
            )
            .unwrap();

        assert_eq!(store.state().mode, DaemonMode::ApplyLowRisk);
        assert_eq!(store.state().phase, DaemonPhase::Faulted);
        assert_eq!(
            store
                .state()
                .faulted
                .as_ref()
                .map(|fault| fault.reason.as_str()),
            Some("fatal daemon error")
        );
        assert_eq!(
            store
                .state()
                .faulted
                .as_ref()
                .and_then(|fault| fault.manual_restore_command.as_deref()),
            Some("stutter autotune restore")
        );
    }

    #[test]
    fn update_from_autotune_snapshot_replaces_state() {
        let mut store = DaemonStateStore::new(DaemonState::default(), None);
        let snapshot = DaemonState {
            mode: DaemonMode::Suggest,
            phase: DaemonPhase::Decide,
            ..DaemonState::default()
        };

        store
            .update_from_autotune_snapshot(snapshot.clone())
            .unwrap();

        assert_eq!(store.state(), &snapshot);
    }

    #[test]
    fn pause_and_resume_update_persisted_state() {
        let dir = temp_dir("pause-resume");
        let path = dir.join("daemon_state.json");
        let writer = DaemonStateSnapshotWriter::new(&path);
        let mut store = DaemonStateStore::new(DaemonState::default(), Some(writer));

        store.pause("operator requested pause").unwrap();
        assert_eq!(store.state().phase, DaemonPhase::Paused);
        assert_eq!(load_daemon_state(&path).unwrap().phase, DaemonPhase::Paused);

        store.resume("operator requested resume").unwrap();
        assert_eq!(store.state().phase, DaemonPhase::Observe);
        assert_eq!(
            load_daemon_state(&path).unwrap().phase,
            DaemonPhase::Observe
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn resume_rejects_faulted_state_until_restore() {
        let mut store = DaemonStateStore::new(
            DaemonState {
                phase: DaemonPhase::Faulted,
                faulted: Some(DaemonFaultState {
                    reason: "restore needed".to_owned(),
                    manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
                }),
                ..DaemonState::default()
            },
            None,
        );

        let err = store.resume("operator requested resume").unwrap_err();

        assert!(err.to_string().contains("restore first"));
        assert_eq!(store.state().phase, DaemonPhase::Faulted);
    }

    #[test]
    fn mark_restored_clears_active_and_fault_state() {
        let mut store = DaemonStateStore::new(
            DaemonState {
                phase: DaemonPhase::Faulted,
                active_rollback: Some(crate::daemon::DaemonRollbackState {
                    action_id: "action-1".to_owned(),
                    mode: DaemonMode::ApplyLowRisk,
                    safety_class: crate::actions::SafetyClass::ReversibleLowRisk,
                    rollback_available: true,
                    token: None,
                    manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
                }),
                faulted: Some(DaemonFaultState {
                    reason: "restore needed".to_owned(),
                    manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
                }),
                degraded: vec![DaemonDegradedStatus {
                    category: "restore".to_owned(),
                    message: "restore needed".to_owned(),
                }],
                ..DaemonState::default()
            },
            None,
        );

        store.mark_restored("restore completed").unwrap();

        assert_eq!(store.state().phase, DaemonPhase::Observe);
        assert!(store.state().active_rollback.is_none());
        assert!(store.state().faulted.is_none());
        assert!(store.state().degraded.is_empty());
    }

    #[test]
    fn store_can_hold_default_daemon_runtime_config_state() {
        let config = DaemonConfig {
            mode: DaemonMode::Suggest,
            source: ActionSource::AutotuneRuntime,
            ..DaemonConfig::default()
        };
        let state = DaemonState {
            mode: config.mode,
            phase: DaemonPhase::Init,
            ..DaemonState::default()
        };

        let store = DaemonStateStore::new(state, None);

        assert_eq!(store.state().mode, DaemonMode::Suggest);
        assert_eq!(store.state().phase, DaemonPhase::Init);
    }
}
