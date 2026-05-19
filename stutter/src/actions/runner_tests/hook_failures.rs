//! Hook tests extracted from `actions::runner`.
//!
//! Owns after-apply and after-rollback hook ordering/failure tests.
//! Does not own policy, audit-log, timeout, rollback, or production runner behavior.

#[cfg(test)]
mod tests {
    use std::fs;

    use super::super::{super::*, TestAction, TestActionLog, apply_policy, temp_dir};

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
}
