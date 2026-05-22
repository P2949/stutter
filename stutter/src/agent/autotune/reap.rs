//! Remote autotune task reaping.

use super::{
    state::{
        daemon_mode_from_agent_mode, daemon_state_for_autotune_stop, mark_agent_daemon_fault,
        replace_agent_daemon_state,
    },
    *,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AutotuneReapStatus {
    NoActiveSession,
    StillActive,
    Completed,
    Failed,
}

pub(crate) async fn reap_finished_autotune(state: &AgentState) -> AutotuneReapStatus {
    let handle = {
        let mut active = state.active_autotune.lock().await;
        match active.as_ref() {
            Some(handle) if handle.join.is_finished() => active.take(),
            Some(_) => return AutotuneReapStatus::StillActive,
            None => return AutotuneReapStatus::NoActiveSession,
        }
    };

    let Some(handle) = handle else {
        return AutotuneReapStatus::NoActiveSession;
    };

    let mode = daemon_mode_from_agent_mode(&handle.mode);
    match handle.join.await {
        Ok(Ok(exit)) => {
            audit_agent_event(
                "remote-autotune-reap",
                true,
                0,
                format!("reason={}", exit.reason),
            );
            replace_agent_daemon_state(state, daemon_state_for_autotune_stop(mode, &exit)).await;
            AutotuneReapStatus::Completed
        }
        Ok(Err(err)) => {
            audit_agent_event("remote-autotune-reap", false, 0, format!("error={err:#}"));
            mark_agent_daemon_fault(
                state,
                mode,
                "autotune_controller_failed",
                format!("autotune controller failed: {err:#}"),
            )
            .await;
            AutotuneReapStatus::Failed
        }
        Err(err) => {
            audit_agent_event(
                "remote-autotune-reap",
                false,
                0,
                format!("join_error={err}"),
            );
            mark_agent_daemon_fault(
                state,
                mode,
                "autotune_controller_join_failed",
                format!("autotune controller task join failed: {err}"),
            )
            .await;
            AutotuneReapStatus::Failed
        }
    }
}
