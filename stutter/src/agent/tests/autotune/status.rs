use super::*;

#[tokio::test]
async fn autotune_status_includes_serializable_daemon_state_without_task_handles() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    *state.active_autotune.lock().await = Some(test_autotune_handle());
    *state.daemon_state.lock().await = DaemonState {
        mode: DaemonMode::ApplyLowRisk,
        phase: DaemonPhase::Apply,
        active_target: Some(DaemonTargetState {
            root_pid: Some(1234),
            active_targets: 1,
            comm: Some("Game.exe".to_owned()),
        }),
        active_experiment: Some(DaemonExperimentState {
            experiment_id: crate::autotune::experiment::ExperimentId::new("experiment-1"),
            action_id: crate::actions::ActionId::new("remote-autotune-start:apply-low-risk"),
            candidate_name: Some("Game.exe".to_owned()),
            mode: DaemonMode::ApplyLowRisk,
            safety_class: SafetyClass::ReversibleLowRisk,
            started_unix_nanos: Some(100),
        }),
        active_rollback: Some(DaemonRollbackState {
            action_id: crate::actions::ActionId::new("remote-autotune-start:apply-low-risk"),
            mode: DaemonMode::ApplyLowRisk,
            safety_class: SafetyClass::ReversibleLowRisk,
            rollback_available: false,
            token: None,
            manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
        }),
        ..DaemonState::default()
    };

    let response = AutotuneStatusResponse {
        active: true,
        mode: Some("apply-low-risk".to_owned()),
        watch_process: Some("Game.exe".to_owned()),
        tree_pid: Some(1234),
        started_unix_nanos: Some(100),
        focus_group: None,
        target_root: Some(1234),
        current_score: None,
        active_profile: None,
        last_decision: None,
        rollback_available: false,
        cooldown_remaining_seconds: None,
        data_quality: None,
        last_fault: None,
        manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
        daemon_state: state.daemon_state.lock().await.clone(),
        message: "test".to_owned(),
    };

    let json = serde_json::to_string(&response).unwrap();

    assert!(json.contains("\"daemon_state\""));
    assert!(json.contains("\"active_experiment\""));
    assert!(json.contains("\"active_rollback\""));
    assert!(!json.contains("stop_tx"));
    assert!(!json.contains("join"));
}
