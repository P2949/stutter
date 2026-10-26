//! The prepared-before-mutation transaction of cgroup placement: every move is planned with the
//! task's complete identity and the identity of the cgroup it leaves (its inode, which a destroyed
//! and recreated cgroup of the same path does not share), the token is made durable through the
//! caller's hook, and only then is the first file written. A target that changed in between aborts
//! the apply with zero writes.
use std::{os::unix::fs::MetadataExt, path::Path};

use anyhow::Context;

use super::{
    CgroupFileWriter, FsCgroupFileWriter,
    fs_io::strip_cgroup_leading_slash,
    model::{CgroupPlacementAction, CgroupPlacementPolicy},
};
use crate::actions::{
    ApplyResult, CgroupCpusetRestoreRecord, CgroupRestoreRecord, PartialApplyError, RollbackToken,
    TaskRestoreIdentity,
};

/// What an apply is going to change, captured before the first write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CgroupPlan {
    pub(super) records: Vec<CgroupRestoreRecord>,
    pub(super) cpuset: Option<CgroupCpusetRestoreRecord>,
    pub(super) target_cgroup_id: u64,
}

impl CgroupPlan {
    pub(super) fn token(&self) -> RollbackToken {
        RollbackToken::CgroupRestore {
            records: self.records.clone(),
            cpuset: self.cpuset.clone(),
        }
    }
}

/// The inode of a cgroup directory: the kernel's identity of that exact cgroup.
pub(crate) fn cgroup_id(path: &Path) -> anyhow::Result<u64> {
    Ok(std::fs::metadata(path)
        .with_context(|| format!("cgroup {} is not readable", path.display()))?
        .ino())
}

impl CgroupPlacementAction {
    pub(super) fn plan_moves(
        &self,
        proc_root: &Path,
        policy: &CgroupPlacementPolicy,
    ) -> anyhow::Result<CgroupPlan> {
        let snapshots = self.collect_target_snapshots_at(proc_root, policy)?;
        let target_abs = self.target_cgroup_abs()?;
        let target_cgroup_id = cgroup_id(&target_abs)?;
        let cpuset = self.cpuset_restore_record(&target_abs)?;
        let mut records = Vec::new();
        for (snapshot, _) in snapshots {
            anyhow::ensure!(
                snapshot.starttime_ticks.is_some() && snapshot.process_pid.is_some(),
                "cgroup target tid={} lacks a complete task identity",
                snapshot.tid
            );
            let original_abs = self
                .cgroup_root
                .join(strip_cgroup_leading_slash(&snapshot.original_cgroup));
            if original_abs == target_abs {
                continue;
            }
            let identity = TaskRestoreIdentity::observed(
                snapshot.tid,
                snapshot.process_pid,
                snapshot.comm.clone(),
                snapshot.starttime_ticks,
                snapshot.exe.clone(),
            );
            records.push(
                CgroupRestoreRecord::new(identity, snapshot.original_cgroup)
                    .with_cgroup_id(cgroup_id(&original_abs)?),
            );
        }
        Ok(CgroupPlan {
            records,
            cpuset,
            target_cgroup_id,
        })
    }

    pub(crate) fn apply_prepared_at(
        &self,
        proc_root: &Path,
        policy: &CgroupPlacementPolicy,
        before_mutation: &mut dyn FnMut(&RollbackToken) -> anyhow::Result<()>,
    ) -> ApplyResult {
        let plan = self.plan_moves(proc_root, policy)?;
        let token = plan.token();
        // The identity-complete original placement is durable before the first write.
        before_mutation(&token)?;
        // Anything that moved between preparation and commit means the snapshot no longer
        // describes the host; abort with zero writes rather than restore a stale placement.
        if self.plan_moves(proc_root, policy)? != plan {
            return Err(anyhow::anyhow!(
                "cgroup target drifted between preparation and commit; nothing was written"
            )
            .into());
        }
        let target_abs = self.target_cgroup_abs()?;
        let mut writer = FsCgroupFileWriter;
        let failed = |source: anyhow::Error| PartialApplyError {
            source,
            // Restoring what was not yet changed writes the value it already has.
            rollback: Some(token.clone()),
        };
        if let Some(cpus) = &self.cpuset_cpus {
            writer
                .write_trimmed(&target_abs.join("cpuset.cpus"), cpus)
                .with_context(|| format!("failed to write {}", target_abs.display()))
                .map_err(failed)?;
        }
        if let Some(mems) = &self.cpuset_mems {
            writer
                .write_trimmed(&target_abs.join("cpuset.mems"), mems)
                .with_context(|| format!("failed to write {}", target_abs.display()))
                .map_err(failed)?;
        }
        // `cgroup.procs` is process-granular: writing any thread id moves its whole thread group,
        // so one write per process is the whole mutation and a second one would be a no-op.
        let mut moved = std::collections::BTreeSet::new();
        for record in &plan.records {
            let process = record
                .restore_identity()
                .process_pid
                .map_or(record.tid(), |pid| pid.as_u32());
            if !moved.insert(process) {
                continue;
            }
            writer
                .write_trimmed(&target_abs.join("cgroup.procs"), &record.tid().to_string())
                .with_context(|| {
                    format!(
                        "failed to move tid={} to {}",
                        record.tid(),
                        target_abs.display()
                    )
                })
                .map_err(failed)?;
        }
        Ok(token)
    }
}
