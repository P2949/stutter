use super::*;

#[tokio::test]
async fn autotune_policy_rejection_updates_faulted_daemon_state() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));

    let response = autotune_start_handler(
        State(state.clone()),
        HeaderMap::new(),
        Json(AutotuneStartRequest {
            mode: "apply-low-risk".to_owned(),
            watch_process: Some("Game.exe".to_owned()),
            tree_pid: None,
            profiles: None,
            config: None,
            duration_seconds: Some(30),
            decision_log: None,
            summary_ms: None,
            preset: None,
            hwmon: false,
            mangohud_log: None,
            auto_focus: false,
            focus_source: None,
            foreground_window: false,
            foreground_source: None,
            foreground_poll_ms: None,
            foreground_max_stale_ms: None,
            washout_seconds: None,
            washout_verify_interval_ms: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let daemon_state = state.daemon_state.lock().await.clone();
    assert_eq!(daemon_state.mode, DaemonMode::ApplyLowRisk);
    assert_eq!(daemon_state.phase, DaemonPhase::Faulted);
    assert!(
        daemon_state
            .faulted
            .as_ref()
            .map(|fault| fault.reason.contains("bearer token"))
            .unwrap_or(false)
    );
}

#[tokio::test]
async fn autotune_policy_rejection_preserves_active_rollback_state() {
    let state = Arc::new(test_agent_state(
        "127.0.0.1:0".parse().unwrap(),
        Some("secret"),
    ));
    {
        let mut daemon_state = state.daemon_state.lock().await;
        daemon_state.mode = DaemonMode::ApplyLowRisk;
        daemon_state.phase = DaemonPhase::Rollback;
        daemon_state.active_rollback = Some(DaemonRollbackState {
            action_id: crate::actions::ActionId::new("cpu-affinity:game"),
            mode: DaemonMode::ApplyLowRisk,
            safety_class: SafetyClass::ReversibleLowRisk,
            rollback_available: true,
            token: None,
            manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
        });
    }
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        axum::http::HeaderValue::from_static("Bearer secret"),
    );

    let response = autotune_start_handler(
        State(state.clone()),
        headers,
        Json(AutotuneStartRequest {
            mode: "apply-low-risk".to_owned(),
            watch_process: Some("Game.exe".to_owned()),
            tree_pid: None,
            profiles: None,
            config: None,
            duration_seconds: Some(30),
            decision_log: None,
            summary_ms: None,
            preset: None,
            hwmon: false,
            mangohud_log: None,
            auto_focus: false,
            focus_source: None,
            foreground_window: false,
            foreground_source: None,
            foreground_poll_ms: None,
            foreground_max_stale_ms: None,
            washout_seconds: None,
            washout_verify_interval_ms: None,
        }),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let daemon_state = state.daemon_state.lock().await.clone();
    assert_eq!(daemon_state.phase, DaemonPhase::Faulted);
    assert_eq!(
        daemon_state
            .active_rollback
            .as_ref()
            .map(|rollback| rollback.action_id.as_str()),
        Some("cpu-affinity:game")
    );
    assert!(
        daemon_state
            .faulted
            .as_ref()
            .and_then(|fault| fault.manual_restore_command.as_deref())
            .is_some_and(|command| command.contains("emergency-restore"))
    );
}
