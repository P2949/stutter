use super::*;

#[tokio::test]
async fn autotune_start_accepts_observe_mode_with_tree_pid() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    let response = autotune_start_handler(
        State(state.clone()),
        HeaderMap::new(),
        Json(AutotuneStartRequest {
            mode: "observe".to_owned(),
            watch_process: None,
            tree_pid: Some(1234),
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

    assert_eq!(response.status(), StatusCode::OK);

    let active = state.active_autotune.lock().await;
    assert!(active.is_some());
    assert_eq!(active.as_ref().unwrap().mode, "observe");
    assert_eq!(active.as_ref().unwrap().tree_pid, Some(1234));
}

#[tokio::test]
async fn autotune_start_accepts_suggest_mode_with_watch_process() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    let response = autotune_start_handler(
        State(state.clone()),
        HeaderMap::new(),
        Json(AutotuneStartRequest {
            mode: "suggest".to_owned(),
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

    assert_eq!(response.status(), StatusCode::OK);

    let active = state.active_autotune.lock().await;
    assert!(active.is_some());
    assert_eq!(active.as_ref().unwrap().mode, "suggest");
    assert_eq!(
        active.as_ref().unwrap().watch_process.as_deref(),
        Some("Game.exe")
    );
}

#[tokio::test]
async fn autotune_start_allows_apply_low_risk_mode_with_valid_auth() {
    let mut state_value = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
    state_value.auth = AgentAuth {
        bearer_token: Some("secret".to_owned()),
        ..AgentAuth::default()
    };
    let state = Arc::new(state_value);
    let response = autotune_start_handler(
        State(state.clone()),
        autotune_headers(Some("secret")),
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

    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let decoded: AutotuneStartResponse = serde_json::from_slice(&body).unwrap();
    let body = String::from_utf8_lossy(&body);

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(decoded.status, "started");
    assert_eq!(decoded.mode, "apply-low-risk");
    assert_eq!(
        decoded.message,
        remote_autotune_start_message(DaemonMode::ApplyLowRisk)
    );
    assert!(state.active_autotune.lock().await.is_some());
}

#[test]
fn autotune_headers_test() {
    let headers = autotune_headers(Some("secret"));
    assert_eq!(
        headers.get(axum::http::header::AUTHORIZATION).unwrap(),
        "Bearer secret"
    );
}

#[test]
fn apply_low_risk_start_behavior_uses_daemon_mode_enum() {
    let mut daemon_config = daemon_config_for_runtime_mode(
        DaemonMode::ApplyLowRisk,
        ActionSource::RemoteAgent,
        Some(1234),
        None,
    );
    daemon_config.remote.allow_remote_apply = true;
    let policy = build_daemon_policy(DaemonPolicyBuildInput {
        config: &daemon_config,
        remote_context: None,
    });
    let runtime_config = AutotuneRuntimeConfig::from_daemon_parts(daemon_config, policy, None);

    let runtime_config = apply_remote_autotune_runtime_mode_overrides(
        runtime_config,
        DaemonMode::ApplyLowRisk,
        Some(42),
    );

    assert_eq!(
        remote_autotune_controller_duration(DaemonMode::ApplyLowRisk, Some(42)),
        None
    );
    assert_eq!(runtime_config.candidate_window_seconds, 42);
    assert_eq!(
        runtime_config
            .daemon_config
            .autotune
            .candidate_window_seconds,
        42
    );
    assert_eq!(
        remote_autotune_start_message(DaemonMode::ApplyLowRisk),
        "remote autotune apply-low-risk controller started; apply modes are enabled"
    );
    assert_eq!(
        remote_autotune_controller_duration(DaemonMode::Suggest, Some(7)),
        Some(Duration::from_secs(7))
    );
}

#[tokio::test]
async fn autotune_start_accepts_system_wide_suggestions_by_default() {
    let mut state_value = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
    state_value.autotune_limits.allow_system_wide_suggestions = true;
    state_value.autotune_limits.allow_system_wide_apply = false;
    let state = Arc::new(state_value);

    let response = autotune_start_handler(
        State(state.clone()),
        HeaderMap::new(),
        Json(autotune_request("observe")),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn autotune_start_rejects_system_wide_apply_by_default() {
    let mut state_value = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
    state_value.autotune_limits.allow_system_wide_suggestions = false;
    state_value.autotune_limits.allow_system_wide_apply = true;
    let state = Arc::new(state_value);

    let response = autotune_start_handler(
        State(state.clone()),
        HeaderMap::new(),
        Json(autotune_request("observe")),
    )
    .await
    .into_response();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn autotune_start_updates_daemon_state_with_target_experiment_and_rollback_availability() {
    let mut state_value = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
    state_value.auth = AgentAuth {
        bearer_token: Some("secret".to_owned()),
        ..AgentAuth::default()
    };
    let state = Arc::new(state_value);

    let response = autotune_start_handler(
        State(state.clone()),
        autotune_headers(Some("secret")),
        Json(AutotuneStartRequest {
            mode: "apply-low-risk".to_owned(),
            watch_process: None,
            tree_pid: Some(1234),
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

    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = String::from_utf8_lossy(&body);

    assert_eq!(status, StatusCode::OK, "body={body}");

    let daemon_state = state.daemon_state.lock().await.clone();
    assert_eq!(daemon_state.mode, DaemonMode::ApplyLowRisk);
    assert_eq!(daemon_state.phase, DaemonPhase::Apply);
    assert_eq!(
        daemon_state
            .active_target
            .as_ref()
            .and_then(|target| target.root_pid),
        Some(1234)
    );
    assert_eq!(
        daemon_state
            .active_experiment
            .as_ref()
            .map(|experiment| experiment.safety_class.clone()),
        Some(SafetyClass::ReversibleLowRisk)
    );
    assert_eq!(
        daemon_state
            .active_rollback
            .as_ref()
            .map(|rollback| rollback.rollback_available),
        Some(false)
    );
    assert_eq!(
        daemon_state
            .active_rollback
            .as_ref()
            .and_then(|rollback| rollback.manual_restore_command.as_deref()),
        Some("stutter daemon emergency-restore")
    );
    assert!(daemon_state.faulted.is_none());
}

#[tokio::test]
async fn autotune_start_apply_low_risk_without_auth_is_rejected_before_mode_validation() {
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
    assert!(state.active_autotune.lock().await.is_none());
}

#[tokio::test]
async fn autotune_start_apply_low_risk_on_non_loopback_is_rejected_even_with_valid_token() {
    let mut state_value = test_agent_state("0.0.0.0:9899".parse().unwrap(), Some("secret"));
    state_value.auth = AgentAuth {
        bearer_token: Some("secret".to_owned()),
        ..AgentAuth::default()
    };
    state_value.bind = "0.0.0.0:9899".parse().unwrap();
    let state = Arc::new(state_value);

    let response = autotune_start_handler(
        State(state.clone()),
        autotune_headers(Some("secret")),
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

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(state.active_autotune.lock().await.is_none());
}
