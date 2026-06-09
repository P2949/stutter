//! Remote autotune stop endpoint.

use super::{
    state::{
        daemon_mode_from_agent_mode, daemon_state_for_agent_decision,
        daemon_state_for_autotune_stop, mark_agent_daemon_fault, replace_agent_daemon_state,
    },
    *,
};

pub(crate) async fn autotune_stop_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) =
        authorize_agent_privileged_operation(&headers, &state, PrivilegedOperation::ControlDaemon)
    {
        return status.into_response();
    }

    let handle = {
        let mut active = state.active_autotune.lock().await;
        active.take()
    };

    let Some(handle) = handle else {
        audit_agent_event(
            "remote-autotune-stop",
            true,
            0,
            "active_before_stop=false".to_owned(),
        );

        replace_agent_daemon_state(
            &state,
            daemon_state_for_agent_decision(
                DaemonMode::Observe,
                DaemonPhase::Disabled,
                "autotune_stop_no_active_session",
                "no remote autotune session was active",
            ),
        )
        .await;

        return Json(AutotuneStopResponse {
            status: "no_active_session".to_owned(),
            message: "no remote autotune session was active".to_owned(),
        })
        .into_response();
    };

    let stop_mode = daemon_mode_from_agent_mode(&handle.mode);
    let _ = handle.stop_tx.send(());

    match handle.join.await {
        Ok(Ok(exit)) => {
            audit_agent_event(
                "remote-autotune-stop",
                true,
                0,
                format!("active_before_stop=true reason={}", exit.reason),
            );

            replace_agent_daemon_state(&state, daemon_state_for_autotune_stop(stop_mode, &exit))
                .await;

            Json(AutotuneStopResponse {
                status: "stopped".to_owned(),
                message: format!("remote autotune session stopped: {}", exit.reason),
            })
            .into_response()
        }
        Ok(Err(err)) => {
            audit_agent_event(
                "remote-autotune-stop",
                false,
                0,
                format!("active_before_stop=true error={err:#}"),
            );

            mark_agent_daemon_fault(
                &state,
                stop_mode,
                "autotune_controller_failed",
                format!("autotune controller failed: {err:#}"),
            )
            .await;

            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("autotune controller failed: {err:#}"),
                }),
            )
                .into_response()
        }
        Err(err) => {
            audit_agent_event(
                "remote-autotune-stop",
                false,
                0,
                format!("active_before_stop=true join_error={err}"),
            );

            mark_agent_daemon_fault(
                &state,
                stop_mode,
                "autotune_controller_join_failed",
                format!("autotune controller task join failed: {err}"),
            )
            .await;

            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("autotune controller task join failed: {err}"),
                }),
            )
                .into_response()
        }
    }
}
