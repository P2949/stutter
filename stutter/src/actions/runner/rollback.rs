//! Rollback helpers for audited action-runner failure paths.
//!
//! Owns timeout rollback and hook-failure rollback handling. It must not perform preflight,
//! policy checks, dry-run/apply/verify sequencing, or final audit result assembly.

use std::{
    path::Path,
    time::{Duration, Instant},
};

use super::{
    ActionHooks,
    audit::{RunnerAuditEventUpdate, append_runner_audit_event},
};
use crate::{
    actions::{ActionError, ActionPhase, RollbackToken, TuningAction},
    audit::AuditEvent,
};

pub(super) fn total_timeout_error(
    started: Instant,
    max_total_duration: Option<Duration>,
    phase: ActionPhase,
) -> Option<ActionError> {
    let max_total_duration = max_total_duration?;
    let elapsed = started.elapsed();

    (elapsed > max_total_duration)
        .then(|| ActionError::timeout(phase, elapsed.as_millis(), max_total_duration.as_millis()))
}

pub(super) fn rollback_after_timeout<A>(
    action: &A,
    rollback: &RollbackToken,
    timeout_error: ActionError,
    audit_path: &Path,
    audit_event: &AuditEvent,
    hooks: &mut ActionHooks<'_>,
) -> ActionError
where
    A: TuningAction,
{
    let Some(timeout) = timeout_error.timeout_details() else {
        return timeout_error;
    };
    let phase = timeout.phase;
    let elapsed_ms = timeout.elapsed_ms;
    let timeout_ms = timeout.timeout_ms;

    match action.rollback(rollback) {
        Ok(()) => {
            if let Err(hook_err) = hooks.run_after_rollback(rollback) {
                append_runner_audit_event(
                    audit_path,
                    audit_event,
                    RunnerAuditEventUpdate::new(
                        Some(ActionPhase::Rollback),
                        false,
                        rollback.affected_tasks(),
                        rollback.restore_path().cloned(),
                        Some("RollbackHookFailed".to_owned()),
                        "rollback completed after action timeout, but after-rollback hook failed",
                    ),
                );

                return ActionError::rollback(format!(
                    "rollback completed after action timeout, but after-rollback hook failed: {hook_err:#}"
                ));
            }

            append_runner_audit_event(
                audit_path,
                audit_event,
                RunnerAuditEventUpdate::new(
                    Some(ActionPhase::Rollback),
                    true,
                    rollback.affected_tasks(),
                    rollback.restore_path().cloned(),
                    None,
                    "rollback completed after action timeout",
                ),
            );

            ActionError::timeout_rollback_completed(phase, elapsed_ms, timeout_ms)
        }
        Err(rollback_error) => {
            ActionError::timeout_rollback_failure(phase, elapsed_ms, timeout_ms, rollback_error)
        }
    }
}

pub(super) fn rollback_after_apply_hook_failure<A>(
    action: &A,
    rollback: &RollbackToken,
    hook_error: anyhow::Error,
    audit_path: &Path,
    audit_event: &AuditEvent,
    hooks: &mut ActionHooks<'_>,
) -> ActionError
where
    A: TuningAction,
{
    let hook_message = format!("after-apply hook failed after mutation: {hook_error:#}");

    match action.rollback(rollback) {
        Ok(()) => match hooks.run_after_rollback(rollback) {
            Ok(()) => {
                append_runner_audit_event(
                    audit_path,
                    audit_event,
                    RunnerAuditEventUpdate::new(
                        Some(ActionPhase::Rollback),
                        true,
                        rollback.affected_tasks(),
                        rollback.restore_path().cloned(),
                        None,
                        "rollback completed after after-apply hook failure",
                    ),
                );

                ActionError::apply(format!("{hook_message}; rollback completed"))
            }
            Err(rollback_hook_error) => {
                append_runner_audit_event(
                    audit_path,
                    audit_event,
                    RunnerAuditEventUpdate::new(
                        Some(ActionPhase::Rollback),
                        false,
                        rollback.affected_tasks(),
                        rollback.restore_path().cloned(),
                        Some("RollbackHookFailed".to_owned()),
                        "rollback completed after after-apply hook failure, but after-rollback hook failed",
                    ),
                );

                ActionError::rollback(format!(
                    "{hook_message}; rollback completed; after-rollback hook failed: {rollback_hook_error:#}"
                ))
            }
        },
        Err(rollback_error) => ActionError::emergency_rollback(hook_message, rollback_error),
    }
}
