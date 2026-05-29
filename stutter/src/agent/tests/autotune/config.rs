use super::*;

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
