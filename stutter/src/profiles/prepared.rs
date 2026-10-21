//! An owned affinity plan whose durable restore snapshot precedes commit.
use std::path::{Path, PathBuf};

use anyhow::{Context, ensure};

use super::{
    Profile,
    plan::{ProfileApplyPlan, planned_profile_apply},
    summary::ProfileApplyResult,
};
use crate::{affinity, process_tree, profile_restore};

pub(crate) struct PreparedAffinityProfile {
    plan: ProfileApplyPlan,
    restore_path: PathBuf,
}

pub(crate) struct AffinityCommitFailure {
    pub source: anyhow::Error,
    pub mutation_started: bool,
}

pub(crate) fn prepare_affinity_profile(
    tree_pid: u32,
    profile: &Profile,
    restore_path: &Path,
    force: bool,
) -> anyhow::Result<PreparedAffinityProfile> {
    ensure!(
        !super::profile_uses_priority_actions(profile),
        "prepared affinity transaction does not authorize priority mutation"
    );
    let snapshot = process_tree::target_snapshot(
        process_tree::TargetSnapshotInput::default().tree_pids(&[tree_pid]),
    );
    let plan = planned_profile_apply(&snapshot.tasks, profile, None)?;
    ensure!(
        !plan.is_empty(),
        "prepared affinity transaction has no effective changes"
    );
    validate_plan(&plan)?;
    profile_restore::save_merged_restore_state(
        restore_path,
        &plan.affinity_records(),
        &[],
        &[],
        force,
    )?;
    Ok(PreparedAffinityProfile {
        plan,
        restore_path: restore_path.to_owned(),
    })
}

impl PreparedAffinityProfile {
    pub(crate) fn restore_path(&self) -> &Path {
        &self.restore_path
    }
    pub(crate) fn affected_tasks(&self) -> usize {
        self.plan.summary.pending_changes
    }

    /// Consuming the exact prepared plan prevents silent replanning after the
    /// controller has durably referenced its original identities and masks.
    pub(crate) fn commit(self) -> Result<ProfileApplyResult, AffinityCommitFailure> {
        let mut mutation_started = false;
        let result = (|| -> anyhow::Result<ProfileApplyResult> {
            validate_plan(&self.plan)?;
            let mut result = ProfileApplyResult {
                summary: self.plan.summary.clone(),
                ..Default::default()
            };
            for change in &self.plan.affinity_changes {
                if !validate_record(&change.record)? {
                    continue;
                }
                mutation_started = true;
                match affinity::set_affinity(change.record.tid, &change.record.applied_mask) {
                    Ok(()) => result.affinity_records.push(change.record.clone()),
                    Err(error) if error.raw_os_error() == Some(libc::ESRCH) => {}
                    Err(error) => return Err(error).context("prepared affinity commit failed"),
                }
            }
            result.affinity_records.sort_by_key(|record| record.tid);
            Ok(result)
        })();
        result.map_err(|source| AffinityCommitFailure {
            source,
            mutation_started,
        })
    }
}

fn validate_plan(plan: &ProfileApplyPlan) -> anyhow::Result<()> {
    for change in &plan.affinity_changes {
        validate_record(&change.record)?;
    }
    Ok(())
}

fn validate_record(record: &affinity::AffinityRecord) -> anyhow::Result<bool> {
    let pid = record
        .process_pid
        .context("prepared affinity requires process identity")?;
    let process_ticks = record
        .process_starttime_ticks
        .filter(|ticks| *ticks > 0)
        .context("prepared affinity requires process start time")?;
    let task_ticks = record
        .task_starttime_ticks
        .filter(|ticks| *ticks > 0)
        .context("prepared affinity requires task start time")?;
    ensure!(
        pid.as_u32() > 0 && record.tid.as_u32() > 0,
        "prepared affinity requires nonzero IDs"
    );
    let Some(current_process) =
        process_tree::process_starttime_at(Path::new("/proc"), pid.as_u32())
    else {
        return Ok(false);
    };
    let Some(current_task) =
        process_tree::process_starttime_at(Path::new("/proc"), record.tid.as_u32())
    else {
        return Ok(false);
    };
    ensure!(
        current_process == process_ticks && current_task == task_ticks,
        "prepared affinity identity changed for TID {}",
        record.tid
    );
    match affinity::read_allowed_mask(record.tid) {
        Ok(mask) => ensure!(
            mask == record.original_mask,
            "prepared affinity original mask changed for TID {}",
            record.tid
        ),
        Err(error) if error.raw_os_error() == Some(libc::ESRCH) => return Ok(false),
        Err(error) => return Err(error).context("failed to revalidate prepared affinity mask"),
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_identity_is_rejected_before_any_commit_syscall() {
        let plan = ProfileApplyPlan {
            affinity_changes: vec![super::super::plan::PlannedAffinityChange {
                record: affinity::AffinityRecord {
                    tid: std::process::id().into(),
                    process_pid: Some(std::process::id().into()),
                    process_starttime_ticks: None,
                    task_starttime_ticks: None,
                    original_mask: affinity::CpuMask::parse("0").unwrap(),
                    applied_mask: affinity::CpuMask::parse("0").unwrap(),
                },
            }],
            ..Default::default()
        };
        let error = PreparedAffinityProfile {
            plan,
            restore_path: "unused".into(),
        }
        .commit()
        .err()
        .unwrap();
        assert!(!error.mutation_started);
        assert!(
            error
                .source
                .to_string()
                .contains("requires process start time")
        );
    }
}
