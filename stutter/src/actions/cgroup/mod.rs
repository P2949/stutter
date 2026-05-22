//! Cgroup placement action.
//!
//! Owns audited task migration into configured cgroups and cpuset restoration.
//! Helper modules separate validation, procfs identity checks, filesystem I/O,
//! and rollback handling so mutation sequencing remains reviewable.

use std::{
    fs,
    fs::OpenOptions,
    io,
    path::{Component, Path, PathBuf},
};

use anyhow::Context;

use crate::{
    actions::{
        ActionId, ActionState, ActionWarning, ApplyResult, CgroupCpusetRestoreRecord,
        CgroupRestoreRecord, PartialApplyError, RestoreIdentityStatus, RollbackToken, SafetyClass,
        TaskIdentity, TaskRestoreIdentity, TuningAction,
        restore_write::{RestoreSummary, RestoreWriteError, classify_restore_write_error},
        rollback::{
            RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult,
            token_dry_run_preview,
        },
        verify_task_identity,
    },
    process_tree::TaskClass,
};

mod fs_io;
mod model;
mod procfs;
mod rollback;
mod validation;

#[cfg(test)]
mod tests;

use fs_io::*;
use model::CgroupTargetSnapshot;
pub use model::{CgroupPlacementAction, CgroupPlacementPolicy, CgroupPlacementTarget};
use procfs::*;
pub(crate) use rollback::CgroupRollbackHandler;
use rollback::*;
use validation::*;
impl CgroupPlacementAction {
    pub fn preflight_with_policy(
        &self,
        policy: &CgroupPlacementPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_at(Path::new("/proc"), policy)
    }

    #[cfg(test)]
    pub(crate) fn preflight_with_policy_at_for_tests(
        &self,
        proc_root: &Path,
        policy: &CgroupPlacementPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_at(proc_root, policy)
    }

    fn preflight_at(
        &self,
        proc_root: &Path,
        policy: &CgroupPlacementPolicy,
    ) -> anyhow::Result<Vec<ActionWarning>> {
        self.collect_target_snapshots_at(proc_root, policy)
            .map(|snapshots| {
                snapshots
                    .into_iter()
                    .flat_map(|(_, warnings)| warnings)
                    .collect()
            })
    }

    fn dry_run_at(
        &self,
        proc_root: &Path,
        policy: &CgroupPlacementPolicy,
    ) -> anyhow::Result<ActionState> {
        let snapshots = self.collect_target_snapshots_at(proc_root, policy)?;
        let target_abs = self.target_cgroup_abs()?;
        let mut warnings = Vec::new();
        let mut pending_changes = 0usize;

        for (snapshot, target_warnings) in snapshots {
            warnings.extend(target_warnings);
            if self
                .cgroup_root
                .join(strip_cgroup_leading_slash(&snapshot.original_cgroup))
                != target_abs
            {
                pending_changes += 1;
            }
        }

        if self.cpuset_cpus.is_some() || self.cpuset_mems.is_some() {
            pending_changes = pending_changes.saturating_add(1);
        }

        Ok(ActionState {
            applied: false,
            affected_tasks: pending_changes,
            checked_tasks: self.targets.len(),
            pending_changes,
            warnings,
        })
    }

    fn verify_at(
        &self,
        proc_root: &Path,
        policy: &CgroupPlacementPolicy,
    ) -> anyhow::Result<ActionState> {
        validate_action_request(self, policy)?;
        let target_rel = normalize_cgroup_path(&self.target_cgroup)?;
        let target_abs = self.target_cgroup_abs()?;
        let mut warnings = Vec::new();
        let mut checked_tasks = 0usize;
        let mut pending_changes = 0usize;

        for target in &self.targets {
            if !task_exists(proc_root, target.identity.tid) {
                warnings.push(ActionWarning {
                    message: format!(
                        "target tid={} exited before cgroup placement verify completed",
                        target.identity.tid
                    ),
                });
                continue;
            }

            checked_tasks += 1;
            let current =
                read_proc_cgroup_path_at(proc_root, target.identity.tid).with_context(|| {
                    format!(
                        "failed to read current cgroup for tid={}",
                        target.identity.tid
                    )
                })?;

            if normalize_cgroup_path(&current)? != target_rel {
                pending_changes += 1;
            }
        }

        if self.cpuset_cpus.is_some() {
            let current = read_trimmed(&target_abs.join("cpuset.cpus")).with_context(|| {
                format!(
                    "failed to read {}",
                    target_abs.join("cpuset.cpus").display()
                )
            })?;
            if Some(current.as_str()) != self.cpuset_cpus.as_deref() {
                pending_changes += 1;
            }
        }

        if self.cpuset_mems.is_some() {
            let current = read_trimmed(&target_abs.join("cpuset.mems")).with_context(|| {
                format!(
                    "failed to read {}",
                    target_abs.join("cpuset.mems").display()
                )
            })?;
            if Some(current.as_str()) != self.cpuset_mems.as_deref() {
                pending_changes += 1;
            }
        }

        Ok(ActionState {
            applied: checked_tasks > 0 && pending_changes == 0,
            affected_tasks: checked_tasks,
            checked_tasks,
            pending_changes,
            warnings,
        })
    }

