use super::*;

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
            let status = verify_task_identity(Path::new("/proc"), &identity);
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

            let original_cgroup = if record.original_cgroup.is_absolute() {
                record.original_cgroup.clone()
            } else {
                Path::new("/sys/fs/cgroup")
                    .join(strip_cgroup_leading_slash(&record.original_cgroup))
            };
            let cgroup_procs = original_cgroup.join("cgroup.procs");
            match write_trimmed(&cgroup_procs, &tid.to_string()) {
                Ok(()) => {
                    restored += 1;
                }
                Err(e) => match classify_restore_write_error(Path::new("/proc"), tid, e) {
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
            match restore_cpuset_record(Path::new("/sys/fs/cgroup"), cpuset) {
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
    let cgroup_path = if record.cgroup_path.starts_with(cgroup_root) {
        record.cgroup_path.clone()
    } else {
        cgroup_root.join(strip_cgroup_leading_slash(&record.cgroup_path))
    };

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
pub(super) fn is_dead_task_io_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause.downcast_ref::<io::Error>().is_some_and(|io_err| {
            io_err.kind() == io::ErrorKind::NotFound || io_err.raw_os_error() == Some(libc::ESRCH)
        })
    })
}
