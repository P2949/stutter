//! Rollback handler for `BlockSchedulerRestore` tokens.
use std::path::Path;

use super::{
    ActionBoundaryError, RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult,
    RollbackToken,
    block_scheduler::{
        BlockSchedulerRecord, BlockSchedulerRestore, RealBlockFiles, restore_recorded,
    },
};

pub const SYS_BLOCK: &str = "/sys/block";

#[derive(Default)]
pub(crate) struct BlockSchedulerRollbackHandler;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct SchedulerRestoreCheck {
    pub restored: usize,
    pub already: usize,
    pub foreign: usize,
    pub replaced: usize,
    pub unproven: usize,
    pub messages: Vec<String>,
}

pub(crate) fn restore_all(records: &[BlockSchedulerRecord]) -> SchedulerRestoreCheck {
    let mut check = SchedulerRestoreCheck::default();
    for record in records {
        match restore_recorded(Path::new(SYS_BLOCK), record, &RealBlockFiles) {
            BlockSchedulerRestore::Restored => check.restored += 1,
            BlockSchedulerRestore::AlreadyRestored => check.already += 1,
            BlockSchedulerRestore::ExternallyChanged => {
                check.foreign += 1;
                check.messages.push(format!(
                    "{} uses another owner's scheduler: left untouched",
                    record.queue.display()
                ));
            }
            BlockSchedulerRestore::UnsafeToRestore => {
                check.replaced += 1;
                check.messages.push(format!(
                    "{} is no longer the recorded device: left untouched",
                    record.queue.display()
                ));
            }
            BlockSchedulerRestore::Unproven => {
                check.unproven += 1;
                check.messages.push(format!(
                    "could not prove {} was restored",
                    record.queue.display()
                ));
            }
        }
    }
    check
}

impl RollbackHandler for BlockSchedulerRollbackHandler {
    fn id(&self) -> &'static str {
        "block-scheduler-rollback"
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        Ok(Vec::new())
    }

    fn dry_run(&self, _: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        Err(ActionBoundaryError::missing_explicit_rollback_token(
            self.id(),
            "block-scheduler-restore",
        )
        .into())
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        Err(ActionBoundaryError::missing_explicit_rollback_token(
            self.id(),
            "block-scheduler-restore",
        )
        .into())
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        matches!(token, RollbackToken::BlockSchedulerRestore { .. })
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
            message:
                "would restore each recorded device's original scheduler while it is still ours"
                    .into(),
        })
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        let RollbackToken::BlockSchedulerRestore { records } = token else {
            return Err(ActionBoundaryError::unsupported_rollback_token(
                self.id(),
                "block-scheduler-restore",
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
