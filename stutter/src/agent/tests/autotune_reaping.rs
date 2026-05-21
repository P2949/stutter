//! Autotune task lifecycle and reaping tests.

use super::{support::*, *};

#[tokio::test]
async fn completed_autotune_is_reaped_by_status() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    *state.active_autotune.lock().await = Some(completed_autotune_handle("suggest"));
    wait_for_finished_autotune(&state).await;

    let response = autotune_status_handler(State(state.clone()), HeaderMap::new())
        .await
        .into_response();
    let (status, autotune_status) = decode_autotune_status(response).await;

    assert_eq!(status, StatusCode::OK);
    assert!(!autotune_status.active);
    assert_eq!(autotune_status.mode, None);
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
async fn completed_autotune_no_longer_blocks_new_session() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    *state.active_autotune.lock().await = Some(completed_autotune_handle("observe"));
    wait_for_finished_autotune(&state).await;

    let response = autotune_start_handler(
        State(state.clone()),
        HeaderMap::new(),
        Json(autotune_request("observe")),
    )
    .await
    .into_response();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = String::from_utf8_lossy(&body);

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(body.contains("\"status\":\"started\""), "body={body}");

    if let Some(handle) = state.active_autotune.lock().await.take() {
        let _ = handle.stop_tx.send(());
        let _ = handle.join.await;
    }
}

#[tokio::test]
async fn failed_autotune_join_marks_daemon_failed_and_degraded() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    *state.active_autotune.lock().await = Some(panicking_autotune_handle("suggest"));
    wait_for_finished_autotune(&state).await;

    let response = daemon_status_handler(State(state.clone()), HeaderMap::new())
        .await
        .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(state.active_autotune.lock().await.is_none());

    let daemon_state = state.daemon_state.lock().await.clone();
    assert_eq!(daemon_state.phase, DaemonPhase::Faulted);
    assert!(daemon_state.faulted.is_some());
    assert!(!daemon_state.degraded.is_empty());
    assert_eq!(
        daemon_state
            .last_decision
            .as_ref()
            .map(|decision| decision.decision.as_str()),
        Some("autotune_controller_join_failed")
    );
}

#[tokio::test]
async fn autotune_reaper_reports_still_active_without_clearing_handle() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    *state.active_autotune.lock().await = Some(test_autotune_handle());

    assert_eq!(
        reap_finished_autotune(&state).await,
        AutotuneReapStatus::StillActive
    );
    let handle = state.active_autotune.lock().await.take().unwrap();
    let _ = handle.stop_tx.send(());
    let _ = handle.join.await;
}

async fn decode_autotune_status(
    response: axum::response::Response,
) -> (StatusCode, crate::remote::AutotuneStatusResponse) {
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let status_body = serde_json::from_slice(&body).unwrap();
    (status, status_body)
}

fn completed_autotune_handle(mode: &str) -> AutotuneControllerHandle {
    let (stop_tx, _stop_rx) = tokio::sync::oneshot::channel();
    AutotuneControllerHandle {
        mode: mode.to_owned(),
        watch_process: Some("Game.exe".to_owned()),
        tree_pid: None,
        started_unix_nanos: crate::audit::unix_nanos_now(),
        stop_tx,
        join: tokio::spawn(async {
            Ok::<crate::autotune::runtime::AutotuneControllerExit, anyhow::Error>(
                crate::autotune::runtime::AutotuneControllerExit {
                    reason: "completed".to_owned(),
                    last_decision: None,
                },
            )
        }),
    }
}

fn panicking_autotune_handle(mode: &str) -> AutotuneControllerHandle {
    let (stop_tx, _stop_rx) = tokio::sync::oneshot::channel();
    AutotuneControllerHandle {
        mode: mode.to_owned(),
        watch_process: Some("Game.exe".to_owned()),
        tree_pid: None,
        started_unix_nanos: crate::audit::unix_nanos_now(),
        stop_tx,
        join: tokio::spawn(async {
            let should_panic = true;
            if should_panic {
                panic!("autotune task panicked");
            }
            Ok::<crate::autotune::runtime::AutotuneControllerExit, anyhow::Error>(
                crate::autotune::runtime::AutotuneControllerExit {
                    reason: "unreachable".to_owned(),
                    last_decision: None,
                },
            )
        }),
    }
}

async fn wait_for_finished_autotune(state: &AgentState) {
    for _ in 0..100 {
        let finished = state
            .active_autotune
            .lock()
            .await
            .as_ref()
            .is_some_and(|handle| handle.join.is_finished());
        if finished {
            return;
        }
        tokio::task::yield_now().await;
        tokio::time::sleep(Duration::from_millis(1)).await;
    }

    panic!("autotune task did not finish");
}
