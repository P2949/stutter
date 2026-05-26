//! Audited action execution flow.
//!
//! Owns preflight, dry-run, apply, verify, timeout, rollback, and terminal audit sequencing. It
//! must not own the runner DTO definitions, policy explanation mapping, or standalone rollback
//! helper implementations.

use std::{path::Path, time::Instant};

use super::{
    audit::{RunnerAuditEventUpdate, append_runner_audit_event},
    model::{ActionHooks, ActionRunPolicy, AuditedActionResult},
    policy::check_action_with_explanation,
    rollback::{rollback_after_apply_hook_failure, rollback_after_timeout, total_timeout_error},
};
use crate::{
    actions::{ActionError, ActionPhase, ActionPreflightReport, TuningAction},
    audit::{AuditEvent, unix_nanos_now},
    daemon_policy::PolicyIntent,
};

pub fn run_audited_action<A>(
    command: &str,
    action: &A,
    run_policy: ActionRunPolicy,
) -> Result<AuditedActionResult, ActionError>
where
    A: TuningAction,
{
    run_audited_action_with_audit_path(
        command,
        action,
        run_policy,
        &crate::audit::default_audit_log_path(),
    )
}

pub(crate) fn run_audited_action_with_hooks<A>(
    command: &str,
    action: &A,
    run_policy: ActionRunPolicy,
    hooks: ActionHooks<'_>,
) -> Result<AuditedActionResult, ActionError>
where
    A: TuningAction,
{
    run_audited_action_with_audit_path_and_hooks(
        command,
        action,
        run_policy,
        &crate::audit::default_audit_log_path(),
        hooks,
    )
}

pub fn run_audited_action_with_audit_path<A>(
    command: &str,
    action: &A,
    run_policy: ActionRunPolicy,
    audit_path: &Path,
) -> Result<AuditedActionResult, ActionError>
where
    A: TuningAction,
{
    run_audited_action_with_audit_path_and_hooks(
        command,
        action,
        run_policy,
        audit_path,
        ActionHooks::none(),
    )
}

