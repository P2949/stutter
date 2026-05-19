//! Audit-path tests extracted from `actions::runner`.
//!
//! Owns audit event coverage for preflight, dry-run, apply failure, and observe dry-run paths.
//! Does not own policy, timeout, rollback, hook-failure, or production runner behavior.

#[cfg(test)]
mod tests {
    use std::fs;

    use super::super::{
        super::*, TestAction, TestActionLog, apply_policy, dry_run_policy, temp_dir, terminal_event,
    };
    use crate::actions::fake_action::FakeAction;

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
}
