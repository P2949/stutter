use std::path::Path;

use anyhow::Context;

use crate::{
    actions::{ActionOutcome, ActionState, RollbackToken, TuningAction},
    audit::{AuditEvent, append_audit_event_to_path, unix_nanos_now},
};

#[derive(Debug)]
pub struct AuditedActionResult {
    pub state: ActionState,
    pub rollback: Option<RollbackToken>,
    pub outcome: ActionOutcome,
}

pub fn run_audited_action<A: TuningAction>(
    command: &str,
    action: &A,
    dry_run: bool,
) -> anyhow::Result<AuditedActionResult> {
    run_audited_action_with_audit_path(
        command,
        action,
        dry_run,
        &crate::audit::default_audit_log_path(),
    )
}

pub fn run_audited_action_with_audit_path<A: TuningAction>(
    command: &str,
    action: &A,
    dry_run: bool,
    audit_path: &Path,
) -> anyhow::Result<AuditedActionResult> {
    let started_unix_nanos = unix_nanos_now();
    let action_id = action.id();
    let safety_class = action.safety_class();

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
        message: String::new(),
    };

    let result = (|| -> anyhow::Result<AuditedActionResult> {
        let preflight_warnings = action.preflight().context("preflight failed")?;

        if dry_run {
            let state = action.dry_run().context("dry run failed")?;
            audit_event.success = true;
            audit_event.affected_tasks = state.affected_tasks;
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
            let rollback = action.apply().context("apply failed")?;
            audit_event.affected_tasks = rollback.affected_tasks();
            audit_event.restore_path = rollback.restore_path().cloned();

            let state = match action.verify() {
                Ok(state) => state,
                Err(verify_err) => {
                    match action.rollback(&rollback) {
                        Ok(()) => {
                            anyhow::bail!("verify failed; rollback completed: {verify_err:#}");
                        }
                        Err(rollback_err) => {
                            anyhow::bail!(
                                "verify failed; emergency rollback failed: verify error: {verify_err:#}; rollback error: {rollback_err:#}"
                            );
                        }
                    }
                }
            };

            audit_event.affected_tasks = state.affected_tasks;
            audit_event.success = true;
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
        audit_event.message = format!("{err:#}");
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
        path::PathBuf,
    };

    use super::*;
    use crate::actions::{
        ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass, TuningAction,
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
    fn preflight_failure_logs_audit_failure() {
        let dir = temp_dir("failed-preflight");
        let audit_path = dir.join("audit.jsonl");
        let log = TestActionLog::default();
        let action = TestAction::new(&log).with_preflight_failure();

        let result = run_audited_action_with_audit_path("test-cmd", &action, false, &audit_path);
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
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn dry_run_does_not_mutate_system() {
        let dir = temp_dir("success-dry-run");
        let audit_path = dir.join("audit.jsonl");
        let log = TestActionLog::default();
        let action = TestAction::new(&log).with_affected_tasks(5);

        let result =
            run_audited_action_with_audit_path("test-cmd", &action, true, &audit_path).unwrap();

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

        let result = run_audited_action_with_audit_path("test-cmd", &action, false, &audit_path);
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

        let result = run_audited_action_with_audit_path("test-cmd", &action, false, &audit_path);
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

        let result = run_audited_action_with_audit_path("test-cmd", &action, false, &audit_path);
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
        fs::remove_dir_all(dir).ok();
    }
}