pub(crate) fn run_audited_action_with_audit_path_and_hooks<A>(
    command: &str,
    action: &A,
    run_policy: ActionRunPolicy,
    audit_path: &Path,
    mut hooks: ActionHooks<'_>,
) -> Result<AuditedActionResult, ActionError>
where
    A: TuningAction,
{
    let dry_run = run_policy.dry_run;
    let started_unix_nanos = unix_nanos_now();
    let started_instant = Instant::now();
    let descriptor = action.descriptor();
    let action_id = descriptor.action_id.clone();
    let safety_class = descriptor.safety_class.clone();

    let mut audit_event = AuditEvent {
        schema_version: 1,
        unix_nanos: started_unix_nanos,
        command: command.to_owned(),
        action_id: Some(action_id.as_str().to_owned()),
        safety_class: Some(safety_class.clone()),
        dry_run,
        success: false,
        affected_tasks: 0,
        restore_path: None,
        action_phase: None,
        error_category: None,
        message: String::new(),
    };

    let result = (|| -> Result<AuditedActionResult, ActionError> {
        audit_event.action_phase = Some(crate::actions::ActionPhase::Preflight);
        let preflight_report =
            ActionPreflightReport::from_preflight_result(action_id.clone(), action.preflight());
        if preflight_report.is_blocked() {
            return Err(ActionError::preflight(preflight_report.blocker_messages()));
        }
        let preflight_warnings = preflight_report.warnings;
        let preflight_message = if preflight_warnings.is_empty() {
            "preflight successful".to_owned()
        } else {
            format!(
                "preflight successful with {} warning{}",
                preflight_warnings.len(),
                if preflight_warnings.len() == 1 {
                    ""
                } else {
                    "s"
                }
            )
        };
        append_runner_audit_event(
            audit_path,
            &audit_event,
            RunnerAuditEventUpdate::new(
                Some(crate::actions::ActionPhase::Preflight),
                true,
                0,
                None,
                None,
                preflight_message,
            ),
        );
        if let Some(timeout) = total_timeout_error(
            started_instant,
            run_policy.max_total_duration,
            ActionPhase::Preflight,
        ) {
            return Err(timeout);
        }

        if dry_run {
            audit_event.action_phase = Some(crate::actions::ActionPhase::DryRun);
            check_action_with_explanation(
                &run_policy.policy,
                &run_policy.context,
                PolicyIntent::DryRun,
                &descriptor,
            )?;
            let state = action.dry_run().map_err(ActionError::dry_run)?;
            append_runner_audit_event(
                audit_path,
                &audit_event,
                RunnerAuditEventUpdate::new(
                    Some(crate::actions::ActionPhase::DryRun),
                    true,
                    state.affected_tasks,
                    None,
                    None,
                    "dry run successful",
                ),
            );
            if let Some(timeout) = total_timeout_error(
                started_instant,
                run_policy.max_total_duration,
                ActionPhase::DryRun,
            ) {
                return Err(timeout);
            }
            audit_event.success = true;
            audit_event.affected_tasks = state.affected_tasks;
            audit_event.action_phase = None;
            audit_event.error_category = None;
            audit_event.message = "dry run successful".to_owned();

            Ok(AuditedActionResult {
                state,
                rollback: None,
            })
        } else {
            audit_event.action_phase = Some(crate::actions::ActionPhase::DryRun);
            check_action_with_explanation(
                &run_policy.policy,
                &run_policy.context,
                PolicyIntent::DryRun,
                &descriptor,
            )?;
            let dry_run_state = action.dry_run().map_err(ActionError::dry_run)?;
            append_runner_audit_event(
                audit_path,
                &audit_event,
                RunnerAuditEventUpdate::new(
                    Some(crate::actions::ActionPhase::DryRun),
                    true,
                    dry_run_state.affected_tasks,
                    None,
                    None,
                    "pre-apply dry run successful",
                ),
            );
            if let Some(timeout) = total_timeout_error(
                started_instant,
                run_policy.max_total_duration,
                ActionPhase::DryRun,
            ) {
                return Err(timeout);
            }
            if let Some(max_affected_tasks) = run_policy.max_affected_tasks
                && dry_run_state.affected_tasks > max_affected_tasks
            {
                return Err(ActionError::scope_limit_exceeded(
                    dry_run_state.affected_tasks,
                    max_affected_tasks,
                ));
            }

            audit_event.action_phase = Some(crate::actions::ActionPhase::Apply);
            check_action_with_explanation(
                &run_policy.policy,
                &run_policy.context,
                PolicyIntent::Apply,
                &descriptor,
            )?;
            let apply_res = action.apply();
            let rollback = match apply_res {
                Ok(token) => token,
                Err(partial_err) => {
                    let source = partial_err.source;
                    if let Some(token) = partial_err.rollback {
                        audit_event.affected_tasks = token.affected_tasks();
                        audit_event.restore_path = token.restore_path().cloned();
                        match action.rollback(&token) {
                            Ok(()) => {
                                append_runner_audit_event(
                                    audit_path,
                                    &audit_event,
                                    RunnerAuditEventUpdate::new(
                                        Some(crate::actions::ActionPhase::Rollback),
                                        true,
                                        token.affected_tasks(),
                                        token.restore_path().cloned(),
                                        None,
                                        "partial rollback attempted after apply failure; partial rollback completed",
                                    ),
                                );
                                return Err(ActionError::apply(format!(
                                    "apply failed: {}; partial rollback attempted and completed successfully",
                                    source
                                )));
                            }
                            Err(rollback_err) => {
                                append_runner_audit_event(
                                    audit_path,
                                    &audit_event,
                                    RunnerAuditEventUpdate::new(
                                        Some(crate::actions::ActionPhase::Rollback),
                                        false,
                                        token.affected_tasks(),
                                        token.restore_path().cloned(),
                                        Some("EmergencyRollbackFailed".to_owned()),
                                        "partial rollback attempted after apply failure; partial rollback failed",
                                    ),
                                );
                                return Err(ActionError::emergency_rollback(
                                    format!("apply failed: {source}"),
                                    rollback_err,
                                ));
                            }
                        }
                    } else {
                        return Err(ActionError::apply(source));
                    }
                }
            };
            audit_event.affected_tasks = rollback.affected_tasks();
            audit_event.restore_path = rollback.restore_path().cloned();

            if let Err(hook_err) = hooks.run_after_apply(&rollback) {
                audit_event.action_phase = Some(crate::actions::ActionPhase::Rollback);
                return Err(rollback_after_apply_hook_failure(
                    action,
                    &rollback,
                    hook_err,
                    audit_path,
                    &audit_event,
                    &mut hooks,
                ));
            }

            append_runner_audit_event(
                audit_path,
                &audit_event,
                RunnerAuditEventUpdate::new(
                    Some(crate::actions::ActionPhase::Apply),
                    true,
                    rollback.affected_tasks(),
                    rollback.restore_path().cloned(),
                    None,
                    "apply successful",
                ),
            );

            if let Some(timeout) = total_timeout_error(
                started_instant,
                run_policy.max_total_duration,
                ActionPhase::Apply,
            ) {
                audit_event.action_phase = Some(crate::actions::ActionPhase::Rollback);
                return Err(rollback_after_timeout(
                    action,
                    &rollback,
                    timeout,
                    audit_path,
                    &audit_event,
                    &mut hooks,
                ));
            }

            audit_event.action_phase = Some(crate::actions::ActionPhase::Verify);
            let state = match action.verify() {
                Ok(state) => {
                    append_runner_audit_event(
                        audit_path,
                        &audit_event,
                        RunnerAuditEventUpdate::new(
                            Some(crate::actions::ActionPhase::Verify),
                            true,
                            state.affected_tasks,
                            rollback.restore_path().cloned(),
                            None,
                            "verify successful",
                        ),
                    );
                    if let Some(timeout) = total_timeout_error(
                        started_instant,
                        run_policy.max_total_duration,
                        ActionPhase::Verify,
                    ) {
                        audit_event.action_phase = Some(crate::actions::ActionPhase::Rollback);
                        return Err(rollback_after_timeout(
                            action,
                            &rollback,
                            timeout,
                            audit_path,
                            &audit_event,
                            &mut hooks,
                        ));
                    }
                    state
                }
                Err(verify_err) => {
                    audit_event.action_phase = Some(crate::actions::ActionPhase::Rollback);
                    match action.rollback(&rollback) {
                        Ok(()) => {
                            if let Err(hook_err) = hooks.run_after_rollback(&rollback) {
                                return Err(ActionError::rollback(format!(
                                    "after-rollback hook failed after verify failure rollback completed: verify error: {verify_err}; hook error: {hook_err:#}"
                                )));
                            }

                            append_runner_audit_event(
                                audit_path,
                                &audit_event,
                                RunnerAuditEventUpdate::new(
                                    Some(crate::actions::ActionPhase::Rollback),
                                    true,
                                    rollback.affected_tasks(),
                                    rollback.restore_path().cloned(),
                                    None,
                                    "rollback completed after verify failure",
                                ),
                            );
                            return Err(ActionError::verify_rollback_completed(verify_err));
                        }
                        Err(rollback_err) => {
                            return Err(ActionError::emergency_rollback(verify_err, rollback_err));
                        }
                    }
                }
            };

            audit_event.affected_tasks = state.affected_tasks;
            audit_event.success = true;
            audit_event.action_phase = None;
            audit_event.error_category = None;
            audit_event.message = "action applied and verified".to_owned();

            Ok(AuditedActionResult {
                state,
                rollback: Some(rollback),
            })
        }
    })();

    if let Err(ref err) = result {
        audit_event.success = false;
        audit_event.action_phase = Some(err.phase());
        audit_event.error_category = Some(err.category().to_owned());
        audit_event.message = err.human_message();
    }

    append_runner_audit_event(
        audit_path,
        &audit_event,
        RunnerAuditEventUpdate::new(
            audit_event.action_phase,
            audit_event.success,
            audit_event.affected_tasks,
            audit_event.restore_path.clone(),
            audit_event.error_category.clone(),
            audit_event.message.clone(),
        ),
    );

    result
}
