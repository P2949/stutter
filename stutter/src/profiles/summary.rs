use std::{collections::BTreeSet, io};

use stutter_core::ids::Tid;

use super::{Profile, matching::matching_profile_rule, plan::anyhow_raw_os_error};
use crate::{
    affinity::{self, AffinityRecord, CpuMask},
    process_tree::{self, TaskMap},
    profile_restore::{IoPrioRestoreRecordV2, NiceRestoreRecordV2},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileApplySummary {
    pub checked_tasks: usize,
    pub pending_changes: usize,
    pub pending_affinity: usize,
    pub pending_nice: usize,
    pub pending_ionice: usize,
}

#[derive(Clone, Debug, Default)]
pub struct ProfileApplyResult {
    pub affinity_records: Vec<AffinityRecord>,
    pub nice_records: Vec<NiceRestoreRecordV2>,
    pub ionice_records: Vec<IoPrioRestoreRecordV2>,
    pub summary: ProfileApplySummary,
}

impl ProfileApplyResult {
    pub fn affected_tasks(&self) -> usize {
        self.summary.pending_changes
    }
}

pub fn profile_apply_summary_for_tree(
    tree_pid: u32,
    profile: &Profile,
) -> anyhow::Result<ProfileApplySummary> {
    let snapshot = process_tree::target_snapshot(
        process_tree::TargetSnapshotInput::default().tree_pids(&[tree_pid]),
    );

    profile_apply_summary(&snapshot.tasks, profile)
}

fn profile_apply_summary(
    tasks: &TaskMap,
    profile: &Profile,
) -> anyhow::Result<ProfileApplySummary> {
    profile_apply_summary_with_readers(
        tasks,
        profile,
        affinity::read_allowed_mask,
        |tid| crate::actions::nice::read_task_nice(tid.as_u32()),
        |tid| crate::actions::ioprio::read_task_ioprio(tid.as_u32()),
    )
}

#[cfg(test)]
pub(crate) fn profile_apply_summary_with_reader<F>(
    tasks: &TaskMap,
    profile: &Profile,
    read_allowed_mask: F,
) -> anyhow::Result<ProfileApplySummary>
where
    F: FnMut(u32) -> io::Result<CpuMask>,
{
    let mut read_allowed_mask = read_allowed_mask;
    profile_apply_summary_with_readers(
        tasks,
        profile,
        |tid| read_allowed_mask(tid.as_u32()),
        |_| Ok(0),
        |_| Ok(0),
    )
}

pub(crate) fn profile_apply_summary_with_readers<FA, FN, FI>(
    tasks: &TaskMap,
    profile: &Profile,
    mut read_allowed_mask: FA,
    mut read_nice: FN,
    mut read_ioprio: FI,
) -> anyhow::Result<ProfileApplySummary>
where
    FA: FnMut(Tid) -> io::Result<CpuMask>,
    FN: FnMut(Tid) -> anyhow::Result<i32>,
    FI: FnMut(Tid) -> anyhow::Result<i32>,
{
    let mut summary = ProfileApplySummary::default();
    let mut pending_tids = BTreeSet::<Tid>::new();

    for task in tasks.values() {
        let Some(rule) = matching_profile_rule(task, profile) else {
            continue;
        };

        summary.checked_tasks += 1;

        if let Some(desired_mask) = &rule.affinity {
            let original_mask = match read_allowed_mask(task.task_id()) {
                Ok(mask) => mask,
                Err(err) if err.raw_os_error() == Some(libc::ESRCH) => {
                    continue;
                }
                Err(err) => {
                    return Err(anyhow::anyhow!(
                        "failed to read CPU affinity for TID {}: {err}",
                        task.tid
                    ));
                }
            };

            if original_mask != *desired_mask {
                summary.pending_affinity += 1;
                pending_tids.insert(task.task_id());
            }
        }

        if let Some(desired_nice) = rule.nice {
            let original_nice = match read_nice(task.task_id()) {
                Ok(nice) => nice,
                Err(err) if anyhow_raw_os_error(&err) == Some(libc::ESRCH) => continue,
                Err(err) => return Err(err),
            };

            if original_nice != desired_nice {
                summary.pending_nice += 1;
                pending_tids.insert(task.task_id());
            }
        }

        if let Some(desired_ioprio) = rule.ionice {
            let original_ioprio = match read_ioprio(task.task_id()) {
                Ok(ioprio) => ioprio,
                Err(err) if anyhow_raw_os_error(&err) == Some(libc::ESRCH) => continue,
                Err(err) => return Err(err),
            };

            if original_ioprio != desired_ioprio.encode()? {
                summary.pending_ionice += 1;
                pending_tids.insert(task.task_id());
            }
        }
    }

    summary.pending_changes = pending_tids.len();
    Ok(summary)
}
