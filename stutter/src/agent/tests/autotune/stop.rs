use super::*;

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
