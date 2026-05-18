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
    let ActionError::Timeout {
        phase,
        elapsed_ms,
        timeout_ms,
    } = timeout_error
    else {
        return timeout_error;
    };

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
        action_id: Some(action_id.0.clone()),
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
            let rollback = action.apply().map_err(ActionError::apply)?;
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
mod tests {
    use std::{
        cell::{Cell, RefCell},
        fs,
        path::{Path, PathBuf},
    };

    use super::*;
    use crate::actions::{
        ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass, TuningAction,
        fake_action::{FakeAction, FakeActionSwitches},
    };

    #[derive(Default)]
    struct TestActionLog {
        events: RefCell<Vec<&'static str>>,
        mutated: Cell<bool>,
        rolled_back: Cell<bool>,
    }

    struct TestAction<'a> {
        should_fail_preflight: bool,
        should_fail_apply: bool,
        should_fail_verify: bool,
        should_fail_rollback: bool,
        affected_tasks: usize,
        log: &'a TestActionLog,
    }

    impl<'a> TestAction<'a> {
        fn new(log: &'a TestActionLog) -> Self {
            Self {
                should_fail_preflight: false,
                should_fail_apply: false,
                should_fail_verify: false,
                should_fail_rollback: false,
                affected_tasks: 5,
                log,
            }
        }

        fn with_preflight_failure(mut self) -> Self {
            self.should_fail_preflight = true;
            self
        }

        fn with_apply_failure(mut self) -> Self {
            self.should_fail_apply = true;
            self
        }

        fn with_verify_failure(mut self) -> Self {
            self.should_fail_verify = true;
            self
        }

        fn with_rollback_failure(mut self) -> Self {
            self.should_fail_rollback = true;
            self
        }

        fn with_affected_tasks(mut self, affected_tasks: usize) -> Self {
            self.affected_tasks = affected_tasks;
            self
        }
    }

    impl TuningAction for TestAction<'_> {
        fn id(&self) -> ActionId {
            ActionId("test-action".to_owned())
        }

        fn describe(&self) -> String {
            "test action".to_owned()
        }

        fn safety_class(&self) -> SafetyClass {
            SafetyClass::ReversibleLowRisk
        }

        fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
            self.log.events.borrow_mut().push("preflight");
            if self.should_fail_preflight {
                anyhow::bail!("preflight intentional failure");
            }
            Ok(vec![ActionWarning {
                message: "test preflight warning".to_owned(),
            }])
        }

        fn dry_run(&self) -> anyhow::Result<ActionState> {
            self.log.events.borrow_mut().push("dry_run");
            Ok(ActionState {
                applied: false,
                affected_tasks: self.affected_tasks,
                checked_tasks: self.affected_tasks,
                pending_changes: self.affected_tasks,
                warnings: vec![],
            })
        }

        fn apply(&self) -> anyhow::Result<RollbackToken> {
            self.log.events.borrow_mut().push("apply");
            if self.should_fail_apply {
                anyhow::bail!("apply intentional failure");
            }
            self.log.mutated.set(true);
            Ok(RollbackToken::CpuAffinityRestoreFile {
                path: PathBuf::from("/tmp/restore"),
                affected_tasks: self.affected_tasks,
            })
        }

        fn verify(&self) -> anyhow::Result<ActionState> {
            self.log.events.borrow_mut().push("verify");
            if self.should_fail_verify {
                anyhow::bail!("verify intentional failure");
            }
            Ok(ActionState {
                applied: true,
                affected_tasks: self.affected_tasks,
                checked_tasks: self.affected_tasks,
                pending_changes: 0,
                warnings: vec![],
            })
        }

        fn rollback(&self, _token: &RollbackToken) -> anyhow::Result<()> {
            self.log.events.borrow_mut().push("rollback");
            if self.should_fail_rollback {
                anyhow::bail!("rollback intentional failure");
            }
            self.log.rolled_back.set(true);
            self.log.mutated.set(false);
            Ok(())
        }
    }

    fn apply_policy() -> ActionRunPolicy {
        ActionRunPolicy::apply_low_risk(ActionSource::Test, false)
    }

    fn dry_run_policy() -> ActionRunPolicy {
        ActionRunPolicy::dry_run(ActionSource::Test)
    }

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-actions-runner-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn all_capabilities_available() -> crate::daemon::capabilities::DaemonCapabilities {
        crate::daemon::capabilities::DaemonCapabilities {
            kernel_release: Some("6.9.1-test".to_owned()),
            btf_available: true,
            sched_tracepoints_available: true,
            perf_permissions_likely: true,
            perf_event_paranoid: Some(1),
            cgroup_v2_available: true,
            sched_ext_available: true,
            uclamp_available: true,
            ionice_available: true,
            irq_affinity_available: true,
            gpu_sysfs_available: true,
        }
    }

    fn terminal_event(events: &[crate::audit::AuditEvent]) -> &crate::audit::AuditEvent {
        events.last().expect("expected at least one audit event")
    }

    fn action_phases(
        events: &[crate::audit::AuditEvent],
    ) -> Vec<Option<crate::actions::ActionPhase>> {
        events.iter().map(|event| event.action_phase).collect()
    }

    #[test]
    fn after_apply_hook_runs_before_verify() {
        let dir = temp_dir("after-apply-hook-order");
        let audit_path = dir.join("audit.jsonl");
        let log = TestActionLog::default();
        let action = TestAction::new(&log).with_affected_tasks(5);
        let mut hook_called = false;

        let result = run_audited_action_with_audit_path_and_hooks(
            "test-cmd",
            &action,
            apply_policy(),
            &audit_path,
            ActionHooks::after_apply(|rollback| {
                assert_eq!(rollback.affected_tasks(), 5);
                log.events.borrow_mut().push("after_apply_hook");
                hook_called = true;
                Ok(())
            }),
        )
        .unwrap();

        assert!(hook_called);
        assert_eq!(result.state.affected_tasks, 5);
        assert_eq!(
            *log.events.borrow(),
            vec![
                "preflight",
                "dry_run",
                "apply",
                "after_apply_hook",
                "verify"
            ]
        );
        assert!(log.mutated.get());
        assert!(!log.rolled_back.get());

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn after_apply_hook_failure_rolls_back_and_writes_rollback_audit_event() {
        let dir = temp_dir("after-apply-hook-failure-rollback-audit");
        let audit_path = dir.join("audit.jsonl");
        let log = TestActionLog::default();
        let action = TestAction::new(&log).with_affected_tasks(5);

        let result = run_audited_action_with_audit_path_and_hooks(
            "test-cmd",
            &action,
            apply_policy(),
            &audit_path,
            ActionHooks::after_apply(|_rollback| {
                anyhow::bail!("intentional after-apply hook failure");
            }),
        );

        assert!(result.is_err());
        assert_eq!(
            *log.events.borrow(),
            vec!["preflight", "dry_run", "apply", "rollback"]
        );
        assert!(!log.mutated.get());
        assert!(log.rolled_back.get());

        let err = result.unwrap_err().to_string();
        assert!(err.contains("after-apply hook failed"));
        assert!(err.contains("rollback completed"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert!(
            events
                .iter()
                .any(|event| event.action_phase == Some(ActionPhase::Rollback)
                    && event.success
                    && event.affected_tasks == 5
                    && event.message == "rollback completed after after-apply hook failure")
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn after_apply_hook_failure_records_failed_rollback_hook_audit_event() {
        let dir = temp_dir("after-apply-hook-failure-rollback-hook-audit");
        let audit_path = dir.join("audit.jsonl");
        let log = TestActionLog::default();
        let action = TestAction::new(&log).with_affected_tasks(5);

        let result = run_audited_action_with_audit_path_and_hooks(
            "test-cmd",
            &action,
            apply_policy(),
            &audit_path,
            ActionHooks::after_apply(|_rollback| {
                anyhow::bail!("intentional after-apply hook failure");
            })
            .with_after_rollback(|_rollback| {
                anyhow::bail!("intentional after-rollback hook failure");
            }),
        );

        assert!(result.is_err());
        assert_eq!(
            *log.events.borrow(),
            vec!["preflight", "dry_run", "apply", "rollback"]
        );
        assert!(!log.mutated.get());
        assert!(log.rolled_back.get());

        let err = result.unwrap_err().to_string();
        assert!(err.contains("after-rollback hook failed"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert!(events.iter().any(|event| {
            event.action_phase == Some(ActionPhase::Rollback)
                && !event.success
                && event.affected_tasks == 5
                && event.error_category.as_deref() == Some("RollbackHookFailed")
                && event
                    .message
                    .contains("rollback completed after after-apply hook failure")
        }));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn after_rollback_hook_runs_after_verify_failure_rollback() {
        let dir = temp_dir("after-rollback-hook-order");
        let audit_path = dir.join("audit.jsonl");
        let log = TestActionLog::default();
        let action = TestAction::new(&log).with_verify_failure();
        let mut after_apply_called = false;
        let mut after_rollback_called = false;

        let result = run_audited_action_with_audit_path_and_hooks(
            "test-cmd",
            &action,
            apply_policy(),
            &audit_path,
            ActionHooks::after_apply(|rollback| {
                assert_eq!(rollback.affected_tasks(), 5);
                log.events.borrow_mut().push("after_apply_hook");
                after_apply_called = true;
                Ok(())
            })
            .with_after_rollback(|rollback| {
                assert_eq!(rollback.affected_tasks(), 5);
                log.events.borrow_mut().push("after_rollback_hook");
                after_rollback_called = true;
                Ok(())
            }),
        );

        assert!(result.is_err());
        assert!(after_apply_called);
        assert!(after_rollback_called);
        assert_eq!(
            *log.events.borrow(),
            vec![
                "preflight",
                "dry_run",
                "apply",
                "after_apply_hook",
                "verify",
                "rollback",
                "after_rollback_hook"
            ]
        );
        assert!(!log.mutated.get());
        assert!(log.rolled_back.get());

        let err = result.unwrap_err().to_string();
        assert!(err.contains("verify failed"));
        assert!(err.contains("rollback completed"));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn after_rollback_hook_runs_after_apply_timeout_rollback() {
        let dir = temp_dir("after-rollback-hook-apply-timeout");
        let audit_path = dir.join("audit.jsonl");
        let action = FakeAction::new()
            .with_slow_apply()
            .with_slow_apply_duration(Duration::from_millis(25));
        let mut after_apply_called = false;
        let mut after_rollback_called = false;

        let result = run_audited_action_with_audit_path_and_hooks(
            "fake-controller",
            &action,
            apply_policy().with_max_total_duration(Duration::from_millis(1)),
            &audit_path,
            ActionHooks::after_apply(|rollback| {
                assert_eq!(rollback.affected_tasks(), 5);
                after_apply_called = true;
                Ok(())
            })
            .with_after_rollback(|rollback| {
                assert_eq!(rollback.affected_tasks(), 5);
                after_rollback_called = true;
                Ok(())
            }),
        );

        assert!(result.is_err());
        assert!(after_apply_called);
        assert!(after_rollback_called);
        assert_eq!(
            action.events(),
            vec!["preflight", "dry_run", "apply", "slow_apply", "rollback"]
        );
        assert!(!action.applied());
        assert!(action.rolled_back());

        let err = result.unwrap_err();
        assert!(matches!(err, ActionError::TimeoutRollbackCompleted { .. }));
        assert!(err.to_string().contains("rollback completed"));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn action_run_policy_constructors_resolve_policies_through_builder() {
        let config = crate::daemon::DaemonConfig {
            mode: DaemonMode::ApplyLowRisk,
            source: ActionSource::Test,
            ..crate::daemon::DaemonConfig::default()
        };
        let expected = build_daemon_policy(DaemonPolicyBuildInput {
            config: &config,
            remote_context: None,
        });
        let run_policy = ActionRunPolicy::apply_low_risk(ActionSource::Test, false);

        assert_eq!(run_policy.policy, expected);
        assert!(!run_policy.dry_run);
        assert_eq!(run_policy.max_affected_tasks, None);
    }

    #[test]
    fn runner_policy_rejection_happens_before_apply() {
        let dir = temp_dir("policy-rejection");
        let audit_path = dir.join("audit.jsonl");
        let action = FakeAction::new().with_safety_class(SafetyClass::ReversibleMediumRisk);

        let result = run_audited_action_with_audit_path(
            "fake-controller",
            &action,
            ActionRunPolicy::apply_low_risk(ActionSource::Test, false),
            &audit_path,
        );

        assert!(result.is_err());
        assert_eq!(action.events(), vec!["preflight", "dry_run"]);
        assert!(!action.applied());
        assert!(!action.rolled_back());

        let err = result.unwrap_err();
        assert!(matches!(err, ActionError::PolicyRejected { .. }));
        assert!(err.to_string().contains("policy rejected"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(
            action_phases(&events),
            vec![
                Some(crate::actions::ActionPhase::Preflight),
                Some(crate::actions::ActionPhase::DryRun),
                Some(crate::actions::ActionPhase::Preflight),
            ]
        );
        let terminal = terminal_event(&events);
        assert!(!terminal.success);
        assert_eq!(terminal.error_category.as_deref(), Some("policy_rejected"));
        assert!(terminal.message.contains("policy rejected"));
        assert!(
            terminal
                .message
                .contains("safety class ReversibleMediumRisk")
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn runner_context_capability_rejection_happens_before_apply() {
        let dir = temp_dir("capability-rejection");
        let audit_path = dir.join("audit.jsonl");
        let action = FakeAction::new().with_action_id("uclamp:min");
        let mut capabilities = all_capabilities_available();
        capabilities.uclamp_available = false;

        let result = run_audited_action_with_audit_path(
            "fake-controller",
            &action,
            apply_policy().with_capabilities(capabilities),
            &audit_path,
        );

        assert!(result.is_err());
        assert_eq!(action.events(), vec!["preflight"]);
        assert!(!action.applied());
        assert!(!action.rolled_back());

        let err = result.unwrap_err();
        assert!(matches!(err, ActionError::PolicyRejected { .. }));
        assert!(err.to_string().contains("uclamp"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 2);
        let terminal = terminal_event(&events);
        assert!(!terminal.success);
        assert_eq!(terminal.error_category.as_deref(), Some("policy_rejected"));
        assert!(terminal.message.contains("uclamp"));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn runner_scope_limit_rejection_happens_after_dry_run_before_apply() {
        let dir = temp_dir("scope-limit");
        let audit_path = dir.join("audit.jsonl");
        let action = FakeAction::new().with_affected_tasks(8);

        let result = run_audited_action_with_audit_path(
            "fake-controller",
            &action,
            apply_policy().with_max_affected_tasks(3),
            &audit_path,
        );

        assert!(result.is_err());
        assert_eq!(action.events(), vec!["preflight", "dry_run"]);
        assert!(!action.applied());
        assert!(!action.rolled_back());

        let err = result.unwrap_err();
        assert!(matches!(err, ActionError::ScopeLimitExceeded { .. }));
        assert!(err.to_string().contains("exceeding scope limit 3"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 3);
        let terminal = terminal_event(&events);
        assert!(!terminal.success);
        assert_eq!(
            terminal.error_category.as_deref(),
            Some("scope_limit_exceeded")
        );
        assert_eq!(
            terminal.action_phase,
            Some(crate::actions::ActionPhase::DryRun)
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn fake_action_preflight_failure_blocks_apply_and_verify() {
        let dir = temp_dir("fake-preflight-failure");
        let audit_path = dir.join("audit.jsonl");
        let action = FakeAction::new().with_fail_preflight();

        let result = run_audited_action_with_audit_path(
            "fake-controller",
            &action,
            apply_policy(),
            &audit_path,
        );

        assert!(result.is_err());
        assert_eq!(action.events(), vec!["preflight"]);
        assert!(!action.applied());
        assert!(!action.rolled_back());

        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("preflight failed"));
        assert!(err.contains("fake preflight failure"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 1);
        let terminal = terminal_event(&events);
        assert!(!terminal.success);
        assert_eq!(terminal.command, "fake-controller");
        assert!(terminal.message.contains("preflight failed"));
        assert!(terminal.message.contains("fake preflight failure"));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn fake_action_apply_failure_blocks_verify_and_rollback() {
        let dir = temp_dir("fake-apply-failure");
        let audit_path = dir.join("audit.jsonl");
        let action = FakeAction::new().with_fail_apply();

        let result = run_audited_action_with_audit_path(
            "fake-controller",
            &action,
            apply_policy(),
            &audit_path,
        );

        assert!(result.is_err());
        assert_eq!(action.events(), vec!["preflight", "dry_run", "apply"]);
        assert!(!action.applied());
        assert!(!action.rolled_back());

        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("apply failed"));
        assert!(err.contains("fake apply failure"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 3);
        let terminal = terminal_event(&events);
        assert!(!terminal.success);
        assert!(terminal.message.contains("apply failed"));
        assert!(terminal.message.contains("fake apply failure"));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn fake_action_verify_failure_rolls_back_mutation() {
        let dir = temp_dir("fake-verify-failure");
        let audit_path = dir.join("audit.jsonl");
        let action = FakeAction::new().with_fail_verify();

        let result = run_audited_action_with_audit_path(
            "fake-controller",
            &action,
            apply_policy(),
            &audit_path,
        );

        assert!(result.is_err());
        assert_eq!(
            action.events(),
            vec!["preflight", "dry_run", "apply", "verify", "rollback"]
        );
        assert!(!action.applied());
        assert!(action.rolled_back());

        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("verify failed"));
        assert!(err.contains("rollback completed"));
        assert!(err.contains("fake verify failure"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 5);
        assert!(events.iter().any(|event| event.action_phase
            == Some(crate::actions::ActionPhase::Rollback)
            && event.success));
        let terminal = terminal_event(&events);
        assert!(!terminal.success);
        assert_eq!(terminal.affected_tasks, 5);
        assert_eq!(
            terminal.restore_path,
            Some(PathBuf::from("/tmp/stutter-fake-action-restore.json"))
        );
        assert!(terminal.message.contains("verify failed"));
        assert!(terminal.message.contains("rollback completed"));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn fake_action_rollback_failure_keeps_mutation_and_reports_emergency() {
        let dir = temp_dir("fake-rollback-failure");
        let audit_path = dir.join("audit.jsonl");
        let action = FakeAction::new().with_fail_verify().with_fail_rollback();

        let result = run_audited_action_with_audit_path(
            "fake-controller",
            &action,
            apply_policy(),
            &audit_path,
        );

        assert!(result.is_err());
        assert_eq!(
            action.events(),
            vec!["preflight", "dry_run", "apply", "verify", "rollback"]
        );
        assert!(action.applied());
        assert!(!action.rolled_back());

        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("emergency rollback failed"));
        assert!(err.contains("fake verify failure"));
        assert!(err.contains("fake rollback failure"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 4);
        let terminal = terminal_event(&events);
        assert!(!terminal.success);
        assert_eq!(terminal.affected_tasks, 5);
        assert!(terminal.message.contains("emergency rollback failed"));
        assert!(terminal.message.contains("fake verify failure"));
        assert!(terminal.message.contains("fake rollback failure"));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn fake_action_slow_apply_still_verifies_and_returns_rollback_token() {
        let dir = temp_dir("fake-slow-apply");
        let audit_path = dir.join("audit.jsonl");
        let action = FakeAction::new()
            .with_switches(FakeActionSwitches {
                slow_apply: true,
                ..FakeActionSwitches::default()
            })
            .with_slow_apply_duration(std::time::Duration::from_millis(25))
            .with_affected_tasks(9);
        let started = std::time::Instant::now();

        let result = run_audited_action_with_audit_path(
            "fake-controller",
            &action,
            apply_policy(),
            &audit_path,
        )
        .unwrap();

        assert!(
            started.elapsed() >= std::time::Duration::from_millis(20),
            "slow_apply switch did not delay apply path long enough"
        );
        assert_eq!(
            action.events(),
            vec!["preflight", "dry_run", "apply", "slow_apply", "verify"]
        );
        assert!(action.applied());
        assert!(!action.rolled_back());
        assert_eq!(result.state.affected_tasks, 9);
        assert_eq!(
            result.rollback,
            Some(RollbackToken::CpuAffinityRestoreFile {
                path: PathBuf::from("/tmp/stutter-fake-action-restore.json"),
                affected_tasks: 9,
            })
        );

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(
            action_phases(&events),
            vec![
                Some(crate::actions::ActionPhase::Preflight),
                Some(crate::actions::ActionPhase::DryRun),
                Some(crate::actions::ActionPhase::Apply),
                Some(crate::actions::ActionPhase::Verify),
                None,
            ]
        );
        let terminal = terminal_event(&events);
        assert!(terminal.success);
        assert_eq!(terminal.affected_tasks, 9);
        assert_eq!(terminal.message, "action applied and verified");

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn runner_timeout_after_apply_rolls_back_mutation() {
        let dir = temp_dir("timeout-after-apply");
        let audit_path = dir.join("audit.jsonl");
        let action = FakeAction::new()
            .with_slow_apply()
            .with_slow_apply_duration(Duration::from_millis(25));

        let result = run_audited_action_with_audit_path(
            "fake-controller",
            &action,
            apply_policy().with_max_total_duration(Duration::from_millis(1)),
            &audit_path,
        );

        assert!(result.is_err());
        assert_eq!(
            action.events(),
            vec!["preflight", "dry_run", "apply", "slow_apply", "rollback"]
        );
        assert!(!action.applied());
        assert!(action.rolled_back());

        let err = result.unwrap_err();
        assert!(matches!(err, ActionError::TimeoutRollbackCompleted { .. }));
        assert!(err.to_string().contains("rollback completed"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert!(events.iter().any(|event| event.action_phase
            == Some(crate::actions::ActionPhase::Rollback)
            && event.success
            && event.message.contains("action timeout")));
        let terminal = terminal_event(&events);
        assert!(!terminal.success);
        assert_eq!(
            terminal.error_category.as_deref(),
            Some("timeout_rollback_completed")
        );
        assert_eq!(
            terminal.action_phase,
            Some(crate::actions::ActionPhase::Apply)
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn fake_action_dry_run_never_applies() {
        let dir = temp_dir("fake-dry-run");
        let audit_path = dir.join("audit.jsonl");
        let action = FakeAction::new().with_affected_tasks(7);

        let result = run_audited_action_with_audit_path(
            "fake-controller",
            &action,
            dry_run_policy(),
            &audit_path,
        )
        .unwrap();

        assert_eq!(action.events(), vec!["preflight", "dry_run"]);
        assert!(!action.applied());
        assert!(!action.rolled_back());
        assert_eq!(result.state.affected_tasks, 7);
        assert_eq!(result.rollback, None);

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 3);
        let terminal = terminal_event(&events);
        assert!(terminal.success);
        assert!(terminal.dry_run);
        assert_eq!(terminal.affected_tasks, 7);
        assert_eq!(terminal.message, "dry run successful");

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn runner_dry_run_in_observe_never_calls_apply() {
        let dir = temp_dir("observe-dry-run");
        let audit_path = dir.join("audit.jsonl");
        let action = FakeAction::new().with_affected_tasks(11);
        let run_policy = ActionRunPolicy {
            policy: DaemonPolicy::observe(ActionSource::Test),
            context: DaemonPolicyContext::default(),
            max_affected_tasks: None,
            max_total_duration: None,
            dry_run: true,
        };

        let result =
            run_audited_action_with_audit_path("fake-controller", &action, run_policy, &audit_path)
                .unwrap();

        assert_eq!(action.events(), vec!["preflight", "dry_run"]);
        assert!(!action.applied());
        assert!(!action.rolled_back());
        assert_eq!(result.state.affected_tasks, 11);
        assert_eq!(result.rollback, None);

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 3);
        let terminal = terminal_event(&events);
        assert!(terminal.success);
        assert!(terminal.dry_run);
        assert_eq!(terminal.message, "dry run successful");

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn action_runner_signature_prevents_apply_without_policy() {
        let _runner: fn(
            &str,
            &FakeAction,
            ActionRunPolicy,
        ) -> Result<AuditedActionResult, ActionError> = run_audited_action::<FakeAction>;
        let _runner_with_audit_path: fn(
            &str,
            &FakeAction,
            ActionRunPolicy,
            &Path,
        ) -> Result<AuditedActionResult, ActionError> =
            run_audited_action_with_audit_path::<FakeAction>;
    }

    #[test]
    fn preflight_failure_logs_audit_failure() {
        let dir = temp_dir("failed-preflight");
        let audit_path = dir.join("audit.jsonl");
        let log = TestActionLog::default();
        let action = TestAction::new(&log).with_preflight_failure();

        let result =
            run_audited_action_with_audit_path("test-cmd", &action, apply_policy(), &audit_path);
        assert!(result.is_err());

        assert_eq!(*log.events.borrow(), vec!["preflight"]);
        assert!(!log.mutated.get());
        assert!(!log.rolled_back.get());

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 1);
        let terminal = terminal_event(&events);
        assert!(!terminal.success);
        assert!(!terminal.dry_run);
        assert!(terminal.message.contains("preflight failed"));
        assert!(terminal.message.contains("preflight intentional failure"));
        assert_eq!(
            terminal.action_phase,
            Some(crate::actions::ActionPhase::Preflight)
        );
        assert_eq!(
            terminal.error_category.as_deref(),
            Some("preflight_failure")
        );
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn dry_run_does_not_mutate_system() {
        let dir = temp_dir("success-dry-run");
        let audit_path = dir.join("audit.jsonl");
        let log = TestActionLog::default();
        let action = TestAction::new(&log).with_affected_tasks(5);

        let result =
            run_audited_action_with_audit_path("test-cmd", &action, dry_run_policy(), &audit_path)
                .unwrap();

        assert_eq!(result.state.affected_tasks, 5);
        assert!(result.rollback.is_none());
        assert_eq!(*log.events.borrow(), vec!["preflight", "dry_run"]);
        assert!(!log.mutated.get());
        assert!(!log.rolled_back.get());

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 3);
        let terminal = terminal_event(&events);
        assert!(terminal.success);
        assert!(terminal.dry_run);
        assert_eq!(terminal.affected_tasks, 5);
        assert_eq!(terminal.message, "dry run successful");
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn apply_failure_writes_failure_audit() {
        let dir = temp_dir("failed-apply");
        let audit_path = dir.join("audit.jsonl");
        let log = TestActionLog::default();
        let action = TestAction::new(&log).with_apply_failure();

        let result =
            run_audited_action_with_audit_path("test-cmd", &action, apply_policy(), &audit_path);
        assert!(result.is_err());

        assert_eq!(*log.events.borrow(), vec!["preflight", "dry_run", "apply"]);
        assert!(!log.mutated.get());
        assert!(!log.rolled_back.get());

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 3);
        let terminal = terminal_event(&events);
        assert!(!terminal.success);
        assert!(!terminal.dry_run);
        assert!(terminal.message.contains("apply failed"));
        assert!(terminal.message.contains("apply intentional failure"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn verify_failure_triggers_rollback_in_audited_runner() {
        let dir = temp_dir("verify-failure-rollback");
        let audit_path = dir.join("audit.jsonl");
        let log = TestActionLog::default();
        let action = TestAction::new(&log).with_verify_failure();

        let result =
            run_audited_action_with_audit_path("test-cmd", &action, apply_policy(), &audit_path);
        assert!(result.is_err());

        assert_eq!(
            *log.events.borrow(),
            vec!["preflight", "dry_run", "apply", "verify", "rollback"]
        );
        assert!(!log.mutated.get());
        assert!(log.rolled_back.get());

        let err = result.unwrap_err().to_string();
        assert!(err.contains("verify failed"));
        assert!(err.contains("rollback completed"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 5);
        let terminal = terminal_event(&events);
        assert!(!terminal.success);
        assert!(!terminal.dry_run);
        assert_eq!(terminal.affected_tasks, 5);
        assert_eq!(terminal.restore_path, Some(PathBuf::from("/tmp/restore")));
        assert!(terminal.message.contains("verify failed"));
        assert!(terminal.message.contains("rollback completed"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn rollback_failure_produces_emergency_status() {
        let dir = temp_dir("rollback-failure-emergency");
        let audit_path = dir.join("audit.jsonl");
        let log = TestActionLog::default();
        let action = TestAction::new(&log)
            .with_verify_failure()
            .with_rollback_failure();

        let result =
            run_audited_action_with_audit_path("test-cmd", &action, apply_policy(), &audit_path);
        assert!(result.is_err());

        assert_eq!(
            *log.events.borrow(),
            vec!["preflight", "dry_run", "apply", "verify", "rollback"]
        );
        assert!(log.mutated.get());
        assert!(!log.rolled_back.get());

        let err = result.unwrap_err().to_string();
        assert!(err.contains("emergency rollback failed"));
        assert!(err.contains("verify intentional failure"));
        assert!(err.contains("rollback intentional failure"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 4);
        let terminal = terminal_event(&events);
        assert!(!terminal.success);
        assert!(!terminal.dry_run);
        assert_eq!(terminal.affected_tasks, 5);
        assert_eq!(terminal.restore_path, Some(PathBuf::from("/tmp/restore")));
        assert!(terminal.message.contains("emergency rollback failed"));
        assert!(terminal.message.contains("verify intentional failure"));
        assert!(terminal.message.contains("rollback intentional failure"));
        assert_eq!(
            terminal.action_phase,
            Some(crate::actions::ActionPhase::EmergencyRollback)
        );
        assert_eq!(
            terminal.error_category.as_deref(),
            Some("emergency_rollback_failure")
        );
        fs::remove_dir_all(dir).ok();
    }
}
