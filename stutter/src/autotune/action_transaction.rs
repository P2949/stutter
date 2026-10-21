//! Durable action-boundary transaction shared by the live controller and crash
//! acceptance. Mutation remains in the audited production action runner.
use std::path::Path;

use crate::{
    actions::{
        TuningAction,
        runner::{
            ActionHooks, ActionRunPolicy, AuditedActionResult,
            run_audited_action_with_audit_path_and_hooks,
        },
    },
    autotune::{
        controller_journal::{
            ControllerJournalActionMetadata, write_controller_journal_applied_with_metadata,
            write_controller_journal_applying_with_metadata, write_controller_journal_clean,
        },
        experiment::ExperimentId,
    },
};

pub(crate) struct JournaledActionInput<'a> {
    pub journal_path: &'a Path,
    pub audit_path: &'a Path,
    pub experiment_id: &'a str,
    pub metadata: ControllerJournalActionMetadata,
}

pub(crate) fn apply_journaled_action<A: TuningAction>(
    action: &A,
    policy: ActionRunPolicy,
    input: JournaledActionInput<'_>,
) -> anyhow::Result<AuditedActionResult> {
    write_controller_journal_applying_with_metadata(
        input.journal_path,
        ExperimentId::try_new(input.experiment_id)?,
        action.id(),
        input.metadata.clone(),
    )?;
    let hooks = ActionHooks::after_apply(|token| {
        write_controller_journal_applied_with_metadata(
            input.journal_path,
            ExperimentId::try_new(input.experiment_id)?,
            action.id(),
            token.clone(),
            input
                .metadata
                .clone()
                .with_verify_result("applied_pending_verify"),
        )
        .map(|_| ())
    })
    .with_after_rollback(|_| write_controller_journal_clean(input.journal_path).map(|_| ()));
    run_audited_action_with_audit_path_and_hooks(
        "autotune candidate",
        action,
        policy,
        input.audit_path,
        hooks,
    )
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        actions::{
            ActionId, ActionState, ActionWarning, RollbackToken, SafetyClass,
            fake_action::FakeAction,
        },
        autotune::controller_journal::{ControllerJournalState, read_controller_journal},
    };
    struct VerifyJournal<'a> {
        inner: FakeAction,
        journal: &'a Path,
        fail_applied_write: bool,
    }
    impl TuningAction for VerifyJournal<'_> {
        fn id(&self) -> ActionId {
            self.inner.id()
        }
        fn describe(&self) -> String {
            self.inner.describe()
        }
        fn safety_class(&self) -> SafetyClass {
            self.inner.safety_class()
        }
        fn descriptor(&self) -> crate::daemon_policy::ActionDescriptor {
            self.inner.descriptor()
        }
        fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
            self.inner.preflight()
        }
        fn dry_run(&self) -> anyhow::Result<ActionState> {
            self.inner.dry_run()
        }
        fn apply(&self) -> crate::actions::ApplyResult {
            let token = self.inner.apply()?;
            let prepare = || -> anyhow::Result<()> {
                // Synthetic restore data: this test proves transaction ordering,
                // not real affinity restoration (covered by live acceptance).
                let path = token.restore_path().expect("fake affinity token");
                let mut file = std::fs::File::create(path)?;
                std::io::Write::write_all(&mut file, b"{\"synthetic\":true}")?;
                file.sync_all()?;
                if self.fail_applied_write {
                    std::fs::remove_file(self.journal)?;
                    std::fs::create_dir(self.journal)?;
                }
                Ok(())
            };
            prepare().map_err(|source| crate::actions::PartialApplyError {
                source,
                rollback: Some(token.clone()),
            })?;
            Ok(token)
        }
        fn verify(&self) -> anyhow::Result<ActionState> {
            let record = read_controller_journal(self.journal)?;
            anyhow::ensure!(
                record.state() == ControllerJournalState::Applied
                    && record.rollback_token().is_some(),
                "durable rollback token must exist before verification; state={:?}",
                record.state()
            );
            anyhow::ensure!(
                record
                    .rollback_token()
                    .and_then(|token| token.restore_path())
                    .is_some_and(|path| path.is_file()),
                "rollback restore file must exist at verification entry"
            );
            self.inner.verify()
        }
        fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
            self.inner.rollback(token)?;
            if self.fail_applied_write {
                std::fs::remove_dir(self.journal)?;
            }
            Ok(())
        }
    }
    #[test]
    fn controller_transaction_persists_rollback_before_verification() {
        let root = crate::test_support::TestRoot::new("controller-token-before-verify");
        let journal = root.join("journal.json");
        let audit = root.join("audit.jsonl");
        let action = VerifyJournal {
            inner: FakeAction::new().with_restore_path(root.join("restore.json")),
            journal: &journal,
            fail_applied_write: false,
        };
        let result = apply_journaled_action(
            &action,
            ActionRunPolicy::apply_low_risk(crate::daemon_policy::ActionSource::Test, false),
            JournaledActionInput {
                journal_path: &journal,
                audit_path: &audit,
                experiment_id: "rollback-before-verify",
                metadata: Default::default(),
            },
        );
        assert!(result.is_ok(), "{result:?}");
        assert!(
            read_controller_journal(&journal)
                .unwrap()
                .rollback_token()
                .is_some()
        );
    }

    #[test]
    fn applied_journal_write_failure_rolls_back_before_verification() {
        let root = crate::test_support::TestRoot::new("journal-write-failure");
        let journal = root.join("journal.json");
        let audit = root.join("audit.jsonl");
        let action = VerifyJournal {
            inner: FakeAction::new().with_restore_path(root.join("restore.json")),
            journal: &journal,
            fail_applied_write: true,
        };
        let error = apply_journaled_action(
            &action,
            ActionRunPolicy::apply_low_risk(crate::daemon_policy::ActionSource::Test, false),
            JournaledActionInput {
                journal_path: &journal,
                audit_path: &audit,
                experiment_id: "write-failure",
                metadata: Default::default(),
            },
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("after-apply hook failed"),
            "{error:#}"
        );
        assert!(action.inner.rolled_back());
        assert!(!action.inner.applied());
        assert!(!action.inner.events().contains(&"verify"));
        assert!(read_controller_journal(&journal).unwrap().is_clean());
        let events = crate::audit::read_audit_tail(&audit, 20).unwrap();
        assert!(events.iter().any(|event| event.action_phase
            == Some(crate::actions::ActionPhase::Rollback)
            && event.success));
        let terminal = events.last().unwrap();
        assert!(!terminal.success);
        assert!(terminal.message.contains("after-apply hook failed"));
    }

    #[test]
    fn verification_failure_cleans_only_after_successful_rollback() {
        for fail_rollback in [false, true] {
            let root = crate::test_support::TestRoot::new("verify-rollback-journal");
            let journal = root.join("journal.json");
            let audit = root.join("audit.jsonl");
            let inner = FakeAction::new()
                .with_restore_path(root.join("restore.json"))
                .with_fail_verify();
            let action = VerifyJournal {
                inner: if fail_rollback {
                    inner.with_fail_rollback()
                } else {
                    inner
                },
                journal: &journal,
                fail_applied_write: false,
            };
            assert!(
                apply_journaled_action(
                    &action,
                    ActionRunPolicy::apply_low_risk(
                        crate::daemon_policy::ActionSource::Test,
                        false
                    ),
                    JournaledActionInput {
                        journal_path: &journal,
                        audit_path: &audit,
                        experiment_id: "verify-failure",
                        metadata: Default::default()
                    },
                )
                .is_err()
            );
            let record = read_controller_journal(&journal).unwrap();
            assert_eq!(record.is_clean(), !fail_rollback);
            assert_eq!(record.rollback_token().is_some(), fail_rollback);
            assert_eq!(action.inner.rolled_back(), !fail_rollback);
            assert_eq!(action.inner.applied(), fail_rollback);
            let events = crate::audit::read_audit_tail(&audit, 20).unwrap();
            let terminal = events.last().unwrap();
            assert!(!terminal.success);
            assert_eq!(
                terminal.error_category.as_deref(),
                Some(if fail_rollback {
                    "emergency_rollback_failure"
                } else {
                    "verify_failure_rollback_completed"
                })
            );
        }
    }
}
