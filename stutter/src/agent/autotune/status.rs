//! Remote autotune status endpoint.

use super::{reap::reap_finished_autotune, *};

pub(crate) async fn autotune_status_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) = authorize(&headers, &state.auth) {
        return status.into_response();
    }

    reap_finished_autotune(&state).await;

    let active = state.active_autotune.lock().await;
    let daemon_state = state.daemon_state.lock().await.clone();
    let disk_status = crate::autotune::status::load_autotune_status(
        &crate::autotune::history::default_autotune_history_path(),
    )
    .ok();

    let response = match active.as_ref() {
        Some(handle) => AutotuneStatusResponse {
            active: true,
            mode: Some(handle.mode.clone()),
            watch_process: handle.watch_process.clone(),
            tree_pid: handle.tree_pid,
            started_unix_nanos: Some(handle.started_unix_nanos),
            focus_group: disk_status
                .as_ref()
                .and_then(|status| status.focus_group.clone()),
            target_root: handle.tree_pid.or_else(|| {
                disk_status
                    .as_ref()
                    .and_then(|status| status.target.as_ref().map(|target| target.pid))
            }),
            current_score: disk_status.as_ref().and_then(|status| status.current_score),
            active_profile: disk_status
                .as_ref()
                .and_then(|status| status.active_profile.clone()),
            last_decision: disk_status
                .as_ref()
                .map(|status| status.last_decision.clone()),
            rollback_available: disk_status
                .as_ref()
                .map(|status| status.rollback_available)
                .unwrap_or(false),
            cooldown_remaining_seconds: disk_status
                .as_ref()
                .and_then(|status| status.cooldown_remaining_seconds),
            data_quality: disk_status
                .as_ref()
                .and_then(|status| status.data_quality.clone()),
            last_fault: disk_status
                .as_ref()
                .and_then(|status| status.last_fault.clone()),
            manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
            daemon_state: daemon_state.clone(),
            message: format!("autotune {} controller active", handle.mode),
        },
        None => AutotuneStatusResponse {
            active: false,
            mode: None,
            watch_process: None,
            tree_pid: None,
            started_unix_nanos: None,
            focus_group: disk_status
                .as_ref()
                .and_then(|status| status.focus_group.clone()),
            target_root: disk_status
                .as_ref()
                .and_then(|status| status.target.as_ref().map(|target| target.pid)),
            current_score: disk_status.as_ref().and_then(|status| status.current_score),
            active_profile: disk_status
                .as_ref()
                .and_then(|status| status.active_profile.clone()),
            last_decision: disk_status
                .as_ref()
                .map(|status| status.last_decision.clone()),
            rollback_available: disk_status
                .as_ref()
                .map(|status| status.rollback_available)
                .unwrap_or(false),
            cooldown_remaining_seconds: disk_status
                .as_ref()
                .and_then(|status| status.cooldown_remaining_seconds),
            data_quality: disk_status
                .as_ref()
                .and_then(|status| status.data_quality.clone()),
            last_fault: disk_status
                .as_ref()
                .and_then(|status| status.last_fault.clone()),
            manual_restore_command: Some("stutter daemon emergency-restore".to_owned()),
            daemon_state,
            message: "no autotune session active".to_owned(),
        },
    };

    Json(response).into_response()
}
