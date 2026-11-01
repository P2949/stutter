use std::{fs, io, path::Path};

use stutter_core::ids::Tid;

use super::{
    load::load_restore_state,
    model::{ProfileRestoreState, ProfileRestoreSummary},
    validate::{restore_priority_identity, restore_record_status},
};
use crate::{
    actions::disposition::RollbackDisposition,
    affinity::{self, RestoreRecordStatus},
};

pub fn restore_saved(path: &Path) -> anyhow::Result<ProfileRestoreSummary> {
    restore_saved_with_validation(path, |_| Ok(()))
}

pub(crate) fn restore_saved_with_validation(
    path: &Path,
    validate: impl FnOnce(&ProfileRestoreState) -> anyhow::Result<()>,
) -> anyhow::Result<ProfileRestoreSummary> {
    let state = load_restore_state(path)?;
    validate(&state)?;
    let (summary, errors) = restore_all(&state);

    if !errors.is_empty() {
        anyhow::bail!(
            "failed to restore {} profile record(s); restore file kept at {}",
            errors.len(),
            path.display()
        );
    }

    fs::remove_file(path).ok();
    Ok(summary)
}

pub fn restore_all(state: &ProfileRestoreState) -> (ProfileRestoreSummary, Vec<anyhow::Error>) {
    restore_all_at(Path::new("/proc"), state)
}

pub(crate) fn restore_all_at(
    proc_root: &Path,
    state: &ProfileRestoreState,
) -> (ProfileRestoreSummary, Vec<anyhow::Error>) {
    restore_all_at_with_ops(
        proc_root,
        state,
        affinity::set_affinity,
        affinity::read_allowed_mask,
        |tid, nice| {
            crate::actions::nice::set_task_nice(tid.as_u32(), nice).map_err(anyhow_to_io_error)
        },
        |tid, ioprio| {
            crate::actions::ioprio::set_task_ioprio(tid.as_u32(), ioprio)
                .map_err(anyhow_to_io_error)
        },
    )
}

pub(crate) fn restore_all_at_with_ops<FA, FM, FN, FI>(
    proc_root: &Path,
    state: &ProfileRestoreState,
    mut set_affinity: FA,
    mut current_mask: FM,
    mut set_nice: FN,
    mut set_ioprio: FI,
) -> (ProfileRestoreSummary, Vec<anyhow::Error>)
where
    FA: FnMut(Tid, &crate::affinity::CpuMask) -> io::Result<()>,
    FM: FnMut(Tid) -> io::Result<crate::affinity::CpuMask>,
    FN: FnMut(Tid, i32) -> io::Result<()>,
    FI: FnMut(Tid, i32) -> io::Result<()>,
{
    let mut summary = ProfileRestoreSummary::default();
    let mut errors = Vec::new();

    for record in &state.affinity_records {
        match restore_record_status(
            proc_root,
            record.tid,
            record.process_pid,
            record.process_starttime_ticks,
            record.task_starttime_ticks,
        ) {
            Ok(RestoreRecordStatus::Verified) => {}
            Ok(RestoreRecordStatus::LegacyUnverified) => {
                summary.legacy_unverified += 1;
                log::warn!(
                    "profile_restore_affinity_missing_identity tid={}; restoring by numeric TID only for legacy affinity record",
                    record.tid
                );
            }
            Ok(RestoreRecordStatus::Dead) => {
                summary.skipped_dead += 1;
                RollbackDisposition::record(|d| d.target_gone += 1);
                continue;
            }
            Ok(RestoreRecordStatus::IdentityMismatch) => {
                summary.skipped_identity_mismatch += 1;
                RollbackDisposition::record(|d| d.identity_superseded += 1);
                continue;
            }
            Err(err) => {
                summary.errors += 1;
                errors.push(err.into());
                continue;
            }
        }

        // Only restore a value Stutter still owns. A mask that is no longer the one Stutter
        // applied belongs to someone else now, and one that cannot be read is not known to be
        // Stutter's: neither is permission to write the old value back.
        match current_mask(record.tid) {
            Ok(live) if live != record.applied_mask => {
                summary.skipped_externally_changed += 1;
                RollbackDisposition::record(|d| d.superseded_externally += 1);
                log::warn!(
                    "profile_restore_affinity_externally_changed tid={} applied={} live={}; preserving the external value",
                    record.tid,
                    record.applied_mask.to_range_string(),
                    live.to_range_string()
                );
                continue;
            }
            Ok(_) => {}
            Err(err) if err.raw_os_error() == Some(libc::ESRCH) => {
                summary.skipped_dead += 1;
                RollbackDisposition::record(|d| d.target_gone += 1);
                continue;
            }
            Err(err) => {
                summary.errors += 1;
                RollbackDisposition::record(|d| d.unresolved += 1);
                errors.push(anyhow::anyhow!(
                    "cannot establish ownership of the affinity of TID {}: {err}",
                    record.tid
                ));
                continue;
            }
        }

        match set_affinity(record.tid, &record.original_mask) {
            Ok(()) => {
                summary.affinity += 1;
                RollbackDisposition::record(|d| d.restored += 1);
            }
            Err(err) if err.raw_os_error() == Some(libc::ESRCH) => summary.skipped_dead += 1,
            Err(err) => {
                summary.errors += 1;
                errors.push(anyhow::anyhow!(
                    "failed to restore affinity for TID {}: {err}",
                    record.tid
                ));
            }
        }
    }

    for record in &state.nice_records {
        if !restore_priority_identity(
            proc_root,
            record.tid,
            record.identity_tuple(),
            &mut summary,
            &mut errors,
        ) {
            continue;
        }

        match set_nice(record.tid, record.original_nice) {
            Ok(()) => summary.nice += 1,
            Err(err) if err.raw_os_error() == Some(libc::ESRCH) => summary.skipped_dead += 1,
            Err(err) => {
                summary.errors += 1;
                errors.push(anyhow::anyhow!(
                    "failed to restore nice={} for TID {}: {err}",
                    record.original_nice,
                    record.tid
                ));
            }
        }
    }

    for record in &state.ionice_records {
        if !restore_priority_identity(
            proc_root,
            record.tid,
            record.identity_tuple(),
            &mut summary,
            &mut errors,
        ) {
            continue;
        }

        match set_ioprio(record.tid, record.original_ioprio) {
            Ok(()) => summary.ionice += 1,
            Err(err) if err.raw_os_error() == Some(libc::ESRCH) => summary.skipped_dead += 1,
            Err(err) => {
                summary.errors += 1;
                errors.push(anyhow::anyhow!(
                    "failed to restore I/O priority={} for TID {}: {err}",
                    record.original_ioprio,
                    record.tid
                ));
            }
        }
    }

    (summary, errors)
}

fn anyhow_to_io_error(err: anyhow::Error) -> io::Error {
    if let Some(io_err) = err.downcast_ref::<io::Error>() {
        if let Some(raw) = io_err.raw_os_error() {
            return io::Error::from_raw_os_error(raw);
        }
        return io::Error::new(io_err.kind(), io_err.to_string());
    }
    io::Error::other(err.to_string())
}
