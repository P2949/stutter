use std::path::Path;

use crate::{
    actions::{ActionError, ActionOutcome, ActionState, RollbackToken, SafetyClass, TuningAction},
    audit::{AuditEvent, append_audit_event_to_path, unix_nanos_now},
    daemon::DaemonConfig,
    daemon_policy::{
        ActionDescriptor, ActionSource, DaemonMode, DaemonPolicy, DaemonPolicyBuildInput,
        PolicyDecisionKind, PolicyIntent, build_daemon_policy,
    },
};

#[derive(Debug)]
pub struct AuditedActionResult {
    pub state: ActionState,
    pub rollback: Option<RollbackToken>,
    pub outcome: ActionOutcome,
}

#[derive(Clone, Debug)]
pub struct ActionRunPolicy {
    pub policy: DaemonPolicy,
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
            dry_run,
        }
    }

    fn policy_intent(&self) -> PolicyIntent {
        if self.dry_run {
            PolicyIntent::DryRun
        } else {
            PolicyIntent::Apply
        }
    }
}

fn check_action_with_explanation(
    policy: &DaemonPolicy,
    intent: PolicyIntent,
    descriptor: &ActionDescriptor,
) -> Result<(), ActionError> {
    let explanation = policy.explain_action(intent, descriptor);
    match explanation.decision {
        PolicyDecisionKind::Allowed => Ok(()),
        PolicyDecisionKind::Rejected { .. } => {
            Err(ActionError::policy_rejected(explanation.final_reason))
        }
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

pub fn run_audited_action_with_audit_path<A>(
    command: &str,
    action: &A,
    run_policy: ActionRunPolicy,
    audit_path: &Path,
) -> Result<AuditedActionResult, ActionError>
where
    A: TuningAction,
{
    let dry_run = run_policy.dry_run;
    let started_unix_nanos = unix_nanos_now();
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

        if dry_run {
            audit_event.action_phase = Some(crate::actions::ActionPhase::DryRun);
            check_action_with_explanation(&run_policy.policy, PolicyIntent::DryRun, &descriptor)?;
            let state = action.dry_run().map_err(ActionError::dry_run)?;
            audit_event.success = true;
            audit_event.affected_tasks = state.affected_tasks;
            audit_event.action_phase = None;
            audit_event.error_category = None;
            audit_event.message = "dry run successful".to_owned();

            let finished_unix_nanos = unix_nanos_now();
            let outcome = ActionOutcome {
                action_id: action_id.clone(),
                safety_class: safety_class.clone(),
                dry_run,
                preflight_warnings,
                state: state.clone(),
                rollback: None,
                started_unix_nanos,
                finished_unix_nanos,
            };

            Ok(AuditedActionResult {
                state,
                rollback: None,
                outcome,
            })
        } else {
            audit_event.action_phase = Some(crate::actions::ActionPhase::Apply);
            check_action_with_explanation(&run_policy.policy, PolicyIntent::Apply, &descriptor)?;
            let rollback = action.apply().map_err(ActionError::apply)?;
            audit_event.affected_tasks = rollback.affected_tasks();
            audit_event.restore_path = rollback.restore_path().cloned();

            audit_event.action_phase = Some(crate::actions::ActionPhase::Verify);
            let state = match action.verify() {
                Ok(state) => state,
                Err(verify_err) => {
                    audit_event.action_phase = Some(crate::actions::ActionPhase::Rollback);
                    match action.rollback(&rollback) {
                        Ok(()) => {
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

            let finished_unix_nanos = unix_nanos_now();
            let outcome = ActionOutcome {
                action_id: action_id.clone(),
                safety_class: safety_class.clone(),
                dry_run,
                preflight_warnings,
                state: state.clone(),
                rollback: Some(rollback.clone()),
                started_unix_nanos,
                finished_unix_nanos,
            };

            Ok(AuditedActionResult {
                state,
                rollback: Some(rollback),
                outcome,
            })
        }
    })();

    if let Err(ref err) = result {
        audit_event.success = false;
        audit_event.action_phase = Some(err.phase());
        audit_event.error_category = Some(err.category().to_owned());
        audit_event.message = err.human_message();
    }

    if let Err(audit_err) = append_audit_event_to_path(audit_path, &audit_event) {
        log::warn!(
            "failed to write audit event to {}: {audit_err:#}",
            audit_path.display()
        );
    }

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
        assert_eq!(action.events(), vec!["preflight"]);
        assert!(!action.applied());
        assert!(!action.rolled_back());

        let err = result.unwrap_err();
        assert!(matches!(err, ActionError::PolicyRejected { .. }));
        assert!(err.to_string().contains("policy rejected"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert!(!events[0].success);
        assert_eq!(events[0].error_category.as_deref(), Some("policy_rejected"));
        assert!(events[0].message.contains("policy rejected"));
        assert!(
            events[0]
                .message
                .contains("safety class ReversibleMediumRisk")
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
        assert!(!events[0].success);
        assert_eq!(events[0].command, "fake-controller");
        assert!(events[0].message.contains("preflight failed"));
        assert!(events[0].message.contains("fake preflight failure"));

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
        assert_eq!(action.events(), vec!["preflight", "apply"]);
        assert!(!action.applied());
        assert!(!action.rolled_back());

        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("apply failed"));
        assert!(err.contains("fake apply failure"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert!(!events[0].success);
        assert!(events[0].message.contains("apply failed"));
        assert!(events[0].message.contains("fake apply failure"));

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
            vec!["preflight", "apply", "verify", "rollback"]
        );
        assert!(!action.applied());
        assert!(action.rolled_back());

        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("verify failed"));
        assert!(err.contains("rollback completed"));
        assert!(err.contains("fake verify failure"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert!(!events[0].success);
        assert_eq!(events[0].affected_tasks, 5);
        assert_eq!(
            events[0].restore_path,
            Some(PathBuf::from("/tmp/stutter-fake-action-restore.json"))
        );
        assert!(events[0].message.contains("verify failed"));
        assert!(events[0].message.contains("rollback completed"));

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
            vec!["preflight", "apply", "verify", "rollback"]
        );
        assert!(action.applied());
        assert!(!action.rolled_back());

        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("emergency rollback failed"));
        assert!(err.contains("fake verify failure"));
        assert!(err.contains("fake rollback failure"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert!(!events[0].success);
        assert_eq!(events[0].affected_tasks, 5);
        assert!(events[0].message.contains("emergency rollback failed"));
        assert!(events[0].message.contains("fake verify failure"));
        assert!(events[0].message.contains("fake rollback failure"));

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
            vec!["preflight", "apply", "slow_apply", "verify"]
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
        assert_eq!(events.len(), 1);
        assert!(events[0].success);
        assert_eq!(events[0].affected_tasks, 9);
        assert_eq!(events[0].message, "action applied and verified");

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
        assert_eq!(events.len(), 1);
        assert!(events[0].success);
        assert!(events[0].dry_run);
        assert_eq!(events[0].affected_tasks, 7);
        assert_eq!(events[0].message, "dry run successful");

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn runner_dry_run_in_observe_never_calls_apply() {
        let dir = temp_dir("observe-dry-run");
        let audit_path = dir.join("audit.jsonl");
        let action = FakeAction::new().with_affected_tasks(11);
        let run_policy = ActionRunPolicy {
            policy: DaemonPolicy::observe(ActionSource::Test),
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
        assert_eq!(events.len(), 1);
        assert!(events[0].success);
        assert!(events[0].dry_run);
        assert_eq!(events[0].message, "dry run successful");

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
        assert!(!events[0].success);
        assert!(!events[0].dry_run);
        assert!(events[0].message.contains("preflight failed"));
        assert!(events[0].message.contains("preflight intentional failure"));
        assert_eq!(
            events[0].action_phase,
            Some(crate::actions::ActionPhase::Preflight)
        );
        assert_eq!(
            events[0].error_category.as_deref(),
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
        assert_eq!(events.len(), 1);
        assert!(events[0].success);
        assert!(events[0].dry_run);
        assert_eq!(events[0].affected_tasks, 5);
        assert_eq!(events[0].message, "dry run successful");
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

        assert_eq!(*log.events.borrow(), vec!["preflight", "apply"]);
        assert!(!log.mutated.get());
        assert!(!log.rolled_back.get());

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert!(!events[0].success);
        assert!(!events[0].dry_run);
        assert!(events[0].message.contains("apply failed"));
        assert!(events[0].message.contains("apply intentional failure"));
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
            vec!["preflight", "apply", "verify", "rollback"]
        );
        assert!(!log.mutated.get());
        assert!(log.rolled_back.get());

        let err = result.unwrap_err().to_string();
        assert!(err.contains("verify failed"));
        assert!(err.contains("rollback completed"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert!(!events[0].success);
        assert!(!events[0].dry_run);
        assert_eq!(events[0].affected_tasks, 5);
        assert_eq!(events[0].restore_path, Some(PathBuf::from("/tmp/restore")));
        assert!(events[0].message.contains("verify failed"));
        assert!(events[0].message.contains("rollback completed"));
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
            vec!["preflight", "apply", "verify", "rollback"]
        );
        assert!(log.mutated.get());
        assert!(!log.rolled_back.get());

        let err = result.unwrap_err().to_string();
        assert!(err.contains("emergency rollback failed"));
        assert!(err.contains("verify intentional failure"));
        assert!(err.contains("rollback intentional failure"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert!(!events[0].success);
        assert!(!events[0].dry_run);
        assert_eq!(events[0].affected_tasks, 5);
        assert_eq!(events[0].restore_path, Some(PathBuf::from("/tmp/restore")));
        assert!(events[0].message.contains("emergency rollback failed"));
        assert!(events[0].message.contains("verify intentional failure"));
        assert!(events[0].message.contains("rollback intentional failure"));
        assert_eq!(
            events[0].action_phase,
            Some(crate::actions::ActionPhase::EmergencyRollback)
        );
        assert_eq!(
            events[0].error_category.as_deref(),
            Some("emergency_rollback_failure")
        );
        fs::remove_dir_all(dir).ok();
    }
}
