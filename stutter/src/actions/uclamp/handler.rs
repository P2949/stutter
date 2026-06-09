use std::path::Path;

use super::{models::UclampCurrentValues, system::set_task_uclamp};
use crate::actions::{
    ActionBoundaryError, RestoreIdentityStatus, RollbackToken,
    restore_write::{RestoreWriteError, classify_restore_write_error},
    rollback::{
        RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult, token_dry_run_preview,
    },
    verify_task_identity,
};

pub(crate) struct UclampRollbackHandler;

impl RollbackHandler for UclampRollbackHandler {
    fn id(&self) -> &'static str {
        "uclamp-rollback"
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        Ok(Vec::new())
    }

    fn dry_run(&self, _: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        Err(
            ActionBoundaryError::missing_explicit_rollback_token(self.id(), "uclamp-restore")
                .into(),
        )
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        Err(
            ActionBoundaryError::missing_explicit_rollback_token(self.id(), "uclamp-restore")
                .into(),
        )
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        token.as_uclamp_restore().is_some()
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        if !self.supports_token(token) {
            return Err(ActionBoundaryError::unsupported_rollback_token(
                self.id(),
                "uclamp-restore",
                token.kind(),
            )
            .into());
        }
        Ok(token_dry_run_preview(self.id(), token, "uclamp-restore"))
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        let Some(records) = token.as_uclamp_restore() else {
            return Err(ActionBoundaryError::unsupported_rollback_token(
                self.id(),
                "uclamp-restore",
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
                    log::debug!("uclamp restore skipped: task tid={} is missing/dead", tid);
                    continue;
                }
                RestoreIdentityStatus::Mismatch { reason } => {
                    skipped_identity_mismatch += 1;
                    let msg = format!(
                        "uclamp restore identity mismatch for tid={}: {}",
                        tid, reason
                    );
                    log::warn!("{}", msg);
                    messages.push(msg);
                    continue;
                }
                RestoreIdentityStatus::UnknownLegacy => {
                    legacy_unverified += 1;
                    log::warn!(
                        "uclamp restore running in legacy mode (unverified identity) for tid={}",
                        tid
                    );
                }
                RestoreIdentityStatus::SameTask => {}
            }

            let requested = UclampCurrentValues {
                sched_util_min: record.original_util_min,
                sched_util_max: record.original_util_max,
            };

            match set_task_uclamp(tid, requested) {
                Ok(_) => {
                    restored += 1;
                }
                Err(e) => match classify_restore_write_error(Path::new("/proc"), tid, e) {
                    RestoreWriteError::MissingTask => {
                        skipped_dead += 1;
                        log::debug!("uclamp restore skipped: task tid={} is dead", tid);
                    }
                    RestoreWriteError::PermissionDenied(e)
                    | RestoreWriteError::InvalidValue(e)
                    | RestoreWriteError::Io(e) => {
                        errors += 1;
                        let msg = format!(
                            "failed to restore uclamp min={} max={} for tid={}: {}",
                            record.original_util_min, record.original_util_max, tid, e
                        );
                        log::error!("{}", msg);
                        messages.push(msg);
                    }
                },
            }
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
