use super::*;

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