    fn collect_target_snapshots_at(
        &self,
        proc_root: &Path,
        policy: &CgroupPlacementPolicy,
    ) -> anyhow::Result<Vec<(CgroupTargetSnapshot, Vec<ActionWarning>)>> {
        validate_action_request(self, policy)?;
        preflight_cgroup_files(self, policy)?;

        let mut snapshots = Vec::with_capacity(self.targets.len());

        for target in &self.targets {
            validate_target_class(target.class)?;
            let snapshot =
                read_target_snapshot_at(proc_root, &target.identity).with_context(|| {
                    format!(
                        "failed to preflight cgroup target tid={}",
                        target.identity.tid
                    )
                })?;
            let warnings = identity_warnings(&target.identity, &snapshot);
            snapshots.push((snapshot, warnings));
        }

        Ok(snapshots)
    }

    fn cpuset_restore_record(
        &self,
        target_abs: &Path,
    ) -> anyhow::Result<Option<CgroupCpusetRestoreRecord>> {
        if self.cpuset_cpus.is_none() && self.cpuset_mems.is_none() {
            return Ok(None);
        }

        Ok(Some(CgroupCpusetRestoreRecord {
            cgroup_path: normalize_cgroup_path(&self.target_cgroup)?,
            original_cpuset_cpus: if self.cpuset_cpus.is_some() {
                read_optional_trimmed(&target_abs.join("cpuset.cpus"))?
            } else {
                None
            },
            original_cpuset_mems: if self.cpuset_mems.is_some() {
                read_optional_trimmed(&target_abs.join("cpuset.mems"))?
            } else {
                None
            },
        }))
    }

    fn target_cgroup_abs(&self) -> anyhow::Result<PathBuf> {
        let target_rel = normalize_cgroup_path(&self.target_cgroup)?;
        Ok(self
            .cgroup_root
            .join(strip_cgroup_leading_slash(&target_rel)))
    }

    pub(crate) fn rollback_at(
        &self,
        proc_root: &Path,
        token: &RollbackToken,
    ) -> anyhow::Result<()> {
        let RollbackToken::CgroupRestore { records, cpuset } = token else {
            anyhow::bail!("rollback token is not a cgroup restore token");
        };

        let mut failures = Vec::new();
        let mut summary = RestoreSummary::default();

        for record in records {
            let identity = record.restore_identity();
            let tid = identity.tid;
            match verify_task_identity(proc_root, &identity) {
                RestoreIdentityStatus::SameTask => {}
                RestoreIdentityStatus::UnknownLegacy => {
                    log::warn!(
                        "cgroup rollback running in legacy mode (unverified identity) for tid={}",
                        tid
                    );
                }
                RestoreIdentityStatus::Missing => {
                    summary.record_missing();
                    log::info!(
                        "cgroup_rollback_skip_exited_task tid={} original_cgroup={}",
                        tid,
                        record.original_cgroup.display()
                    );
                    continue;
                }
                RestoreIdentityStatus::Mismatch { reason } => {
                    summary.record_identity_mismatch();
                    log::warn!(
                        "cgroup_rollback_skip_identity_mismatch tid={} original_cgroup={} reason={}",
                        tid,
                        record.original_cgroup.display(),
                        reason
                    );
                    continue;
                }
            }

            let original_abs = self
                .cgroup_root
                .join(strip_cgroup_leading_slash(&record.original_cgroup));
            let procs = original_abs.join("cgroup.procs");

            if let Err(err) = write_trimmed(&procs, &tid.to_string()) {
                match classify_restore_write_error(proc_root, tid, err) {
                    RestoreWriteError::MissingTask => {
                        summary.record_missing();
                        log::info!(
                            "cgroup_rollback_skip_exited_task tid={} original_cgroup={}",
                            tid,
                            record.original_cgroup.display()
                        );
                    }
                    RestoreWriteError::PermissionDenied(err)
                    | RestoreWriteError::InvalidValue(err)
                    | RestoreWriteError::Io(err) => {
                        summary.record_failure();
                        failures.push(format!(
                            "tid={} original_cgroup={} error={err:#}",
                            tid,
                            original_abs.display()
                        ));
                    }
                }
            } else {
                summary.record_restored();
            }
        }

        if let Some(cpuset) = cpuset
            && let Err(err) = restore_cpuset_record(&self.cgroup_root, cpuset)
        {
            summary.record_failure();
            failures.push(format!("cpuset_restore_error={err:#}"));
        }

        if summary.has_failures() {
            anyhow::bail!(
                "failed to rollback cgroup placement after attempting all records: restored={} skipped_missing={} skipped_identity_mismatch={} failed={} errors={}",
                summary.restored,
                summary.skipped_missing,
                summary.skipped_identity_mismatch,
                summary.failed,
                failures.join("; ")
            );
        }

        Ok(())
    }

