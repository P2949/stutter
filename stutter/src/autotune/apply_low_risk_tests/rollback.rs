//! Rollback and recovery tests extracted from `autotune::apply_low_risk`.
//!
//! Owns rollback guard, apply-then-rollback lifecycle, and startup recovery tests.
//! Does not own policy gates, target resolution, experiment resolution, audit/journal hook tests, or production behavior.

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf, time::Duration};

    use super::super::*;
    use crate::{
        actions::{RollbackToken, SafetyClass},
        autotune::controller_journal::{
            ControllerJournalActionMetadata, write_controller_journal_applied_with_metadata,
        },
    };

    #[test]
    fn audited_rollback_guard_rolls_back_explicitly() {
        let action = TestAction {
            id: "rollback-guard",
            safety_class: SafetyClass::ReversibleLowRisk,
            should_fail_apply: false,
            should_fail_verify: false,
            affected_tasks: 7,
        };
        let token = RollbackToken::CpuAffinityRestoreFile {
            path: PathBuf::from("/tmp/stutter-test-restore.json"),
            affected_tasks: 7,
        };

        let mut guard = AuditedRollbackGuard::new(&action, token);
        assert!(!guard.rollback_performed());
        guard.rollback_now().unwrap();
        assert!(guard.rollback_performed());
    }

    #[tokio::test]
    async fn apply_low_risk_applies_one_action_and_rolls_back() {
        let mut executor = FakeExecutor::low_risk();

        let outcome = run_apply_low_risk_with_executor(&mut executor, Duration::ZERO)
            .await
            .unwrap();

        assert_eq!(outcome.candidate_name, "game-main");
        assert_eq!(outcome.action_kind, "cpu_affinity_profile");
        assert_eq!(outcome.affected_tasks, 31);
        assert_eq!(outcome.safety_class, SafetyClass::ReversibleLowRisk);
        assert!(outcome.rollback_performed);
        assert_eq!(executor.dry_run_calls, 1);
        assert_eq!(executor.apply_calls, 1);
        assert_eq!(executor.rollback_calls, 1);
    }

    #[test]
    fn startup_recovery_recovers_applied_journal_written_before_crash() {
        struct RecoveryExecutor {
            calls: usize,
            affected_tasks: usize,
        }

        impl crate::autotune::startup_recovery::StartupRecoveryRollbackExecutor for RecoveryExecutor {
            fn rollback(
                &mut self,
                _token: &RollbackToken,
            ) -> anyhow::Result<crate::autotune::startup_recovery::StartupRecoveryRollbackSummary>
            {
                self.calls += 1;
                Ok(
                    crate::autotune::startup_recovery::StartupRecoveryRollbackSummary {
                        affected_tasks: self.affected_tasks,
                        message: format!("fake restored={}", self.affected_tasks),
                    },
                )
            }
        }

        let dir = temp_dir("applied-journal-before-crash-recovery");
        let journal_path = dir.join("controller_journal.json");
        let recovery_audit_path = dir.join("recovery-audit.jsonl");
        let history_path = dir.join("history.jsonl");
        let state_snapshot_path = dir.join("daemon_state.json");
        let experiment_id = "apply-low-risk:game-main";
        let action_id = "cpu-affinity-profile:game-main";
        let rollback = RollbackToken::CpuAffinityRestoreFile {
            path: PathBuf::from("/tmp/stutter-test-restore.json"),
            affected_tasks: 31,
        };

        write_controller_journal_applied_with_metadata(
            &journal_path,
            experiment_id,
            action_id,
            rollback,
            ControllerJournalActionMetadata::default()
                .with_candidate("game-main")
                .with_target_identity("pid:1234:starttime:unknown:active_tasks:31")
                .with_restore_command("stutter autotune restore")
                .with_verify_result("applied_pending_verify")
                .with_mode(crate::daemon_policy::DaemonMode::ApplyLowRisk)
                .with_safety_class(SafetyClass::ReversibleLowRisk),
        )
        .unwrap();

        let record =
            crate::autotune::controller_journal::read_controller_journal(&journal_path).unwrap();
        assert_eq!(
            record.state(),
            crate::autotune::controller_journal::ControllerJournalState::Applied
        );
        assert_eq!(
            record.rollback_token().map(RollbackToken::affected_tasks),
            Some(31)
        );

        let config = crate::autotune::startup_recovery::StartupRecoveryConfig {
            rollback_on_crash_recovery: true,
            journal_path: journal_path.clone(),
            audit_path: recovery_audit_path,
            history_path,
            state_snapshot_path,
        };
        let mut recovery_executor = RecoveryExecutor {
            calls: 0,
            affected_tasks: 31,
        };

        let outcome = crate::autotune::startup_recovery::recover_controller_journal_with_executor(
            config.clone(),
            &mut recovery_executor,
        )
        .unwrap();

        match outcome {
            crate::autotune::startup_recovery::StartupRecoveryOutcome::Recovered {
                experiment_id,
                action_id,
                affected_tasks,
                manual_restore_command,
            } => {
                assert_eq!(experiment_id, "apply-low-risk:game-main");
                assert_eq!(action_id, "cpu-affinity-profile:game-main");
                assert_eq!(affected_tasks, 31);
                assert!(manual_restore_command.ends_with("stutter restore"));
            }
            other => panic!("expected recovered startup recovery outcome, got {other:?}"),
        }

        assert_eq!(recovery_executor.calls, 1);
        assert!(
            crate::autotune::controller_journal::read_controller_journal(&journal_path)
                .unwrap()
                .is_clean()
        );

        fs::remove_dir_all(dir).ok();
    }
}
