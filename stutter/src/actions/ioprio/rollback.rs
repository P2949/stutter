use std::path::Path;

use super::apply::set_task_ioprio;
use crate::actions::{
    ActionBoundaryError, RestoreIdentityStatus, RollbackToken,
    restore_write::{RestoreWriteError, classify_restore_write_error},
    rollback::{
        RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult, token_dry_run_preview,
    },
    verify_task_identity,
};

pub(crate) struct IoPrioRollbackHandler;

impl RollbackHandler for IoPrioRollbackHandler {
    fn id(&self) -> &'static str {
        "ioprio-rollback"
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        Ok(Vec::new())
    }

    fn dry_run(&self, _: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        Err(
            ActionBoundaryError::missing_explicit_rollback_token(self.id(), "ioprio-restore")
                .into(),
        )
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        Err(
            ActionBoundaryError::missing_explicit_rollback_token(self.id(), "ioprio-restore")
                .into(),
        )
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        token.as_ioprio_restore().is_some()
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        if !self.supports_token(token) {
            return Err(ActionBoundaryError::unsupported_rollback_token(
                self.id(),
                "ioprio-restore",
                token.kind(),
            )
            .into());
        }
        Ok(token_dry_run_preview(self.id(), token, "ioprio-restore"))
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        let Some(records) = token.as_ioprio_restore() else {
            return Err(ActionBoundaryError::unsupported_rollback_token(
                self.id(),
                "ioprio-restore",
                token.kind(),
            )
            .into());
        };

        let mut restored = 0;
        let mut skipped_dead = 0;
        let mut skipped_identity_mismatch = 0;
        let mut legacy_unverified = 0;
        let mut errors = 0;
        let mut messages = Vec::new();

        for record in records {
            let identity = record.restore_identity();
            let tid = identity.tid.as_u32();
            let status = verify_task_identity(Path::new("/proc"), &identity);
            match status {
                RestoreIdentityStatus::Missing => {
                    skipped_dead += 1;
                    log::debug!("ioprio restore skipped: task tid={} is missing/dead", tid);
                    continue;
                }
                RestoreIdentityStatus::Mismatch { reason } => {
                    skipped_identity_mismatch += 1;
                    let msg = format!(
                        "ioprio restore identity mismatch for tid={}: {}",
                        tid, reason
                    );
                    log::warn!("{}", msg);
                    messages.push(msg);
                    continue;
                }
                RestoreIdentityStatus::UnknownLegacy => {
                    legacy_unverified += 1;
                    log::warn!(
                        "ioprio restore running in legacy mode (unverified identity) for tid={}",
                        tid
                    );
                }
                RestoreIdentityStatus::SameTask => {}
            }

            match set_task_ioprio(tid, record.original_ioprio) {
                Ok(_) => {
                    restored += 1;
                }
                Err(e) => match classify_restore_write_error(Path::new("/proc"), tid, e) {
                    RestoreWriteError::MissingTask => {
                        skipped_dead += 1;
                        log::debug!("ioprio restore skipped: task tid={} is dead", tid);
                    }
                    RestoreWriteError::PermissionDenied(e)
                    | RestoreWriteError::InvalidValue(e)
                    | RestoreWriteError::Io(e) => {
                        errors += 1;
                        let msg = format!(
                            "failed to restore original I/O priority={} for tid={}: {}",
                            record.original_ioprio, tid, e
                        );
                        log::error!("{}", msg);
                        messages.push(msg);
                    }
                },
            }
        }

        if errors > 0 {
            return Err(ActionBoundaryError::restore_failed(
                "ioprio",
                format!("failed to restore I/O priority: {}", messages.join("; ")),
            )
            .into());
        }

        Ok(RollbackResult {
            handler_id: self.id(),
            restore_path: token.restore_path().cloned().unwrap_or_default(),
            restored,
            skipped_dead,
            skipped_identity_mismatch,
            legacy_unverified,
            errors,
            messages,
        })
    }
}
