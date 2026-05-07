use std::path::Path;

use anyhow::Context;

use crate::{
    actions::{ActionOutcome, ActionState, RollbackToken, TuningAction},
    audit::{AuditEvent, append_audit_event_to_path, unix_nanos_now},
};

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

            let state = action.verify().context("verify failed")?;
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
    use std::{fs, path::PathBuf};

    use super::*;
    use crate::actions::{
        ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass, TuningAction,
    };

    struct TestAction {
        should_fail_preflight: bool,
        should_fail_apply: bool,
        affected_tasks: usize,
    }

    impl TuningAction for TestAction {
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
            if self.should_fail_preflight {
                anyhow::bail!("preflight intentional failure");
            }
            Ok(vec![ActionWarning {
                message: "test preflight warning".to_owned(),
            }])
        }
        fn dry_run(&self) -> anyhow::Result<ActionState> {
            Ok(ActionState {
                applied: false,
                affected_tasks: self.affected_tasks,
                checked_tasks: self.affected_tasks,
                pending_changes: self.affected_tasks,
                warnings: vec![],
            })
        }
        fn apply(&self) -> anyhow::Result<RollbackToken> {
            if self.should_fail_apply {
                anyhow::bail!("apply intentional failure");
            }
            Ok(RollbackToken::CpuAffinityRestoreFile {
                path: PathBuf::from("/tmp/restore"),
                affected_tasks: self.affected_tasks,
            })
        }
        fn verify(&self) -> anyhow::Result<ActionState> {
            Ok(ActionState {
                applied: true,
                affected_tasks: self.affected_tasks,
                checked_tasks: self.affected_tasks,
                pending_changes: 0,
                warnings: vec![],
            })
        }
        fn rollback(&self, _token: &RollbackToken) -> anyhow::Result<()> {
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
    fn audited_action_logs_failed_preflight() {
        let dir = temp_dir("failed-preflight");
        let audit_path = dir.join("audit.jsonl");
        let action = TestAction {
            should_fail_preflight: true,
            should_fail_apply: false,
            affected_tasks: 10,
        };

        let result = run_audited_action_with_audit_path("test-cmd", &action, false, &audit_path);
        assert!(result.is_err());

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert!(!events[0].success);
        assert!(events[0].message.contains("preflight failed"));
        assert!(events[0].message.contains("preflight intentional failure"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn audited_action_logs_failed_apply() {
        let dir = temp_dir("failed-apply");
        let audit_path = dir.join("audit.jsonl");
        let action = TestAction {
            should_fail_preflight: false,
            should_fail_apply: true,
            affected_tasks: 10,
        };

        let result = run_audited_action_with_audit_path("test-cmd", &action, false, &audit_path);
        assert!(result.is_err());

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert!(!events[0].success);
        assert!(events[0].message.contains("apply failed"));
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn audited_action_logs_successful_dry_run() {
        let dir = temp_dir("success-dry-run");
        let audit_path = dir.join("audit.jsonl");
        let action = TestAction {
            should_fail_preflight: false,
            should_fail_apply: false,
            affected_tasks: 5,
        };

        let result =
            run_audited_action_with_audit_path("test-cmd", &action, true, &audit_path).unwrap();
        assert_eq!(result.state.affected_tasks, 5);
        assert!(result.rollback.is_none());
        assert_eq!(result.outcome.action_id, ActionId("test-action".to_owned()));
        assert_eq!(result.outcome.safety_class, SafetyClass::ReversibleLowRisk);
        assert!(result.outcome.dry_run);
        assert_eq!(result.outcome.state.affected_tasks, 5);
        assert!(result.outcome.rollback.is_none());
        assert_eq!(result.outcome.preflight_warnings.len(), 1);
        assert_eq!(
            result.outcome.preflight_warnings[0].message,
            "test preflight warning"
        );
        assert!(result.outcome.finished_unix_nanos >= result.outcome.started_unix_nanos);

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].success);
        assert!(events[0].dry_run);
        assert_eq!(events[0].affected_tasks, 5);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn audited_action_returns_successful_apply_outcome() {
        let dir = temp_dir("success-apply");
        let audit_path = dir.join("audit.jsonl");
        let action = TestAction {
            should_fail_preflight: false,
            should_fail_apply: false,
            affected_tasks: 7,
        };

        let result =
            run_audited_action_with_audit_path("test-cmd", &action, false, &audit_path).unwrap();

        assert_eq!(result.state.affected_tasks, 7);
        assert!(result.rollback.is_some());
        assert_eq!(result.outcome.action_id, ActionId("test-action".to_owned()));
        assert_eq!(result.outcome.safety_class, SafetyClass::ReversibleLowRisk);
        assert!(!result.outcome.dry_run);
        assert_eq!(result.outcome.state.affected_tasks, 7);
        assert!(result.outcome.rollback.is_some());
        assert_eq!(result.outcome.preflight_warnings.len(), 1);
        assert_eq!(
            result.outcome.preflight_warnings[0].message,
            "test preflight warning"
        );
        assert!(result.outcome.finished_unix_nanos >= result.outcome.started_unix_nanos);

        let outcome_json = serde_json::to_string(&result.outcome).unwrap();
        let parsed_outcome: ActionOutcome = serde_json::from_str(&outcome_json).unwrap();
        assert_eq!(parsed_outcome.action_id, ActionId("test-action".to_owned()));
        assert_eq!(parsed_outcome.state.affected_tasks, 7);
        assert!(parsed_outcome.rollback.is_some());

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].success);
        assert!(!events[0].dry_run);
        assert_eq!(events[0].affected_tasks, 7);
        assert_eq!(events[0].restore_path, Some(PathBuf::from("/tmp/restore")));
        fs::remove_dir_all(dir).ok();
    }
}
