//! Rollback handler for `CgroupControlRestore` tokens.
use super::control_transaction::{ControlRecord, RealFiles, RestoreOutcome, restore_recorded};
use crate::actions::{
    ActionBoundaryError, RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult,
    RollbackToken,
};

#[derive(Default)]
pub(crate) struct CgroupControlRollbackHandler;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ControlRestoreCheck {
    pub restored: usize,
    pub already: usize,
    pub foreign: usize,
    pub replaced: usize,
    pub unproven: usize,
    pub messages: Vec<String>,
}

pub(crate) fn restore_all(records: &[ControlRecord]) -> ControlRestoreCheck {
    let mut check = ControlRestoreCheck::default();
    for record in records {
        let (file, _) = record.control.file_and_controller();
        match restore_recorded(record, &RealFiles) {
            RestoreOutcome::Restored => check.restored += 1,
            RestoreOutcome::AlreadyRestored => check.already += 1,
            RestoreOutcome::ExternallyChanged => {
                check.foreign += 1;
                check.messages.push(format!(
                    "{}/{file} holds another owner's value: left untouched",
                    record.dir.display()
                ));
            }
            RestoreOutcome::UnsafeToRestore => {
                check.replaced += 1;
                check.messages.push(format!(
                    "{} is no longer the recorded group: left untouched",
                    record.dir.display()
                ));
            }
            RestoreOutcome::Unproven => {
                check.unproven += 1;
                check.messages.push(format!(
                    "could not prove {}/{file} was restored",
                    record.dir.display()
                ));
            }
        }
    }
    check
}

impl RollbackHandler for CgroupControlRollbackHandler {
    fn id(&self) -> &'static str {
        "cgroup-control-rollback"
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        Ok(Vec::new())
    }

    fn dry_run(&self, _: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        Err(ActionBoundaryError::missing_explicit_rollback_token(
            self.id(),
            "cgroup-control-restore",
        )
        .into())
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        Err(ActionBoundaryError::missing_explicit_rollback_token(
            self.id(),
            "cgroup-control-restore",
        )
        .into())
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        matches!(token, RollbackToken::CgroupControlRestore { .. })
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        anyhow::ensure!(
            self.supports_token(token),
            "unsupported token {}",
            token.kind()
        );
        Ok(RollbackPreview {
            handler_id: self.id(),
            restore_path: Default::default(),
            affected_tasks: token.affected_tasks(),
            message: "would restore each recorded cgroup control while it is still ours".into(),
        })
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        let RollbackToken::CgroupControlRestore { records } = token else {
            return Err(ActionBoundaryError::unsupported_rollback_token(
                self.id(),
                "cgroup-control-restore",
                token.kind(),
            )
            .into());
        };
        let check = restore_all(records);
        Ok(RollbackResult {
            handler_id: self.id(),
            restore_path: Default::default(),
            restored: check.restored + check.already,
            skipped_dead: 0,
            skipped_identity_mismatch: check.replaced,
            skipped_externally_changed: check.foreign,
            legacy_unverified: 0,
            errors: check.unproven,
            messages: check.messages,
        })
    }
}
