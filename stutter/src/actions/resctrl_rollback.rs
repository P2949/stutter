//! Rollback handler for `ResctrlGroupRemove` tokens: removes exactly the recorded kernel group.
use super::{
    ActionBoundaryError, RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult,
    RollbackToken,
    resctrl_alloc::{CatRestore, ResctrlGroupRecord, remove_recorded},
};

#[derive(Default)]
pub(crate) struct ResctrlRollbackHandler;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct RemovalCheck {
    pub removed: usize,
    pub absent: usize,
    pub replaced: usize,
    pub unproven: usize,
    pub messages: Vec<String>,
}

pub(crate) fn remove_all(records: &[ResctrlGroupRecord]) -> RemovalCheck {
    let mut check = RemovalCheck::default();
    for record in records {
        match remove_recorded(record) {
            CatRestore::Removed => check.removed += 1,
            CatRestore::Absent => check.absent += 1,
            CatRestore::UnsafeToRestore => {
                check.replaced += 1;
                check.messages.push(format!(
                    "{} now names a different group: left untouched",
                    record.group.display()
                ));
            }
            CatRestore::Unproven => {
                check.unproven += 1;
                check
                    .messages
                    .push(format!("could not remove {}", record.group.display()));
            }
        }
    }
    check
}

impl RollbackHandler for ResctrlRollbackHandler {
    fn id(&self) -> &'static str {
        "resctrl-rollback"
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        Ok(Vec::new())
    }

    fn dry_run(&self, _: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        Err(
            ActionBoundaryError::missing_explicit_rollback_token(self.id(), "resctrl-group-remove")
                .into(),
        )
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        Err(
            ActionBoundaryError::missing_explicit_rollback_token(self.id(), "resctrl-group-remove")
                .into(),
        )
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        matches!(token, RollbackToken::ResctrlGroupRemove { .. })
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
            message: "would remove the recorded resctrl groups while they are still the same kernel objects".into(),
        })
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        let RollbackToken::ResctrlGroupRemove { records } = token else {
            return Err(ActionBoundaryError::unsupported_rollback_token(
                self.id(),
                "resctrl-group-remove",
                token.kind(),
            )
            .into());
        };
        let check = remove_all(records);
        Ok(RollbackResult {
            handler_id: self.id(),
            restore_path: Default::default(),
            restored: check.removed + check.absent,
            skipped_dead: 0,
            skipped_identity_mismatch: check.replaced,
            skipped_externally_changed: 0,
            legacy_unverified: 0,
            errors: check.unproven,
            messages: check.messages,
        })
    }
}
