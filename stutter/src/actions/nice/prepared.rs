//! The prepared-before-mutation transaction of nice: every target is snapshotted with a complete
//! task identity, the token is made durable through the caller's hook, and only then is the first
//! write issued. A target that changed in between aborts the apply with zero writes.
use std::path::Path;

use super::model::{NiceAction, set_task_nice};
use crate::actions::{ApplyResult, NiceRestoreRecord, RollbackToken, TaskRestoreIdentity};

impl NiceAction {
    /// Original-value records for every target whose nice really changes, each bound to the
    /// observed task identity. `require_identity` demands a process id and start time so a
    /// recycled TID can never match.
    pub(super) fn plan_records(
        &self,
        proc_root: &Path,
        require_identity: bool,
    ) -> anyhow::Result<Vec<NiceRestoreRecord>> {
        let mut plan = Vec::new();
        for (snapshot, _) in self.collect_target_snapshots_at(proc_root, &self.policy)? {
            if require_identity {
                anyhow::ensure!(
                    snapshot.starttime_ticks.is_some() && snapshot.process_pid.is_some(),
                    "nice target tid={} lacks a complete task identity",
                    snapshot.tid
                );
            }
            if snapshot.current_nice == self.nice {
                continue;
            }
            plan.push(NiceRestoreRecord::new(
                TaskRestoreIdentity::observed(
                    snapshot.tid,
                    snapshot.process_pid,
                    snapshot.comm.clone(),
                    snapshot.starttime_ticks,
                    snapshot.exe.clone(),
                ),
                snapshot.current_nice,
            ));
        }
        Ok(plan)
    }

    pub(super) fn apply_plan(&self, plan: Vec<NiceRestoreRecord>) -> ApplyResult {
        crate::actions::transaction::ApplyTransaction::new().apply_planned_loop(
            plan,
            |record| {
                set_task_nice(record.tid(), self.nice)
                    .map_err(|e| e.context(format!("failed to set nice for tid={}", record.tid())))
            },
            |records| RollbackToken::NiceRestore { records },
        )
    }

    pub(crate) fn apply_prepared_at(
        &self,
        proc_root: &Path,
        before_mutation: &mut dyn FnMut(&RollbackToken) -> anyhow::Result<()>,
    ) -> ApplyResult {
        let plan = self.plan_records(proc_root, true)?;
        // The identity-complete original values are durable before the first write.
        before_mutation(&RollbackToken::NiceRestore {
            records: plan.clone(),
        })?;
        // Anything that moved between preparation and commit means the snapshot no longer
        // describes the host; abort with zero writes rather than restore a stale value.
        if self.plan_records(proc_root, true)? != plan {
            return Err(anyhow::anyhow!(
                "nice target drifted between preparation and commit; nothing was written"
            )
            .into());
        }
        self.apply_plan(plan)
    }
}
