//! Timeout tests extracted from `actions::runner`.
//!
//! Owns slow-apply and total-timeout rollback tests.
//! Does not own policy, audit-log, generic rollback, hook-failure, or production runner behavior.

#[cfg(test)]
mod tests {
    use std::{fs, time::Duration};

    use super::super::{super::*, action_phases, apply_policy, temp_dir, terminal_event};
    use crate::actions::{
        ActionFailure, RollbackOutcome,
        fake_action::{FakeAction, FakeActionSwitches},
    };

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
        assert!(matches!(
            err.failure(),
            ActionFailure::Rollback(RollbackOutcome::TimeoutRollbackCompleted { .. })
        ));
        assert!(err.to_string().contains("rollback completed"));

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
        assert!(matches!(
            err.failure(),
            ActionFailure::Rollback(RollbackOutcome::TimeoutRollbackCompleted { .. })
        ));
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
}
