use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use stutter_core::ids::Tid;

use super::{
    Profile, ProfileRule,
    action_decision::{ActionStatus, decide_profile_task_actions},
    matching::matching_profile_rule,
    summary::ProfileApplySummary,
};
use crate::{
    actions::TaskIdentity,
    affinity::{self, AffinityRecord, CpuMask},
    process_tree::{TaskInfo, TaskMap},
    profile_restore::{IoPrioRestoreRecordV2, NiceRestoreRecordV2},
};

#[derive(Default)]
pub struct ProfileApplyCache {
    pub(super) known_correct: BTreeSet<ProfileApplyCacheKey>,
}

impl ProfileApplyCache {
    pub fn clear(&mut self) {
        self.known_correct.clear();
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct ProfileApplyCacheKey {
    tid: Tid,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
    desired_affinity: Option<CpuMask>,
    desired_nice: Option<i32>,
    desired_ionice: Option<crate::actions::ioprio::IoPrioValue>,
}

#[derive(Clone, Debug)]
pub struct PlannedAffinityChange {
    pub record: AffinityRecord,
}

#[derive(Clone, Debug, Default)]
pub struct ProfileApplyPlan {
    pub affinity_changes: Vec<PlannedAffinityChange>,
    pub nice_groups: BTreeMap<i32, Vec<TaskIdentity>>,
    pub ionice_groups: BTreeMap<crate::actions::ioprio::IoPrioValue, Vec<TaskIdentity>>,
    pub summary: ProfileApplySummary,
    pub(super) nice_records: Vec<NiceRestoreRecordV2>,
    pub(super) ionice_records: Vec<IoPrioRestoreRecordV2>,
    pub(super) cache_keys: Vec<ProfileApplyCacheKey>,
}

impl ProfileApplyPlan {
    pub fn is_empty(&self) -> bool {
        self.affinity_changes.is_empty()
            && self.nice_groups.is_empty()
            && self.ionice_groups.is_empty()
    }

    pub(super) fn affinity_records(&self) -> Vec<AffinityRecord> {
        self.affinity_changes
            .iter()
            .map(|planned| planned.record.clone())
            .collect()
    }
}

pub(super) fn planned_profile_apply(
    tasks: &TaskMap,
    profile: &Profile,
    cache: Option<&mut ProfileApplyCache>,
) -> anyhow::Result<ProfileApplyPlan> {
    planned_profile_apply_with_readers(
        tasks,
        profile,
        cache,
        affinity::read_allowed_mask,
        |tid| crate::actions::nice::read_task_nice(tid.as_u32()),
        |tid| crate::actions::ioprio::read_task_ioprio(tid.as_u32()),
    )
}

pub(crate) fn planned_profile_apply_with_readers<FA, FN, FI>(
    tasks: &TaskMap,
    profile: &Profile,
    mut cache: Option<&mut ProfileApplyCache>,
    mut read_allowed_mask: FA,
    mut read_nice: FN,
    mut read_ioprio: FI,
) -> anyhow::Result<ProfileApplyPlan>
where
    FA: FnMut(Tid) -> io::Result<CpuMask>,
    FN: FnMut(Tid) -> anyhow::Result<i32>,
    FI: FnMut(Tid) -> anyhow::Result<i32>,
{
    let mut plan = ProfileApplyPlan::default();
    let mut seen_cache_keys = BTreeSet::new();
    let mut pending_tids = BTreeSet::<Tid>::new();

    for task in tasks.values() {
        let Some(rule) = matching_profile_rule(task, profile) else {
            continue;
        };

        plan.summary.checked_tasks += 1;

        let cache_key = ProfileApplyCacheKey::new(task, rule);
        seen_cache_keys.insert(cache_key.clone());

        if cache
            .as_ref()
            .is_some_and(|cache| cache.known_correct.contains(&cache_key))
        {
            continue;
        }

        let action_decision = decide_profile_task_actions(
            task,
            rule,
            &mut read_allowed_mask,
            &mut read_nice,
            &mut read_ioprio,
        )?;
        if action_decision.task_disappeared() {
            continue;
        }

        if let Some(decision) = &action_decision.affinity {
            match decision.status {
                ActionStatus::Pending => {
                    let Some(original_mask) = decision.current.clone() else {
                        anyhow::bail!(
                            "internal error: pending affinity decision for TID {} is missing current mask",
                            task.tid
                        );
                    };
                    let Some(applied_mask) = decision.desired.clone() else {
                        anyhow::bail!(
                            "internal error: pending affinity decision for TID {} is missing desired mask",
                            task.tid
                        );
                    };
                    plan.summary.pending_affinity += 1;
                    pending_tids.insert(task.task_id());
                    plan.affinity_changes.push(PlannedAffinityChange {
                        record: AffinityRecord {
                            tid: task.task_id(),
                            process_pid: Some(task.process_id()),
                            process_starttime_ticks: task.process_starttime_ticks,
                            task_starttime_ticks: task.task_starttime_ticks,
                            original_mask,
                            applied_mask,
                        },
                    });
                }
                ActionStatus::SkippedIncompleteIdentity => {
                    log::warn!(
                        "profile_skip_incomplete_identity tid={} comm={} process_pid={}",
                        task.tid,
                        task.comm,
                        task.process_pid
                    );
                }
                _ => {}
            }
        }

        if let Some(decision) = &action_decision.nice {
            match decision.status {
                ActionStatus::Pending => {
                    let Some(desired_nice) = decision.desired else {
                        anyhow::bail!(
                            "internal error: pending nice decision for TID {} is missing desired nice",
                            task.tid
                        );
                    };
                    let Some(original_nice) = decision.current else {
                        anyhow::bail!(
                            "internal error: pending nice decision for TID {} is missing current nice",
                            task.tid
                        );
                    };
                    plan.summary.pending_nice += 1;
                    pending_tids.insert(task.task_id());
                    plan.nice_groups
                        .entry(desired_nice)
                        .or_default()
                        .push(TaskIdentity::from_task_info(task));
                    plan.nice_records.push(NiceRestoreRecordV2 {
                        tid: task.task_id(),
                        process_pid: Some(task.process_id()),
                        process_starttime_ticks: task.process_starttime_ticks,
                        task_starttime_ticks: task.task_starttime_ticks,
                        comm: Some(task.comm.clone()),
                        original_nice,
                        applied_nice: desired_nice,
                    });
                }
                ActionStatus::SkippedIncompleteIdentity => {
                    log::warn!(
                        "profile_skip_priority_incomplete_identity tid={} comm={} process_pid={}",
                        task.tid,
                        task.comm,
                        task.process_pid
                    );
                }
                _ => {}
            }
        }

        if let Some(decision) = &action_decision.ionice {
            match decision.status {
                ActionStatus::Pending => {
                    let Some(desired_ioprio) = decision.desired else {
                        anyhow::bail!(
                            "internal error: pending ionice decision for TID {} is missing desired I/O priority",
                            task.tid
                        );
                    };
                    let Some(desired_encoded) = decision.desired_encoded else {
                        anyhow::bail!(
                            "internal error: pending ionice decision for TID {} is missing encoded desired I/O priority",
                            task.tid
                        );
                    };
                    let Some(original_ioprio) = decision.current_encoded else {
                        anyhow::bail!(
                            "internal error: pending ionice decision for TID {} is missing current I/O priority",
                            task.tid
                        );
                    };
                    plan.summary.pending_ionice += 1;
                    pending_tids.insert(task.task_id());
                    plan.ionice_groups
                        .entry(desired_ioprio)
                        .or_default()
                        .push(TaskIdentity::from_task_info(task));
                    plan.ionice_records.push(IoPrioRestoreRecordV2 {
                        tid: task.task_id(),
                        process_pid: Some(task.process_id()),
                        process_starttime_ticks: task.process_starttime_ticks,
                        task_starttime_ticks: task.task_starttime_ticks,
                        comm: Some(task.comm.clone()),
                        original_ioprio,
                        applied_ioprio: desired_encoded,
                    });
                }
                ActionStatus::SkippedIncompleteIdentity => {
                    log::warn!(
                        "profile_skip_priority_incomplete_identity tid={} comm={} process_pid={}",
                        task.tid,
                        task.comm,
                        task.process_pid
                    );
                }
                _ => {}
            }
        }

        if action_decision.pending() {
            plan.cache_keys.push(cache_key);
        } else if let Some(cache) = cache.as_mut() {
            cache.known_correct.insert(cache_key);
        }
    }

    plan.summary.pending_changes = pending_tids.len();

    if let Some(cache) = cache.as_mut() {
        cache
            .known_correct
            .retain(|cache_key| seen_cache_keys.contains(cache_key));
    }

    Ok(plan)
}

impl ProfileApplyCacheKey {
    fn new(task: &TaskInfo, rule: &ProfileRule) -> Self {
        Self {
            tid: task.task_id(),
            process_starttime_ticks: task.process_starttime_ticks,
            task_starttime_ticks: task.task_starttime_ticks,
            desired_affinity: rule.affinity.clone(),
            desired_nice: rule.nice,
            desired_ionice: rule.ionice,
        }
    }
}
