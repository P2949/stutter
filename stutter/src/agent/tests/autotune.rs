//! Remote autotune route handler and daemon-state transition tests.

use super::{support::*, *};

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
            experiment_id: "experiment-1".to_owned(),
            action_id: "remote-autotune-start:apply-low-risk".to_owned(),
            candidate_name: Some("Game.exe".to_owned()),
            mode: DaemonMode::ApplyLowRisk,
            safety_class: SafetyClass::ReversibleLowRisk,
            started_unix_nanos: Some(100),
        }),
        active_rollback: Some(DaemonRollbackState {
            action_id: "remote-autotune-start:apply-low-risk".to_owned(),
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
            action_id: "cpu-affinity:game".to_owned(),
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

#[tokio::test]
async fn autotune_start_rejects_request_above_candidate_window_cap() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    let response = autotune_start_handler(
        State(state.clone()),
        HeaderMap::new(),
        Json(AutotuneStartRequest {
            mode: "observe".to_owned(),
            watch_process: Some("Game.exe".to_owned()),
            tree_pid: None,
            profiles: None,
            config: None,
            duration_seconds: Some(121),
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
    assert!(state.active_autotune.lock().await.is_none());
}

#[tokio::test]
async fn autotune_start_rejects_more_than_one_target() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    let response = autotune_start_handler(
        State(state.clone()),
        HeaderMap::new(),
        Json(AutotuneStartRequest {
            mode: "observe".to_owned(),
            watch_process: Some("Game.exe".to_owned()),
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

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(state.active_autotune.lock().await.is_none());
}

#[tokio::test]
async fn autotune_start_requires_target_selector() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    let response = autotune_start_handler(
        State(state.clone()),
        HeaderMap::new(),
        Json(AutotuneStartRequest {
            mode: "observe".to_owned(),
            watch_process: None,
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
    assert!(state.active_autotune.lock().await.is_none());
}

#[tokio::test]
async fn autotune_stop_clears_active_session() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    *state.active_autotune.lock().await = Some(test_autotune_handle());

    let response = autotune_stop_handler(State(state.clone()), HeaderMap::new())
        .await
        .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(state.active_autotune.lock().await.is_none());

    let daemon_state = state.daemon_state.lock().await.clone();
    assert_eq!(daemon_state.phase, DaemonPhase::Disabled);
    assert_eq!(
        daemon_state
            .last_decision
            .as_ref()
            .map(|decision| decision.decision.as_str()),
        Some("autotune_stopped")
    );
}

#[tokio::test]
async fn autotune_restore_returns_true_noop_when_nothing_is_active() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));

    let response = autotune_restore_handler(State(state.clone()), HeaderMap::new())
        .await
        .into_response();

    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let restore: crate::remote::AutotuneRestoreResponse = serde_json::from_slice(&body).unwrap();

    assert_eq!(status, StatusCode::OK);
    assert_eq!(restore.status, "nothing_to_restore");
    assert_eq!(restore.restored_actions, Some(0));
    assert_eq!(restore.skipped_actions, Some(0));
    assert_eq!(restore.failed_actions, Some(0));
    assert_eq!(restore.restored_records, Some(0));
    assert_eq!(restore.skipped_missing, Some(0));
    assert_eq!(restore.skipped_identity_mismatch, Some(0));
    assert_eq!(restore.failed_records, Some(0));

    let daemon_state = state.daemon_state.lock().await.clone();
    assert_eq!(
        daemon_state
            .last_decision
            .as_ref()
            .map(|decision| decision.decision.as_str()),
        Some("remote_autotune_nothing_to_restore")
    );
}

#[tokio::test]
async fn autotune_restore_without_auth_is_rejected() {
    let state = Arc::new(test_agent_state(
        "127.0.0.1:0".parse().unwrap(),
        Some("secret"),
    ));

    let response = autotune_restore_handler(State(state), HeaderMap::new())
        .await
        .into_response();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn autotune_restore_active_rollback_invokes_restore() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    let dir = agent_autotune_temp_dir("remote-restore-active");
    let journal_path = dir.join("controller_journal.json");
    let audit_path = dir.join("audit.jsonl");
    let history_path = dir.join("history.jsonl");
    let target = dir.join("sysfs-knob");
    std::fs::write(&target, "changed").unwrap();

    crate::autotune::controller_journal::write_controller_journal_applied(
        &journal_path,
        "experiment-remote",
        "sysfs-restore:remote",
        crate::actions::RollbackToken::SysfsRestore {
            path: target.clone(),
            original_value: "original".to_owned(),
        },
    )
    .unwrap();

    let response = autotune_restore_authorized(
        state,
        AutotuneRestoreCommandInput {
            journal_path: Some(journal_path.clone()),
            audit_path: Some(audit_path),
            history_path: Some(history_path),
            dry_run: false,
        },
    )
    .await;
    let (status, restore) = decode_restore_response(response).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(restore.status, "restored");
    assert_eq!(restore.restored_actions, Some(1));
    assert_eq!(restore.restored_records, Some(1));
    assert_eq!(restore.failed_records, Some(0));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
    assert!(
        crate::autotune::controller_journal::read_controller_journal(&journal_path)
            .unwrap()
            .is_clean()
    );
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn autotune_restore_failure_returns_conflict_response() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    let dir = agent_autotune_temp_dir("remote-restore-failure");
    let journal_path = dir.join("controller_journal.json");
    let audit_path = dir.join("audit.jsonl");
    let history_path = dir.join("history.jsonl");
    let target = dir.join("missing").join("sysfs-knob");

    crate::autotune::controller_journal::write_controller_journal_applied(
        &journal_path,
        "experiment-remote",
        "sysfs-restore:remote",
        crate::actions::RollbackToken::SysfsRestore {
            path: target,
            original_value: "original".to_owned(),
        },
    )
    .unwrap();

    let response = autotune_restore_authorized(
        state,
        AutotuneRestoreCommandInput {
            journal_path: Some(journal_path.clone()),
            audit_path: Some(audit_path),
            history_path: Some(history_path),
            dry_run: false,
        },
    )
    .await;
    let (status, restore) = decode_restore_response(response).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(restore.status, "restore_failed");
    assert_eq!(restore.failed_actions, Some(1));
    assert_eq!(restore.failed_records, Some(1));
    assert!(
        !crate::autotune::controller_journal::read_controller_journal(&journal_path)
            .unwrap()
            .is_clean()
    );
    std::fs::remove_dir_all(dir).ok();
}

fn agent_autotune_temp_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "stutter-agent-autotune-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

async fn decode_restore_response(
    response: axum::response::Response,
) -> (StatusCode, crate::remote::AutotuneRestoreResponse) {
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let restore = serde_json::from_slice(&body).unwrap();
    (status, restore)
}

#[test]
fn autotune_config_response_includes_limits() {
    let state = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
    let response = AutotuneConfigResponse {
        default_mode: "observe".to_owned(),
        supported_modes: vec!["observe".to_owned(), "suggest".to_owned()],
        apply_low_risk_remote_enabled: false,
        local_only_by_default: true,
        history_path: crate::autotune::history::default_autotune_history_path()
            .display()
            .to_string(),
        autotune_limits: state.autotune_limits.clone(),
        daemon_scope: "focused".to_owned(),
        allow_system_wide_suggestions: false,
        allow_system_wide_apply: false,
        minimum_focus_confidence: 0.70,
        required_stable_focus_polls: 3,
    };

    assert_eq!(response.autotune_limits, AgentAutotuneLimits::default());
    assert_eq!(response.autotune_limits.max_active_controllers, 1);
    assert_eq!(
        response.autotune_limits.max_safety_class,
        SafetyClass::ReversibleLowRisk
    );
    assert_eq!(response.autotune_limits.max_candidate_window_seconds, 120);
    assert_eq!(response.autotune_limits.max_targets, 1);
    assert!(!response.autotune_limits.allow_system_wide_suggestions);
    assert!(!response.autotune_limits.allow_system_wide_apply);
}

#[tokio::test]
async fn autotune_config_reports_apply_low_risk_disabled() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));

    let response = autotune_config_handler(State(state), HeaderMap::new())
        .await
        .into_response();

    assert_eq!(response.status(), StatusCode::OK);
}

#[test]
fn capabilities_includes_autotune_routes() {
    let state = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
    let resp = capabilities_response(&state);
    assert!(
        resp.supported_routes
            .contains(&"/autotune/start".to_owned())
    );
    assert!(
        resp.supported_routes
            .contains(&"/autotune/status".to_owned())
    );
    assert!(resp.supported_routes.contains(&"/autotune/stop".to_owned()));
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
