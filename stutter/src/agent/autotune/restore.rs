//! Remote autotune restore endpoint.

use super::{state::mark_agent_daemon_fault, *};

pub(crate) async fn autotune_restore_handler(
    State(state): State<Arc<AgentState>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(status) =
        authorize_agent_privileged_operation(&headers, &state, PrivilegedOperation::RollbackAction)
    {
        audit_agent_event(
            "remote-autotune-restore",
            false,
            0,
            "unauthorized".to_owned(),
        );
        return status.into_response();
    }

    autotune_restore_authorized(
        state,
        AutotuneRestoreCommandInput {
            journal_path: None,
            audit_path: None,
            history_path: None,
            dry_run: false,
        },
    )
    .await
}

pub(crate) async fn autotune_restore_authorized(
    state: Arc<AgentState>,
    input: AutotuneRestoreCommandInput,
) -> axum::response::Response {
    let outcome = match restore_known_autotune_actions(input) {
        Ok(outcome) => outcome,
        Err(err) => {
            audit_agent_event(
                "remote-autotune-restore",
                false,
                0,
                format!("restore_failed error={err:#}"),
            );
            mark_agent_daemon_fault(
                &state,
                DaemonMode::ApplyLowRisk,
                "remote_autotune_restore_failed",
                format!("remote autotune restore failed: {err:#}"),
            )
            .await;
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("remote autotune restore failed: {err:#}"),
                }),
            )
                .into_response();
        }
    };

    let ok = matches!(
        &outcome.status,
        AutotuneRestoreStatus::Clean | AutotuneRestoreStatus::Restored
    );
    let status = match &outcome.status {
        AutotuneRestoreStatus::Clean => "nothing_to_restore",
        AutotuneRestoreStatus::Restored => "restored",
        AutotuneRestoreStatus::DryRun => "dry_run",
        AutotuneRestoreStatus::ApplyingWithoutRollbackToken | AutotuneRestoreStatus::Faulted => {
            "restore_failed"
        }
    };
    let message = format!(
        "remote autotune restore status={:?} restored_actions={} skipped_actions={} failed_actions={}",
        outcome.status, outcome.restored_actions, outcome.skipped_actions, outcome.failed_actions
    );

    audit_agent_event(
        "remote-autotune-restore",
        ok,
        outcome.restored_actions,
        message.clone(),
    );

    if ok {
        let decision = match outcome.status {
            AutotuneRestoreStatus::Clean => "remote_autotune_nothing_to_restore",
            _ => "remote_autotune_restored",
        };
        let mut daemon_state = state.daemon_state.lock().await;
        daemon_state.phase = DaemonPhase::Observe;
        daemon_state.active_experiment = None;
        daemon_state.active_rollback = None;
        daemon_state.faulted = None;
        daemon_state.degraded.clear();
        daemon_state.last_decision = Some(daemon_decision_state(decision, message.clone()));
    } else {
        mark_agent_daemon_fault(
            &state,
            DaemonMode::ApplyLowRisk,
            "remote_autotune_restore_incomplete",
            message.clone(),
        )
        .await;
    }

    let http_status = if ok {
        StatusCode::OK
    } else {
        StatusCode::CONFLICT
    };
    (
        http_status,
        Json(AutotuneRestoreResponse {
            status: status.to_owned(),
            message,
            restored_actions: Some(outcome.restored_actions),
            skipped_actions: Some(outcome.skipped_actions),
            failed_actions: Some(outcome.failed_actions),
            restored_records: Some(outcome.restored_records),
            skipped_missing: Some(outcome.skipped_missing),
            skipped_identity_mismatch: Some(outcome.skipped_identity_mismatch),
            failed_records: Some(outcome.failed_records),
            restore_messages: outcome.messages,
        }),
    )
        .into_response()
}
