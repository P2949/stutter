use std::path::Path;

use anyhow::Context;

use super::model::{IoPrioAction, IoPrioValue};
use crate::actions::{
    ActionId, ActionState, ActionWarning, ApplyResult, IoPrioRestoreRecord, RollbackToken,
    SafetyClass, TaskRestoreIdentity, TuningAction,
};

impl TuningAction for IoPrioAction {
    fn id(&self) -> ActionId {
        ActionId::new(format!(
            "ioprio:set:{}:targets:{}",
            self.ioprio.label(),
            self.targets.len()
        ))
    }

    fn describe(&self) -> String {
        let tids = self
            .targets
            .iter()
            .map(|target| target.tid.to_string())
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "set I/O priority {} for task(s) [{}]",
            self.ioprio.label(),
            tids
        )
    }

    fn safety_class(&self) -> SafetyClass {
        SafetyClass::ReversibleMediumRisk
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_with_policy(&self.policy)
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.dry_run_at(Path::new("/proc"), &self.policy)
    }

    fn apply(&self) -> ApplyResult {
        (|| {
            let snapshots = self.collect_target_snapshots_at(Path::new("/proc"), &self.policy)?;
            let requested = self.ioprio.encode()?;
            let filtered_snapshots: Vec<_> = snapshots
                .into_iter()
                .filter(|(snapshot, _)| snapshot.current_ioprio != requested)
                .collect();

            let records = filtered_snapshots
                .into_iter()
                .map(|(snapshot, _)| {
                    let identity = TaskRestoreIdentity::observed(
                        snapshot.tid,
                        snapshot.process_pid,
                        snapshot.comm.clone(),
                        snapshot.starttime_ticks,
                        snapshot.exe.clone(),
                    );

                    IoPrioRestoreRecord::new(identity, snapshot.current_ioprio)
                })
                .collect::<Vec<_>>();

            let tx = crate::actions::transaction::ApplyTransaction::new();
            tx.apply_planned_loop(
                records,
                |record| {
                    set_task_ioprio(record.tid(), requested).map_err(|e| {
                        e.context(format!(
                            "failed to set I/O priority for tid={}",
                            record.tid()
                        ))
                    })
                },
                |records| RollbackToken::IoPrioRestore { records },
            )
        })()
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.verify_at(Path::new("/proc"), &self.policy)
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        let Some(records) = token.as_ioprio_restore() else {
            return Err(crate::actions::ActionError::invalid_rollback_token_kind(
                token.kind_error("ioprio-restore"),
            )
            .into());
        };

        let mut summary = crate::actions::restore_write::RestoreSummary::default();
        let mut failures = Vec::new();

        for record in records {
            let identity = record.restore_identity();
            let tid = identity.tid.as_u32();
            match crate::actions::verify_task_identity(Path::new("/proc"), &identity) {
                crate::actions::RestoreIdentityStatus::SameTask => {}
                crate::actions::RestoreIdentityStatus::UnknownLegacy => {
                    log::warn!(
                        "ioprio rollback running in legacy mode (unverified identity) for tid={}",
                        tid
                    );
                }
                crate::actions::RestoreIdentityStatus::Missing => {
                    summary.record_missing();
                    log::warn!("ioprio rollback skipped: task tid={} is missing/dead", tid);
                    continue;
                }
                crate::actions::RestoreIdentityStatus::Mismatch { reason } => {
                    summary.record_identity_mismatch();
                    log::warn!(
                        "ioprio rollback skipped identity mismatch for tid={}: {}",
                        tid,
                        reason
                    );
                    continue;
                }
            }

            match set_task_ioprio(tid, record.original_ioprio) {
                Ok(()) => summary.record_restored(),
                Err(err) => match crate::actions::restore_write::classify_restore_write_error(
                    Path::new("/proc"),
                    tid,
                    err,
                ) {
                    crate::actions::restore_write::RestoreWriteError::MissingTask => {
                        summary.record_missing();
                        log::warn!("ioprio rollback skipped: task tid={} is missing/dead", tid);
                    }
                    crate::actions::restore_write::RestoreWriteError::PermissionDenied(err)
                    | crate::actions::restore_write::RestoreWriteError::InvalidValue(err)
                    | crate::actions::restore_write::RestoreWriteError::Io(err) => {
                        summary.record_failure();
                        failures.push(format!(
                            "failed to restore original I/O priority={} for tid={}: {err:#}",
                            record.original_ioprio, tid
                        ));
                    }
                },
            }
        }

        if summary.has_failures() {
            anyhow::bail!(
                "failed to rollback I/O priority after attempting all records: restored={} skipped_missing={} skipped_identity_mismatch={} failed={} errors={}",
                summary.restored,
                summary.skipped_missing,
                summary.skipped_identity_mismatch,
                summary.failed,
                failures.join("; ")
            );
        }

        Ok(())
    }
}

pub(crate) fn read_task_ioprio(tid: u32) -> anyhow::Result<i32> {
    crate::actions::syscalls::ioprio_get_process(tid)
        .with_context(|| format!("ioprio_get(IOPRIO_WHO_PROCESS, {tid}) failed"))
}

pub(crate) fn set_task_ioprio(tid: u32, encoded_ioprio: i32) -> anyhow::Result<()> {
    IoPrioValue::decode(encoded_ioprio)?;

    crate::actions::syscalls::ioprio_set_process(tid, encoded_ioprio).with_context(|| {
        format!(
            "ioprio_set(IOPRIO_WHO_PROCESS, {}, {}) failed",
            tid, encoded_ioprio
        )
    })
}