    fn apply_at(&self, proc_root: &Path, policy: &CgroupPlacementPolicy) -> ApplyResult {
        let mut writer = FsCgroupFileWriter;
        self.apply_at_with_writer(proc_root, policy, &mut writer)
    }

    fn apply_at_with_writer<W: CgroupFileWriter>(
        &self,
        proc_root: &Path,
        policy: &CgroupPlacementPolicy,
        writer: &mut W,
    ) -> ApplyResult {
        let snapshots = self.collect_target_snapshots_at(proc_root, policy)?;
        let target_abs = self.target_cgroup_abs()?;
        let cpuset = self.cpuset_restore_record(&target_abs)?;
        let mut cpuset_changed = false;

        if let Some(cpuset_cpus) = &self.cpuset_cpus {
            writer
                .write_trimmed(&target_abs.join("cpuset.cpus"), cpuset_cpus)
                .with_context(|| {
                    format!(
                        "failed to write {}",
                        target_abs.join("cpuset.cpus").display()
                    )
                })?;
            cpuset_changed = true;
        }

        if let Some(cpuset_mems) = &self.cpuset_mems
            && let Err(err) = writer
                .write_trimmed(&target_abs.join("cpuset.mems"), cpuset_mems)
                .with_context(|| {
                    format!(
                        "failed to write {}",
                        target_abs.join("cpuset.mems").display()
                    )
                })
        {
            return Err(self.partial_apply_error_after_rollback(
                proc_root,
                err,
                Vec::new(),
                cpuset_changed,
                &cpuset,
            ));
        }
        if self.cpuset_mems.is_some() {
            cpuset_changed = true;
        }

        let mut records = Vec::new();
        for (snapshot, _) in snapshots {
            let current_target = self
                .cgroup_root
                .join(strip_cgroup_leading_slash(&snapshot.original_cgroup));
            if current_target == target_abs {
                continue;
            }

            let identity = TaskRestoreIdentity::observed(
                snapshot.tid,
                snapshot.process_pid,
                snapshot.comm.clone(),
                snapshot.starttime_ticks,
                snapshot.exe.clone(),
            );

            if let Err(err) = writer
                .write_trimmed(&target_abs.join("cgroup.procs"), &snapshot.tid.to_string())
                .with_context(|| {
                    format!(
                        "failed to move tid={} to {}",
                        snapshot.tid,
                        target_abs.display()
                    )
                })
            {
                return Err(self.partial_apply_error_after_rollback(
                    proc_root,
                    err,
                    records,
                    cpuset_changed,
                    &cpuset,
                ));
            }

            records.push(CgroupRestoreRecord::new(identity, snapshot.original_cgroup));
        }

        Ok(RollbackToken::CgroupRestore {
            records,
            cpuset: cpuset.filter(|_| cpuset_changed),
        })
    }

    fn partial_apply_error_after_rollback(
        &self,
        proc_root: &Path,
        source: anyhow::Error,
        records: Vec<CgroupRestoreRecord>,
        cpuset_changed: bool,
        cpuset: &Option<CgroupCpusetRestoreRecord>,
    ) -> PartialApplyError {
        let rollback = cgroup_partial_token(records, cpuset_changed, cpuset);
        let source = match rollback.as_ref() {
            Some(token) => match self.rollback_at(proc_root, token) {
                Ok(()) => source,
                Err(rollback_err) => anyhow::anyhow!(
                    "apply failed: {source:#}; partial cgroup rollback failed: {rollback_err:#}"
                ),
            },
            None => source,
        };

        PartialApplyError { source, rollback }
    }
}

impl TuningAction for CgroupPlacementAction {
    fn id(&self) -> ActionId {
        ActionId::new(format!(
            "cgroup:place:{}:targets:{}",
            normalize_cgroup_path(&self.target_cgroup)
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| self.target_cgroup.display().to_string()),
            self.targets.len()
        ))
    }

    fn describe(&self) -> String {
        let tids = self
            .targets
            .iter()
            .map(|target| target.identity.tid.to_string())
            .collect::<Vec<_>>()
            .join(",");
        format!(
            "move task(s) [{}] to cgroup {}",
            tids,
            self.target_cgroup.display()
        )
    }

    fn safety_class(&self) -> SafetyClass {
        SafetyClass::ReversibleMediumRisk
    }

    fn preflight(&self) -> anyhow::Result<Vec<ActionWarning>> {
        self.preflight_with_policy(&CgroupPlacementPolicy::default())
    }

    fn dry_run(&self) -> anyhow::Result<ActionState> {
        self.dry_run_at(Path::new("/proc"), &CgroupPlacementPolicy::default())
    }

    fn apply(&self) -> ApplyResult {
        self.apply_at(Path::new("/proc"), &CgroupPlacementPolicy::default())
    }

    fn verify(&self) -> anyhow::Result<ActionState> {
        self.verify_at(Path::new("/proc"), &CgroupPlacementPolicy::default())
    }

    fn rollback(&self, token: &RollbackToken) -> anyhow::Result<()> {
        self.rollback_at(Path::new("/proc"), token)
    }
}
