use super::{executors::*, manual_command::*, types::*};
use crate::actions::{
    RollbackCandidate, RollbackHandler, RollbackPreview, RollbackRegistry, RollbackResult,
    RollbackToken,
};

#[derive(Default)]
pub(super) struct AutotuneRollbackTokenHandler;

impl RollbackHandler for AutotuneRollbackTokenHandler {
    fn id(&self) -> &'static str {
        "autotune-rollback-token"
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        Ok(Vec::new())
    }

    fn dry_run(&self, _: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        anyhow::bail!("autotune rollback token handler requires an explicit rollback token")
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        anyhow::bail!("autotune rollback token handler requires an explicit rollback token")
    }

    fn supports_token(&self, _: &RollbackToken) -> bool {
        true
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        Ok(RollbackPreview {
            handler_id: self.id(),
            restore_path: token.restore_path().cloned().unwrap_or_default(),
            affected_tasks: token.affected_tasks(),
            message: format!(
                "would restore rollback_kind={} manual_restore_command=\"{}\"",
                rollback_token_kind(token),
                manual_restore_command_for_token(token)
            ),
        })
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        let summary = restore_rollback_token_direct(token)?;
        Ok(RollbackResult {
            handler_id: self.id(),
            restore_path: token.restore_path().cloned().unwrap_or_default(),
            restored: summary.restored_items,
            skipped_dead: summary.skipped_items,
            skipped_identity_mismatch: 0,
            legacy_unverified: 0,
            errors: 0,
            messages: summary.messages,
        })
    }
}

pub fn default_autotune_rollback_registry() -> RollbackRegistry {
    let mut registry = crate::actions::default_rollback_registry();
    registry.register(AutotuneRollbackTokenHandler);
    registry
}

pub(super) fn rollback_restore_summary_from_registry_result(
    token: &RollbackToken,
    result: RollbackResult,
) -> RollbackRestoreSummary {
    RollbackRestoreSummary {
        rollback_kind: rollback_token_kind(token).to_owned(),
        restored_items: result.restored,
        skipped_items: result.skipped_dead
            + result.skipped_identity_mismatch
            + result.legacy_unverified,
        skipped_missing: result.skipped_dead,
        skipped_identity_mismatch: result.skipped_identity_mismatch,
        failed_items: result.errors,
        messages: result.messages,
    }
}
