//! Remote autotune policy construction and rejection tests.

use super::{support::*, *};

#[test]
fn remote_autotune_observe_builds_observe_policy_without_token() {
    let state = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
    let headers = HeaderMap::new();

    let policy =
        policy_for_remote_autotune_start(&headers, &state, &autotune_request("observe")).unwrap();

    assert_eq!(policy.mode, DaemonMode::Observe);
    assert_eq!(policy.source, ActionSource::RemoteAgent);
}

#[test]
fn remote_autotune_apply_requires_configured_bearer_token() {
    let state = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
    let headers = HeaderMap::new();

    let rejection =
        policy_for_remote_autotune_start(&headers, &state, &autotune_request("apply-low-risk"))
            .unwrap_err();

    assert_eq!(rejection.status, StatusCode::UNAUTHORIZED);
    assert!(rejection.response_message.contains("bearer token"));
}

#[test]
fn remote_autotune_apply_requires_valid_bearer_token() {
    let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
    let headers = HeaderMap::new();

    let rejection =
        policy_for_remote_autotune_start(&headers, &state, &autotune_request("apply-low-risk"))
            .unwrap_err();

    assert_eq!(rejection.status, StatusCode::UNAUTHORIZED);
    assert!(rejection.response_message.contains("valid bearer token"));
}

#[test]
fn remote_autotune_all_apply_modes_require_bearer_token_before_limit_rejection() {
    let state = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
    let headers = HeaderMap::new();

    for mode in ["apply-low-risk", "apply-medium-risk", "apply-high-risk"] {
        let rejection = policy_for_remote_autotune_start(&headers, &state, &autotune_request(mode))
            .unwrap_err();

        assert_eq!(rejection.status, StatusCode::UNAUTHORIZED);
        assert!(rejection.response_message.contains("bearer token"));
    }
}

#[test]
fn remote_autotune_apply_requires_loopback_bind() {
    let state = test_agent_state("0.0.0.0:0".parse().unwrap(), Some("secret"));
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        axum::http::HeaderValue::from_static("Bearer secret"),
    );

    let rejection =
        policy_for_remote_autotune_start(&headers, &state, &autotune_request("apply-low-risk"))
            .unwrap_err();

    assert_eq!(rejection.status, StatusCode::FORBIDDEN);
    assert!(rejection.response_message.contains("loopback"));
}

#[test]
fn remote_autotune_apply_low_risk_builds_low_risk_policy_with_valid_auth() {
    let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        axum::http::HeaderValue::from_static("Bearer secret"),
    );

    let policy =
        policy_for_remote_autotune_start(&headers, &state, &autotune_request("apply-low-risk"))
            .unwrap();

    assert_eq!(policy.mode, DaemonMode::ApplyLowRisk);
    assert_eq!(policy.source, ActionSource::RemoteAgent);
    assert!(!policy.allow_system_wide_suggestions);
    assert!(!policy.allow_system_wide_apply);
    assert!(!policy.allow_high_risk);
}

#[test]
fn remote_autotune_apply_rejects_when_system_health_blocks_apply() {
    let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        axum::http::HeaderValue::from_static("Bearer secret"),
    );
    let health = SystemHealthSnapshot {
        ok_for_apply: false,
        reason_code: Some("cpu_overheated".to_owned()),
        ..SystemHealthSnapshot::default()
    };

    let rejection = policy_for_remote_autotune_start_with_safety_context(
        &headers,
        &state,
        &autotune_request("apply-low-risk"),
        &DaemonState::default(),
        &health,
        &test_capabilities(),
    )
    .unwrap_err();

    assert_eq!(rejection.status, StatusCode::BAD_REQUEST);
    assert!(rejection.response_message.contains("cpu_overheated"));
    assert!(rejection.audit_message.contains("cpu_overheated"));
}

#[test]
fn remote_autotune_policy_build_is_deterministic_for_same_context() {
    let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        axum::http::HeaderValue::from_static("Bearer secret"),
    );
    let request = autotune_request("apply-low-risk");
    let first_context =
        remote_policy_context_for_request(&state, authorize(&headers, &state.auth).is_ok());
    let second_context =
        remote_policy_context_for_request(&state, authorize(&headers, &state.auth).is_ok());

    let first =
        daemon_policy_for_remote_mode(DaemonMode::ApplyLowRisk, &state, &request, first_context);
    let second =
        daemon_policy_for_remote_mode(DaemonMode::ApplyLowRisk, &state, &request, second_context);

    assert_eq!(first, second);
    assert!(first.remote_apply.allow_remote_apply);
    assert_eq!(
        first.remote_apply.max_remote_targets,
        state.autotune_limits.max_targets
    );
}

#[test]
fn remote_autotune_high_risk_rejected_by_default_limits() {
    let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        axum::http::HeaderValue::from_static("Bearer secret"),
    );

    let rejection =
        policy_for_remote_autotune_start(&headers, &state, &autotune_request("apply-high-risk"))
            .unwrap_err();

    assert_eq!(rejection.status, StatusCode::BAD_REQUEST);
    assert!(
        rejection
            .response_message
            .contains("configured remote limits")
    );
}

#[test]
fn autotune_start_rejects_wrong_bearer_token() {
    let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        axum::http::HeaderValue::from_static("Bearer wrong"),
    );

    let rejection =
        policy_for_remote_autotune_start(&headers, &state, &autotune_request("observe"))
            .unwrap_err();

    assert_eq!(rejection.status, StatusCode::FORBIDDEN);
}

#[test]
fn remote_autotune_observe_target_count_rejection_uses_policy_explanation() {
    let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        axum::http::HeaderValue::from_static("Bearer secret"),
    );
    let mut request = autotune_request("observe");
    request.tree_pid = Some(1234);
    request.watch_process = Some("game".to_owned());

    let rejection = policy_for_remote_autotune_start(&headers, &state, &request).unwrap_err();

    assert_eq!(rejection.status, StatusCode::BAD_REQUEST);
    assert!(rejection.response_message.contains("max_targets"));
    assert!(rejection.audit_message.contains("max_targets"));
}

#[test]
fn remote_autotune_target_count_rejection_uses_policy_explanation() {
    let state = test_agent_state("127.0.0.1:0".parse().unwrap(), Some("secret"));
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::AUTHORIZATION,
        axum::http::HeaderValue::from_static("Bearer secret"),
    );
    let mut request = autotune_request("apply-low-risk");
    request.tree_pid = Some(1234);
    request.watch_process = Some("game".to_owned());

    let rejection = policy_for_remote_autotune_start(&headers, &state, &request).unwrap_err();

    assert_eq!(rejection.status, StatusCode::BAD_REQUEST);
    assert!(rejection.response_message.contains("max_targets"));
    assert!(rejection.audit_message.contains("max_targets"));
}

#[test]
fn supported_remote_modes_derive_from_limits() {
    let limits = AgentAutotuneLimits::default();

    assert_eq!(
        supported_remote_mode_labels(&limits),
        vec![
            "observe".to_owned(),
            "suggest".to_owned(),
            "apply-low-risk".to_owned()
        ]
    );
}
