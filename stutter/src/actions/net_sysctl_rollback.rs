//! Rollback handler for `NetSysctlRestore` tokens.
use std::path::Path;

use super::{
    ActionBoundaryError, RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult,
    RollbackToken,
    net_sysctl::{NetRestore, NetSysctlRecord, restore_recorded},
};

#[derive(Default)]
pub(crate) struct NetSysctlRollbackHandler;

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct NetRestoreCheck {
    pub restored: usize,
    pub already: usize,
    pub foreign: usize,
    pub replaced: usize,
    pub unproven: usize,
    pub messages: Vec<String>,
}

pub(crate) fn restore_all(records: &[NetSysctlRecord]) -> NetRestoreCheck {
    let mut check = NetRestoreCheck::default();
    for record in records {
        match restore_recorded(Path::new("/proc"), record) {
            NetRestore::Restored => check.restored += 1,
            NetRestore::AlreadyRestored => check.already += 1,
            NetRestore::ExternallyChanged => {
                check.foreign += 1;
                check.messages.push(format!(
                    "{} holds another owner's value: left untouched",
                    record.path.display()
                ));
            }
            NetRestore::UnsafeToRestore => {
                check.replaced += 1;
                check.messages.push(format!(
                    "{} is not in the recorded network namespace: left untouched",
                    record.path.display()
                ));
            }
            NetRestore::Unproven => {
                check.unproven += 1;
                check.messages.push(format!(
                    "could not prove {} was restored",
                    record.path.display()
                ));
            }
        }
    }
    check
}

impl RollbackHandler for NetSysctlRollbackHandler {
    fn id(&self) -> &'static str {
        "net-sysctl-rollback"
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        Ok(Vec::new())
    }

    fn dry_run(&self, _: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        Err(
            ActionBoundaryError::missing_explicit_rollback_token(self.id(), "net-sysctl-restore")
                .into(),
        )
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        Err(
            ActionBoundaryError::missing_explicit_rollback_token(self.id(), "net-sysctl-restore")
                .into(),
        )
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        matches!(token, RollbackToken::NetSysctlRestore { .. })
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
                "would restore each recorded sysctl in its network namespace while it is still ours"
                    .into(),
        })
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        let RollbackToken::NetSysctlRestore { records } = token else {
            return Err(ActionBoundaryError::unsupported_rollback_token(
                self.id(),
                "net-sysctl-restore",
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
