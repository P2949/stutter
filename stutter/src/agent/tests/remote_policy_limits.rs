//! Remote policy tests for bearer-token and loopback-limit edge cases.

use axum::http::{HeaderMap, StatusCode};

use super::*;
use crate::{
    actions::SafetyClass,
    daemon_policy::DaemonMode,
    remote::{AgentAutotuneLimits, AutotuneStartRequest},
};

fn test_agent_state(token: Option<&str>, bind_loopback: bool) -> AgentState {
    AgentState {
        active_run: Mutex::new(None),
        active_autotune: Mutex::new(None),
        daemon_state: Mutex::new(DaemonState::default()),
        runs_dir: PathBuf::from("/tmp"),
        auth: AgentAuth {
            bearer_token: token.map(str::to_owned),
            ..AgentAuth::default()
        },
        bind: if bind_loopback {
            "127.0.0.1:0".parse().unwrap()
        } else {
            "0.0.0.0:0".parse().unwrap()
        },
        unix_socket: None,
        limits: AgentLimits {
            max_duration_seconds: 300,
            max_targets: 128,
            max_concurrent_recordings: 1,
        },
        autotune_limits: AgentAutotuneLimits {
            max_active_controllers: 1,
            max_mode: DaemonMode::ApplyLowRisk,
            max_safety_class: SafetyClass::ReversibleLowRisk,
            allow_high_risk: false,
            max_candidate_window_seconds: 60,
            max_targets: 10,
            allow_system_wide_suggestions: false,
            allow_system_wide_apply: false,
        },
        health_thresholds: SystemHealthThresholds::default(),
    }
}

fn autotune_start_request(mode: &str) -> AutotuneStartRequest {
    AutotuneStartRequest {
        mode: mode.to_owned(),
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
    }
}

#[test]
fn remote_policy_rejects_apply_without_bearer_token() {
    let state = test_agent_state(Some("secret"), true);
    let request = autotune_start_request("apply-low-risk");

    let res = policy_for_remote_autotune_start(&HeaderMap::new(), &state, &request);
    assert_eq!(res.unwrap_err().status, StatusCode::UNAUTHORIZED);

    let mut headers = HeaderMap::new();
    headers.insert("Authorization", "Bearer wrong".parse().unwrap());
    let res = policy_for_remote_autotune_start(&headers, &state, &request);
    assert_eq!(res.unwrap_err().status, StatusCode::FORBIDDEN);

    let mut headers = HeaderMap::new();
    headers.insert("Authorization", "Bearer secret".parse().unwrap());
    let res = policy_for_remote_autotune_start(&headers, &state, &request);
    assert!(res.is_ok());
}

#[test]
fn remote_policy_rejects_apply_on_non_loopback_bind() {
    let state = test_agent_state(Some("secret"), false);
    let request = autotune_start_request("apply-low-risk");

    let mut headers = HeaderMap::new();
    headers.insert("Authorization", "Bearer secret".parse().unwrap());

    let res = policy_for_remote_autotune_start(&headers, &state, &request);
    assert_eq!(res.unwrap_err().status, StatusCode::FORBIDDEN);
}

#[test]
fn remote_policy_response_uses_policy_explanation_text() {
    let state = test_agent_state(Some("secret"), false);
    let request = autotune_start_request("apply-low-risk");

    let mut headers = HeaderMap::new();
    headers.insert("Authorization", "Bearer secret".parse().unwrap());

    let err = policy_for_remote_autotune_start(&headers, &state, &request).unwrap_err();

    assert_eq!(err.status, StatusCode::FORBIDDEN);
    assert!(err.response_message.contains("loopback"));
    assert!(err.audit_message.contains("loopback"));
}
