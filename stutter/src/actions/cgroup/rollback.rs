use std::path::Path;

use anyhow::Context;

use super::fs_io::{resolve_cgroup_fs_path, write_trimmed};
use crate::actions::{
    CgroupCpusetRestoreRecord, CgroupRestoreRecord, RestoreIdentityStatus, RollbackToken,
    restore_write::{RestoreWriteError, classify_restore_write_error},
    rollback::{
        RollbackCandidate, RollbackHandler, RollbackPreview, RollbackResult, token_dry_run_preview,
    },
    verify_task_identity,
};

pub(crate) struct CgroupRollbackHandler;

impl RollbackHandler for CgroupRollbackHandler {
    fn id(&self) -> &'static str {
        "cgroup-rollback"
    }

    fn discover(&self) -> anyhow::Result<Vec<RollbackCandidate>> {
        Ok(Vec::new())
    }

    fn dry_run(&self, _: &RollbackCandidate) -> anyhow::Result<RollbackPreview> {
        anyhow::bail!("cgroup rollback requires an explicit rollback token")
    }

    fn restore(&self, _: RollbackCandidate) -> anyhow::Result<RollbackResult> {
        anyhow::bail!("cgroup rollback requires an explicit rollback token")
    }

    fn supports_token(&self, token: &RollbackToken) -> bool {
        matches!(token, RollbackToken::CgroupRestore { .. })
    }

    fn dry_run_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackPreview> {
        if !self.supports_token(token) {
            anyhow::bail!("cgroup rollback handler does not support {token:?}");
        }
        Ok(token_dry_run_preview(self.id(), token, "cgroup-restore"))
    }

    fn restore_token(&self, token: &RollbackToken) -> anyhow::Result<RollbackResult> {
        self.restore_token_at(Path::new("/proc"), Path::new("/sys/fs/cgroup"), token)
    }
}

impl CgroupRollbackHandler {
    pub(super) fn restore_token_at(
        &self,
        proc_root: &Path,
        cgroup_root: &Path,
        token: &RollbackToken,
    ) -> anyhow::Result<RollbackResult> {
        let RollbackToken::CgroupRestore { records, cpuset } = token else {
            anyhow::bail!("cgroup rollback handler does not support {token:?}");
        };

        let mut restored = 0;
        let mut skipped_dead = 0;
        let mut skipped_identity_mismatch = 0;
        let mut legacy_unverified = 0;
        let mut errors = 0;
        let mut messages = Vec::new();

        for record in records {
            let identity = record.restore_identity();
            let tid = identity.tid;
            let status = verify_task_identity(proc_root, &identity);
            match status {
                RestoreIdentityStatus::Missing => {
                    skipped_dead += 1;
                    log::debug!("cgroup restore skipped: task tid={} is missing/dead", tid);
                    continue;
                }
                RestoreIdentityStatus::Mismatch { reason } => {
                    skipped_identity_mismatch += 1;
                    let msg = format!(
                        "cgroup restore identity mismatch for tid={}: {}",
                        tid, reason
                    );
                    log::warn!("{}", msg);
                    messages.push(msg);
                    continue;
                }
                RestoreIdentityStatus::UnknownLegacy => {
                    legacy_unverified += 1;
                    log::warn!(
                        "cgroup restore running in legacy mode (unverified identity) for tid={}",
                        tid
                    );
                }
                RestoreIdentityStatus::SameTask => {}
            }

            let original_cgroup = resolve_cgroup_fs_path(cgroup_root, &record.original_cgroup)
                .with_context(|| {
                    format!(
                        "failed to resolve original cgroup path {}",
                        record.original_cgroup.display()
                    )
                })?;
            let cgroup_procs = original_cgroup.join("cgroup.procs");
            match write_trimmed(&cgroup_procs, &tid.to_string()) {
                Ok(()) => {
                    restored += 1;
                }
                Err(e) => match classify_restore_write_error(proc_root, tid, e) {
                    RestoreWriteError::MissingTask => {
                        skipped_dead += 1;
                        log::debug!("cgroup restore skipped: task tid={} is missing/dead", tid);
                    }
                    RestoreWriteError::PermissionDenied(e)
                    | RestoreWriteError::InvalidValue(e)
                    | RestoreWriteError::Io(e) => {
                        errors += 1;
                        let msg = format!(
                            "failed to restore pid={} to cgroup {}: {}",
                            tid,
                            original_cgroup.display(),
                            e
                        );
                        log::error!("{}", msg);
                        messages.push(msg);
                    }
                },
            }
        }

        if let Some(cpuset) = cpuset {
            match restore_cpuset_record(cgroup_root, cpuset) {
                Ok(restored_files) => {
                    restored += restored_files;
                }
                Err(err) => {
                    errors += 1;
                    let msg = format!("failed to restore cgroup cpuset state: {err:#}");
                    log::error!("{}", msg);
                    messages.push(msg);
                }
            }
        }

        if errors > 0 {
            anyhow::bail!(
                "failed to rollback cgroup placement: {}",
                messages.join("; ")
            );
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

pub(super) fn cgroup_partial_token(
    records: Vec<CgroupRestoreRecord>,
    cpuset_changed: bool,
    cpuset: &Option<CgroupCpusetRestoreRecord>,
) -> Option<RollbackToken> {
    if records.is_empty() && !cpuset_changed {
        None
    } else {
        Some(RollbackToken::CgroupRestore {
            records,
            cpuset: if cpuset_changed { cpuset.clone() } else { None },
        })
    }
}

pub(super) fn restore_cpuset_record(
    cgroup_root: &Path,
    record: &CgroupCpusetRestoreRecord,
) -> anyhow::Result<usize> {
    let cgroup_path =
        resolve_cgroup_fs_path(cgroup_root, &record.cgroup_path).with_context(|| {
            format!(
                "failed to resolve cgroup cpuset restore path {}",
                record.cgroup_path.display()
            )
        })?;

    let mut restored = 0usize;
    if let Some(original) = &record.original_cpuset_cpus {
        write_trimmed(&cgroup_path.join("cpuset.cpus"), original).with_context(|| {
            format!(
                "failed to restore {}",
                cgroup_path.join("cpuset.cpus").display()
            )
        })?;
        restored += 1;
    }

    if let Some(original) = &record.original_cpuset_mems {
        write_trimmed(&cgroup_path.join("cpuset.mems"), original).with_context(|| {
            format!(
                "failed to restore {}",
                cgroup_path.join("cpuset.mems").display()
            )
        })?;
        restored += 1;
    }

    Ok(restored)
}

#[cfg(test)]
use std::io;

#[cfg(test)]
pub(super) fn is_dead_task_io_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause.downcast_ref::<io::Error>().is_some_and(|io_err| {
            io_err.kind() == io::ErrorKind::NotFound || io_err.raw_os_error() == Some(libc::ESRCH)
        })
    })
}
