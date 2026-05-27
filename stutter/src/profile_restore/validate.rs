use std::{io, path::Path};

use stutter_core::ids::{Pid, Tid};

use super::model::{ProfileRestoreSummary, RestoreIdentity};
use crate::affinity::{self, RestoreRecordStatus};

pub(crate) fn restore_priority_identity(
    proc_root: &Path,
    tid: Tid,
    identity: RestoreIdentity,
    summary: &mut ProfileRestoreSummary,
    errors: &mut Vec<anyhow::Error>,
) -> bool {
    match restore_record_status(
        proc_root,
        tid,
        identity.process_pid,
        identity.process_starttime_ticks,
        identity.task_starttime_ticks,
    ) {
        Ok(RestoreRecordStatus::Verified) => true,
        Ok(RestoreRecordStatus::Dead) => {
            summary.skipped_dead += 1;
            false
        }
        Ok(RestoreRecordStatus::IdentityMismatch | RestoreRecordStatus::LegacyUnverified) => {
            summary.skipped_identity_mismatch += 1;
            false
        }
        Err(err) => {
            summary.errors += 1;
            errors.push(err.into());
            false
        }
    }
}

pub(crate) fn restore_record_status(
    proc_root: &Path,
    tid: Tid,
    process_pid: Option<Pid>,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
) -> io::Result<RestoreRecordStatus> {
    affinity::restore_identity_status_at(
        proc_root,
        tid,
        process_pid,
        process_starttime_ticks,
        task_starttime_ticks,
    )
}
