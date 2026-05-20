//! Rollback tests extracted from `actions::runner`.
//!
//! Owns verify-failure rollback and emergency rollback failure tests.
//! Does not own policy, audit-only, timeout, hook-failure, or production runner behavior.

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::super::{
        super::*, TestAction, TestActionLog, apply_policy, temp_dir, terminal_event,
    };
    use crate::actions::fake_action::FakeAction;

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
    fn fake_action_partial_apply_failure_rolls_back_applied_prefix() {
        let dir = temp_dir("fake-partial-apply-failure");
        let audit_path = dir.join("audit.jsonl");
        let action = FakeAction::new()
            .with_affected_tasks(2)
            .with_partial_apply_failure();

        let result = run_audited_action_with_audit_path(
            "fake-controller",
            &action,
            apply_policy(),
            &audit_path,
        );

        assert!(result.is_err());
        assert_eq!(
            action.events(),
            vec!["preflight", "dry_run", "apply", "rollback"]
        );
        assert!(!action.applied());
        assert!(action.rolled_back());

        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("fake apply failure after first target"));
        assert!(err.contains("partial rollback attempted"));
        assert!(err.contains("completed successfully"));

        let events = crate::audit::read_audit_tail(&audit_path, 10).unwrap();
        assert_eq!(events.len(), 4);
        assert!(events.iter().any(|event| event.action_phase
            == Some(crate::actions::ActionPhase::Rollback)
            && event.success
            && event.affected_tasks == 1
            && event.message.contains("partial rollback attempted")));
        let terminal = terminal_event(&events);
        assert!(!terminal.success);
        assert_eq!(terminal.affected_tasks, 1);
        assert!(terminal.message.contains("partial rollback attempted"));
        assert!(
            terminal
                .message
                .contains("fake apply failure after first target")
        );

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
