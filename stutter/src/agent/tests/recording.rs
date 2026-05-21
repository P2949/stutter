//! Recording task lifecycle and reaping tests.

use super::{support::*, *};

#[tokio::test]
async fn finished_recording_is_cleared_by_record_status() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    *state.active_run.lock().await = Some(completed_record_handle("finished-recording"));
    wait_for_finished_recording(&state).await;

    let response = status_handler(State(state.clone()), HeaderMap::new())
        .await
        .into_response();
    let (status, record_status) = decode_record_status(response).await;

    assert_eq!(status, StatusCode::OK);
    assert!(!record_status.active);
    assert_eq!(record_status.run_id, None);
    assert!(state.active_run.lock().await.is_none());

    let daemon_state = state.daemon_state.lock().await.clone();
    assert_eq!(daemon_state.phase, DaemonPhase::Disabled);
    assert_eq!(
        daemon_state
            .last_decision
            .as_ref()
            .map(|decision| decision.decision.as_str()),
        Some("record_completed")
    );
}

#[tokio::test]
async fn finished_recording_no_longer_blocks_new_recording_start() {
    let mut state_value = test_agent_state("127.0.0.1:0".parse().unwrap(), None);
    state_value.runs_dir = agent_recording_temp_dir("finished_recording_no_longer_blocks_start");
    let state = Arc::new(state_value);
    *state.active_run.lock().await = Some(completed_record_handle("stale-recording"));
    wait_for_finished_recording(&state).await;

    let mut request = minimal_remote_request();
    request.target_pids = vec![std::process::id()];
    request.duration_seconds = Some(30);

    let response = start_record_handler(State(state.clone()), HeaderMap::new(), Json(request))
        .await
        .into_response();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = String::from_utf8_lossy(&body);

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert!(body.contains("\"status\":\"started\""), "body={body}");

    if let Some(handle) = state.active_run.lock().await.take() {
        let _ = handle.stop_tx.send(());
        let _ = handle.join.await;
    }
}

#[tokio::test]
async fn failed_recording_join_marks_daemon_failed_and_degraded() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    *state.active_run.lock().await = Some(panicking_record_handle("panicking-recording"));
    wait_for_finished_recording(&state).await;

    let response = daemon_status_handler(State(state.clone()), HeaderMap::new())
        .await
        .into_response();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(state.active_run.lock().await.is_none());

    let daemon_state = state.daemon_state.lock().await.clone();
    assert_eq!(daemon_state.phase, DaemonPhase::Faulted);
    assert!(daemon_state.faulted.is_some());
    assert!(!daemon_state.degraded.is_empty());
    assert_eq!(
        daemon_state
            .last_decision
            .as_ref()
            .map(|decision| decision.decision.as_str()),
        Some("record_join_failed")
    );
}

#[tokio::test]
async fn reaper_reports_still_active_without_clearing_handle() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    *state.active_run.lock().await = Some(RunHandle {
        id: "active-recording".to_owned(),
        stop_tx,
        join: tokio::spawn(async move {
            let _ = stop_rx.await;
            Ok::<String, anyhow::Error>("stopped".to_owned())
        }),
    });

    assert_eq!(
        reap_finished_recording(&state).await,
        RecordingReapStatus::StillActive
    );
    let handle = state.active_run.lock().await.take().unwrap();
    let _ = handle.stop_tx.send(());
    let _ = handle.join.await;
}

#[tokio::test]
async fn stop_recording_releases_active_run_lock_before_join_wait() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let (stop_seen_tx, stop_seen_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();
    *state.active_run.lock().await = Some(RunHandle {
        id: "slow-stop-recording".to_owned(),
        stop_tx,
        join: tokio::spawn(async move {
            let _ = stop_rx.await;
            let _ = stop_seen_tx.send(());
            let _ = release_rx.await;
            Ok::<String, anyhow::Error>("stopped".to_owned())
        }),
    });

    let stop_task = tokio::spawn({
        let state = Arc::clone(&state);
        async move {
            stop_record_handler(State(state), HeaderMap::new())
                .await
                .into_response()
        }
    });
    stop_seen_rx.await.unwrap();

    let active = tokio::time::timeout(Duration::from_millis(100), state.active_run.lock())
        .await
        .expect("active_run lock should be available while stop waits for join");
    assert!(active.is_none());
    drop(active);

    release_tx.send(()).unwrap();
    let response = stop_task.await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(state.active_run.lock().await.is_none());
}

fn completed_record_handle(run_id: &str) -> RunHandle {
    let (stop_tx, _stop_rx) = tokio::sync::oneshot::channel();
    RunHandle {
        id: run_id.to_owned(),
        stop_tx,
        join: tokio::spawn(async { Ok::<String, anyhow::Error>("completed".to_owned()) }),
    }
}

fn panicking_record_handle(run_id: &str) -> RunHandle {
    let (stop_tx, _stop_rx) = tokio::sync::oneshot::channel();
    RunHandle {
        id: run_id.to_owned(),
        stop_tx,
        join: tokio::spawn(async {
            let should_panic = true;
            if should_panic {
                panic!("record task panicked");
            }
            Ok::<String, anyhow::Error>("unreachable".to_owned())
        }),
    }
}

async fn wait_for_finished_recording(state: &AgentState) {
    for _ in 0..100 {
        let finished = state
            .active_run
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

    panic!("recording task did not finish");
}

async fn decode_record_status(
    response: axum::response::Response,
) -> (StatusCode, RecordStatusResponse) {
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let decoded = serde_json::from_slice(&body).unwrap();
    (status, decoded)
}

fn agent_recording_temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "stutter-agent-recording-test-{name}-{}",
        crate::audit::unix_nanos_now()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
