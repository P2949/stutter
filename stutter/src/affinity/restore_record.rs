use std::{fs, io, path::Path};

use serde::{Deserialize, Serialize};
use stutter_core::ids::{Pid, Tid};

use super::{CpuMask, syscall::set_affinity};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AffinityRecord {
    pub tid: Tid,
    #[serde(default)]
    pub process_pid: Option<Pid>,
    #[serde(default)]
    pub process_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub task_starttime_ticks: Option<u64>,
    pub original_mask: CpuMask,
    pub applied_mask: CpuMask,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RestoreState {
    pub schema_version: u32,
    pub records: Vec<AffinityRecord>,
}

#[cfg(test)]
pub(super) const RESTORE_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Default, Eq, PartialEq)]
pub struct RestoreSummary {
    pub restored: usize,
    pub skipped_dead: usize,
    pub skipped_identity_mismatch: usize,
    pub legacy_unverified: usize,
    pub errors: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreRecordStatus {
    Verified,
    LegacyUnverified,
    Dead,
    IdentityMismatch,
}

pub fn restore_all(records: &[AffinityRecord]) -> (RestoreSummary, Vec<anyhow::Error>) {
    restore_all_at(Path::new("/proc"), records)
}

fn restore_all_at(
    proc_root: &Path,
    records: &[AffinityRecord],
) -> (RestoreSummary, Vec<anyhow::Error>) {
    let mut summary = RestoreSummary::default();
    let mut errors = Vec::new();

    for record in records {
        match restore_record_status_at(proc_root, record) {
            Ok(RestoreRecordStatus::Verified) => {}
            Ok(RestoreRecordStatus::LegacyUnverified) => {
                summary.legacy_unverified += 1;
                log::warn!(
                    "restore_record_missing_identity tid={}; restoring by numeric TID only for legacy restore file",
                    record.tid
                );
            }
            Ok(RestoreRecordStatus::Dead) => {
                summary.skipped_dead += 1;
                continue;
            }
            Ok(RestoreRecordStatus::IdentityMismatch) => {
                summary.skipped_identity_mismatch += 1;
                log::warn!(
                    "restore_record_identity_mismatch tid={}; skipping affinity restore to avoid TID reuse damage",
                    record.tid
                );
                continue;
            }
            Err(err) => {
                summary.errors += 1;
                errors.push(err.into());
                continue;
            }
        }

        match set_affinity(record.tid, &record.original_mask) {
            Ok(()) => summary.restored += 1,
            Err(err) if err.raw_os_error() == Some(libc::ESRCH) => {
                summary.skipped_dead += 1;
            }
            Err(err) => {
                summary.errors += 1;
                errors.push(affinity_set_error(record.tid, err));
            }
        }
    }

    (summary, errors)
}

pub(super) fn restore_record_status_at(
    proc_root: &Path,
    record: &AffinityRecord,
) -> io::Result<RestoreRecordStatus> {
    restore_identity_status_at(
        proc_root,
        record.tid,
        record.process_pid,
        record.process_starttime_ticks,
        record.task_starttime_ticks,
    )
}

pub(crate) fn restore_identity_status_at(
    proc_root: &Path,
    tid: Tid,
    process_pid: Option<Pid>,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
) -> io::Result<RestoreRecordStatus> {
    if process_pid.is_none() && process_starttime_ticks.is_none() && task_starttime_ticks.is_none()
    {
        return Ok(RestoreRecordStatus::LegacyUnverified);
    }

    let (Some(process_pid), Some(process_starttime_ticks), Some(task_starttime_ticks)) =
        (process_pid, process_starttime_ticks, task_starttime_ticks)
    else {
        return Ok(RestoreRecordStatus::IdentityMismatch);
    };

    let process_stat_path = proc_root.join(process_pid.to_string()).join("stat");
    let process_starttime = match stat_starttime_at(&process_stat_path) {
        Ok(starttime) => starttime,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(RestoreRecordStatus::Dead),
        Err(err) => {
            return Err(io::Error::new(
                err.kind(),
                format!(
                    "failed to read process identity for TID {} via {}: {err}",
                    tid,
                    process_stat_path.display()
                ),
            ));
        }
    };
    if process_starttime != Some(process_starttime_ticks) {
        return Ok(RestoreRecordStatus::IdentityMismatch);
    }

    let task_stat_path = proc_root
        .join(process_pid.to_string())
        .join("task")
        .join(tid.to_string())
        .join("stat");
    let task_starttime = match stat_starttime_at(&task_stat_path) {
        Ok(starttime) => starttime,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(RestoreRecordStatus::Dead),
        Err(err) => {
            return Err(io::Error::new(
                err.kind(),
                format!(
                    "failed to read task identity for TID {} via {}: {err}",
                    tid,
                    task_stat_path.display()
                ),
            ));
        }
    };
    if task_starttime != Some(task_starttime_ticks) {
        return Ok(RestoreRecordStatus::IdentityMismatch);
    }

    Ok(RestoreRecordStatus::Verified)
}

#[cfg(test)]
impl AffinityRecord {
    pub(super) fn has_identity(&self) -> bool {
        self.process_pid.is_some()
            || self.process_starttime_ticks.is_some()
            || self.task_starttime_ticks.is_some()
    }
}

fn stat_starttime_at(path: &Path) -> io::Result<Option<u64>> {
    let stat = fs::read_to_string(path)?;
    Ok(crate::process_tree::parse_proc_stat_starttime(&stat))
}

fn affinity_set_error(tid: Tid, err: io::Error) -> anyhow::Error {
    anyhow::anyhow!("failed to set CPU affinity for TID {tid}: {err}")
}
