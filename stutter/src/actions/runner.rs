use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crate::{
    actions::{ActionError, ActionPhase, ActionState, RollbackToken, SafetyClass, TuningAction},
    audit::{AuditEvent, append_audit_event_to_path, unix_nanos_now},
    daemon::DaemonConfig,
    daemon_policy::{
        ActionDescriptor, ActionSource, DaemonMode, DaemonPolicy, DaemonPolicyBuildInput,
        DaemonPolicyContext, PolicyDecisionKind, PolicyIntent, build_daemon_policy,
    },
};

#[derive(Debug)]
pub struct AuditedActionResult {
    pub state: ActionState,
    pub rollback: Option<RollbackToken>,
}

type ActionHookResult = anyhow::Result<()>;
type AfterApplyHook<'a> = dyn FnMut(&RollbackToken) -> ActionHookResult + 'a;
type RollbackHook<'a> = dyn FnMut(&RollbackToken) -> ActionHookResult + 'a;

pub(crate) struct ActionHooks<'a> {
    after_apply: Option<Box<AfterApplyHook<'a>>>,
    after_rollback: Option<Box<RollbackHook<'a>>>,
}

impl<'a> ActionHooks<'a> {
    pub(crate) fn none() -> Self {
        Self {
            after_apply: None,
            after_rollback: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn after_apply<F>(after_apply: F) -> Self
    where
        F: FnMut(&RollbackToken) -> ActionHookResult + 'a,
    {
        Self {
            after_apply: Some(Box::new(after_apply)),
            after_rollback: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_after_rollback<F>(mut self, after_rollback: F) -> Self
    where
        F: FnMut(&RollbackToken) -> ActionHookResult + 'a,
    {
        self.after_rollback = Some(Box::new(after_rollback));
        self
    }

    fn run_after_apply(&mut self, rollback: &RollbackToken) -> ActionHookResult {
        if let Some(after_apply) = self.after_apply.as_mut() {
            after_apply(rollback)?;
        }

        Ok(())
    }

    fn run_after_rollback(&mut self, rollback: &RollbackToken) -> ActionHookResult {
        if let Some(after_rollback) = self.after_rollback.as_mut() {
            after_rollback(rollback)?;
        }

        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct ActionRunPolicy {
    pub policy: DaemonPolicy,
    pub context: DaemonPolicyContext,
    pub max_affected_tasks: Option<usize>,
    pub max_total_duration: Option<Duration>,
    pub dry_run: bool,
}

impl ActionRunPolicy {
    pub fn dry_run(source: ActionSource) -> Self {
        Self::for_mode(DaemonMode::Suggest, source, true)
    }

    pub fn apply_low_risk(source: ActionSource, dry_run: bool) -> Self {
        Self::for_mode(DaemonMode::ApplyLowRisk, source, dry_run)
    }

    pub fn apply_medium_risk(source: ActionSource, dry_run: bool) -> Self {
        Self::for_mode(DaemonMode::ApplyMediumRisk, source, dry_run)
    }

    pub fn for_action<A: TuningAction>(action: &A, dry_run: bool, source: ActionSource) -> Self {
        if dry_run {
            return Self::dry_run(source);
        }

        match action.safety_class() {
            SafetyClass::ObserveOnly | SafetyClass::ReversibleLowRisk => {
                Self::apply_low_risk(source, false)
            }
            SafetyClass::ReversibleMediumRisk | SafetyClass::HighRisk => {
                Self::apply_medium_risk(source, false)
            }
        }
    }

    fn for_mode(mode: DaemonMode, source: ActionSource, dry_run: bool) -> Self {
        let config = DaemonConfig {
            mode,
            source,
            ..DaemonConfig::default()
        };

        Self {
            policy: build_daemon_policy(DaemonPolicyBuildInput {
                config: &config,
                remote_context: None,
            }),
            context: DaemonPolicyContext::default(),
            max_affected_tasks: None,
            max_total_duration: None,
            dry_run,
        }
    }

    #[cfg(test)]
    pub fn with_capabilities(
        mut self,
        capabilities: crate::daemon::capabilities::DaemonCapabilities,
    ) -> Self {
        self.context.capabilities = Some(capabilities);
        self
    }

    #[cfg(test)]
    pub fn with_max_affected_tasks(mut self, max_affected_tasks: usize) -> Self {
        self.max_affected_tasks = Some(max_affected_tasks);
        self
    }

    #[cfg(test)]
    pub fn with_max_total_duration(mut self, max_total_duration: Duration) -> Self {
        self.max_total_duration = Some(max_total_duration);
        self
    }
}

fn check_action_with_explanation(
    policy: &DaemonPolicy,
    context: &DaemonPolicyContext,
    intent: PolicyIntent,
    descriptor: &ActionDescriptor,
) -> Result<(), ActionError> {
    let explanation = policy.explain_action_with_context(intent, descriptor, context);
    match explanation.decision {
        PolicyDecisionKind::Allowed => Ok(()),
        PolicyDecisionKind::Rejected { .. } => {
            Err(ActionError::policy_rejected(explanation.final_reason))
        }
    }
}

struct RunnerAuditEventUpdate {
    phase: Option<ActionPhase>,
    success: bool,
    affected_tasks: usize,
    restore_path: Option<PathBuf>,
    error_category: Option<String>,
    message: String,
}

impl RunnerAuditEventUpdate {
    fn new(
        phase: Option<ActionPhase>,
        success: bool,
        affected_tasks: usize,
        restore_path: Option<PathBuf>,
        error_category: Option<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            phase,
            success,
            affected_tasks,
            restore_path,
            error_category,
            message: message.into(),
        }
    }
}

fn append_runner_audit_event(
    audit_path: &Path,
    base_event: &AuditEvent,
    update: RunnerAuditEventUpdate,
) {
    let mut event = base_event.clone();
    event.unix_nanos = unix_nanos_now();
    event.action_phase = update.phase;
    event.success = update.success;
    event.affected_tasks = update.affected_tasks;
    event.restore_path = update.restore_path;
    event.error_category = update.error_category;
    event.message = update.message;

    if let Err(audit_err) = append_audit_event_to_path(audit_path, &event) {
        log::warn!(
            "failed to write audit event to {}: {audit_err:#}",
            audit_path.display()
        );
    }
}

fn total_timeout_error(
    started: Instant,
    max_total_duration: Option<Duration>,
    phase: ActionPhase,
) -> Option<ActionError> {
    let max_total_duration = max_total_duration?;
    let elapsed = started.elapsed();

    (elapsed > max_total_duration)
        .then(|| ActionError::timeout(phase, elapsed.as_millis(), max_total_duration.as_millis()))
}

fn rollback_after_timeout<A>(
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

fn rollback_after_apply_hook_failure<A>(
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
        let preflight_warnings = action.preflight().map_err(ActionError::preflight)?;
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
                                        "partial rollback completed after apply failure",
                                    ),
                                );
                                return Err(ActionError::apply(format!(
                                    "apply failed: {}; partial rollback completed successfully",
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
                                        "partial rollback failed after apply failure",
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

#[cfg(test)]
#[path = "runner_tests/mod.rs"]
mod tests;
