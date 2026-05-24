use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use anyhow::Context;

use crate::{
    actions::{
        TaskIdentity, TuningAction,
        ioprio::{IoPrioAction, IoPrioPolicy, IoPrioValue},
        nice::{NiceAction, NicePolicy},
        runner::{ActionRunPolicy, run_audited_action},
    },
    affinity::{self, AffinityRecord, CpuMask},
    process_tree::{self, CompiledPattern, TaskClass, TaskInfo},
    profile_restore::{self, IoPrioRestoreRecordV2, NiceRestoreRecordV2},
};

pub(crate) mod parse;
pub(crate) mod render;

pub(crate) mod warnings;

#[cfg(test)]
pub(crate) use parse::parse_profiles;
pub use parse::{load_first_profile, load_profiles};
pub use render::{generate_topology_template, render_profiles_toml};
use warnings::warn_profile_offline_cpus;
#[cfg(test)]
pub(crate) use warnings::{profile_offline_cpu_warnings, profile_rule_overlap_warnings};

#[derive(Clone, Debug)]
pub struct Profile {
    pub name: String,
    pub rules: Vec<ProfileRule>,
}

#[derive(Clone, Debug)]
pub struct ProfileRule {
    pub affinity: Option<CpuMask>,
    pub nice: Option<i32>,
    pub ionice: Option<IoPrioValue>,
    pub match_class: Vec<TaskClass>,
    pub match_comm: Vec<CompiledPattern>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileApplySummary {
    pub checked_tasks: usize,
    pub pending_changes: usize,
    pub pending_affinity: usize,
    pub pending_nice: usize,
    pub pending_ionice: usize,
}

#[derive(Default)]
pub struct ProfileApplyCache {
    known_correct: BTreeSet<ProfileApplyCacheKey>,
}

impl ProfileApplyCache {
    pub fn clear(&mut self) {
        self.known_correct.clear();
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ProfileApplyCacheKey {
    tid: u32,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
    desired_affinity: Option<CpuMask>,
    desired_nice: Option<i32>,
    desired_ionice: Option<IoPrioValue>,
}

#[derive(Clone, Debug)]
pub struct PlannedAffinityChange {
    pub record: AffinityRecord,
}

pub struct ProfileEvaluationInput<'a> {
    pub profile: &'a Profile,
    pub active_tasks: &'a [crate::autotune::observation::ActiveTaskSnapshot],
    pub topology: Option<&'a crate::topology::TopologyModel>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileTaskPlan {
    pub tid: u32,
    pub process_pid: u32,
    pub comm: String,
    pub class: TaskClass,
    pub requested_mask: String,
    pub matched_rule_index: usize,
    pub matched_rule_name: Option<String>,
}

pub fn evaluate_profile_for_tasks(input: ProfileEvaluationInput<'_>) -> Vec<ProfileTaskPlan> {
    let _topology = input.topology;

    input
        .active_tasks
        .iter()
        .filter_map(|task| {
            let info = task_info_from_active_snapshot(task);
            let (rule_index, rule) = matching_profile_rule_with_index(&info, input.profile)?;
            let requested_mask = rule.affinity.as_ref()?.to_range_string();

            Some(ProfileTaskPlan {
                tid: task.tid,
                process_pid: task.process_pid,
                comm: task.comm.clone(),
                class: task.class,
                requested_mask,
                matched_rule_index: rule_index,
                matched_rule_name: None,
            })
        })
        .collect()
}

#[derive(Clone, Debug, Default)]
pub struct ProfileApplyPlan {
    pub affinity_changes: Vec<PlannedAffinityChange>,
    pub nice_groups: BTreeMap<i32, Vec<TaskIdentity>>,
    pub ionice_groups: BTreeMap<IoPrioValue, Vec<TaskIdentity>>,
    pub summary: ProfileApplySummary,
    nice_records: Vec<NiceRestoreRecordV2>,
    ionice_records: Vec<IoPrioRestoreRecordV2>,
    cache_keys: Vec<ProfileApplyCacheKey>,
}

impl ProfileApplyPlan {
    pub fn is_empty(&self) -> bool {
        self.affinity_changes.is_empty()
            && self.nice_groups.is_empty()
            && self.ionice_groups.is_empty()
    }

    fn affinity_records(&self) -> Vec<AffinityRecord> {
        self.affinity_changes
            .iter()
            .map(|planned| planned.record.clone())
            .collect()
    }
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

pub fn profile_uses_priority_actions(profile: &Profile) -> bool {
    profile
        .rules
        .iter()
        .any(|rule| rule.nice.is_some() || rule.ionice.is_some())
}

pub fn apply_profile_to_tree(
    tree_pid: u32,
    profile: &Profile,
    force_restore_overwrite: bool,
    dry_run: bool,
) -> anyhow::Result<Vec<AffinityRecord>> {
    apply_managed_profile_to_tree_with_cache(
        tree_pid,
        profile,
        force_restore_overwrite,
        dry_run,
        None,
    )
    .map(|result| result.affinity_records)
}

pub fn apply_managed_profile_to_tree(
    tree_pid: u32,
    profile: &Profile,
    force_restore_overwrite: bool,
    dry_run: bool,
) -> anyhow::Result<ProfileApplyResult> {
    apply_managed_profile_to_tree_with_cache(
        tree_pid,
        profile,
        force_restore_overwrite,
        dry_run,
        None,
    )
}

pub fn apply_managed_profile_to_tree_cached(
    tree_pid: u32,
    profile: &Profile,
    force_restore_overwrite: bool,
    dry_run: bool,
    cache: &mut ProfileApplyCache,
) -> anyhow::Result<ProfileApplyResult> {
    apply_managed_profile_to_tree_with_cache(
        tree_pid,
        profile,
        force_restore_overwrite,
        dry_run,
        Some(cache),
    )
}

fn apply_managed_profile_to_tree_with_cache(
    tree_pid: u32,
    profile: &Profile,
    force_restore_overwrite: bool,
    dry_run: bool,
    mut cache: Option<&mut ProfileApplyCache>,
) -> anyhow::Result<ProfileApplyResult> {
    let snapshot = process_tree::target_snapshot(
        process_tree::TargetSnapshotInput::default().tree_pids(&[tree_pid]),
    );

    if let Err(err) = warn_profile_offline_cpus(profile) {
        log::warn!("profile_online_cpu_check_failed err={err:#}");
    }

    let planned = planned_profile_apply(&snapshot.tasks, profile, cache.as_deref_mut())?;
    if planned.is_empty() {
        return Ok(ProfileApplyResult {
            summary: planned.summary,
            ..ProfileApplyResult::default()
        });
    }

    if dry_run {
        for planned in &planned.affinity_changes {
            log::info!(
                "dry_run: would apply mask {} to TID {}",
                planned.record.applied_mask.to_range_string(),
                planned.record.tid
            );
        }
        return Ok(result_from_plan(&planned));
    }

    preflight_profile_plan(&planned)?;

    let restore_path = profile_restore::default_restore_path();
    let affinity_records = planned.affinity_records();
    profile_restore::save_merged_restore_state(
        &restore_path,
        &affinity_records,
        &planned.nice_records,
        &planned.ionice_records,
        force_restore_overwrite,
    )?;

    let mut result = result_from_plan(&planned);
    result.affinity_records.clear();

    if let Err(err) = apply_profile_plan(&planned, &mut result) {
        if let Err(restore_err) = profile_restore::restore_saved(&restore_path) {
            anyhow::bail!(
                "profile apply failed: {err:#}; emergency restore failed: {restore_err:#}"
            );
        }
        anyhow::bail!("profile apply failed; restore completed: {err:#}");
    }

    if let Err(err) = verify_affinity_plan(&planned) {
        if let Err(restore_err) = profile_restore::restore_saved(&restore_path) {
            anyhow::bail!(
                "profile verify failed: {err:#}; emergency restore failed: {restore_err:#}"
            );
        }
        anyhow::bail!("profile verify failed; restore completed: {err:#}");
    }

    if let Some(cache) = cache {
        for cache_key in &planned.cache_keys {
            cache.known_correct.insert(cache_key.clone());
        }
    }

    result.affinity_records.sort_by_key(|record| record.tid);
    Ok(result)
}

fn planned_profile_apply(
    tasks: &BTreeMap<u32, TaskInfo>,
    profile: &Profile,
    cache: Option<&mut ProfileApplyCache>,
) -> anyhow::Result<ProfileApplyPlan> {
    planned_profile_apply_with_readers(
        tasks,
        profile,
        cache,
        affinity::read_allowed_mask_raw,
        crate::actions::nice::read_task_nice,
        crate::actions::ioprio::read_task_ioprio,
    )
}

fn planned_profile_apply_with_readers<FA, FN, FI>(
    tasks: &BTreeMap<u32, TaskInfo>,
    profile: &Profile,
    mut cache: Option<&mut ProfileApplyCache>,
    mut read_allowed_mask: FA,
    mut read_nice: FN,
    mut read_ioprio: FI,
) -> anyhow::Result<ProfileApplyPlan>
where
    FA: FnMut(u32) -> io::Result<CpuMask>,
    FN: FnMut(u32) -> anyhow::Result<i32>,
    FI: FnMut(u32) -> anyhow::Result<i32>,
{
    let mut plan = ProfileApplyPlan::default();
    let mut seen_cache_keys = BTreeSet::new();
    let mut pending_tids = BTreeSet::new();

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

        let mut task_pending = false;

        if let Some(desired_mask) = &rule.affinity {
            let original_mask = match read_allowed_mask(task.tid) {
                Ok(mask) => mask,
                Err(err) if err.raw_os_error() == Some(libc::ESRCH) => {
                    continue; // Task is dead, skip it.
                }
                Err(err) => {
                    return Err(anyhow::anyhow!(
                        "failed to read CPU affinity for TID {}: {err}",
                        task.tid
                    ));
                }
            };

            if original_mask != *desired_mask {
                if !task_has_complete_identity(task) {
                    log::warn!(
                        "profile_skip_incomplete_identity tid={} comm={} process_pid={}",
                        task.tid,
                        task.comm,
                        task.process_pid
                    );
                } else {
                    plan.summary.pending_affinity += 1;
                    pending_tids.insert(task.tid);
                    task_pending = true;
                    plan.affinity_changes.push(PlannedAffinityChange {
                        record: AffinityRecord {
                            tid: task.tid,
                            process_pid: Some(task.process_pid),
                            process_starttime_ticks: task.process_starttime_ticks,
                            task_starttime_ticks: task.task_starttime_ticks,
                            original_mask,
                            applied_mask: desired_mask.clone(),
                        },
                    });
                }
            }
        }

        if let Some(desired_nice) = rule.nice {
            let original_nice = match read_nice(task.tid) {
                Ok(nice) => nice,
                Err(err) if anyhow_raw_os_error(&err) == Some(libc::ESRCH) => continue,
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!("failed to read nice for profile target TID {}", task.tid)
                    });
                }
            };

            if original_nice != desired_nice {
                if !task_has_complete_identity(task) {
                    log::warn!(
                        "profile_skip_priority_incomplete_identity tid={} comm={} process_pid={}",
                        task.tid,
                        task.comm,
                        task.process_pid
                    );
                } else {
                    plan.summary.pending_nice += 1;
                    pending_tids.insert(task.tid);
                    task_pending = true;
                    plan.nice_groups
                        .entry(desired_nice)
                        .or_default()
                        .push(TaskIdentity::from_task_info(task));
                    plan.nice_records.push(NiceRestoreRecordV2 {
                        tid: task.tid,
                        process_pid: Some(task.process_pid),
                        process_starttime_ticks: task.process_starttime_ticks,
                        task_starttime_ticks: task.task_starttime_ticks,
                        comm: Some(task.comm.clone()),
                        original_nice,
                        applied_nice: desired_nice,
                    });
                }
            }
        }

        if let Some(desired_ioprio) = rule.ionice {
            let original_ioprio = match read_ioprio(task.tid) {
                Ok(ioprio) => ioprio,
                Err(err) if anyhow_raw_os_error(&err) == Some(libc::ESRCH) => continue,
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!(
                            "failed to read I/O priority for profile target TID {}",
                            task.tid
                        )
                    });
                }
            };
            let desired_encoded = desired_ioprio.encode()?;

            if original_ioprio != desired_encoded {
                if !task_has_complete_identity(task) {
                    log::warn!(
                        "profile_skip_priority_incomplete_identity tid={} comm={} process_pid={}",
                        task.tid,
                        task.comm,
                        task.process_pid
                    );
                } else {
                    plan.summary.pending_ionice += 1;
                    pending_tids.insert(task.tid);
                    task_pending = true;
                    plan.ionice_groups
                        .entry(desired_ioprio)
                        .or_default()
                        .push(TaskIdentity::from_task_info(task));
                    plan.ionice_records.push(IoPrioRestoreRecordV2 {
                        tid: task.tid,
                        process_pid: Some(task.process_pid),
                        process_starttime_ticks: task.process_starttime_ticks,
                        task_starttime_ticks: task.task_starttime_ticks,
                        comm: Some(task.comm.clone()),
                        original_ioprio,
                        applied_ioprio: desired_encoded,
                    });
                }
            }
        }

        if task_pending {
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
    tasks: &BTreeMap<u32, TaskInfo>,
    profile: &Profile,
) -> anyhow::Result<ProfileApplySummary> {
    profile_apply_summary_with_readers(
        tasks,
        profile,
        affinity::read_allowed_mask_raw,
        crate::actions::nice::read_task_nice,
        crate::actions::ioprio::read_task_ioprio,
    )
}

#[cfg(test)]
fn profile_apply_summary_with_reader<F>(
    tasks: &BTreeMap<u32, TaskInfo>,
    profile: &Profile,
    read_allowed_mask: F,
) -> anyhow::Result<ProfileApplySummary>
where
    F: FnMut(u32) -> io::Result<CpuMask>,
{
    profile_apply_summary_with_readers(tasks, profile, read_allowed_mask, |_| Ok(0), |_| Ok(0))
}

fn profile_apply_summary_with_readers<FA, FN, FI>(
    tasks: &BTreeMap<u32, TaskInfo>,
    profile: &Profile,
    mut read_allowed_mask: FA,
    mut read_nice: FN,
    mut read_ioprio: FI,
) -> anyhow::Result<ProfileApplySummary>
where
    FA: FnMut(u32) -> io::Result<CpuMask>,
    FN: FnMut(u32) -> anyhow::Result<i32>,
    FI: FnMut(u32) -> anyhow::Result<i32>,
{
    let mut summary = ProfileApplySummary::default();
    let mut pending_tids = BTreeSet::new();

    for task in tasks.values() {
        let Some(rule) = matching_profile_rule(task, profile) else {
            continue;
        };

        summary.checked_tasks += 1;

        if let Some(desired_mask) = &rule.affinity {
            let original_mask = match read_allowed_mask(task.tid) {
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
                pending_tids.insert(task.tid);
            }
        }

        if let Some(desired_nice) = rule.nice {
            let original_nice = match read_nice(task.tid) {
                Ok(nice) => nice,
                Err(err) if anyhow_raw_os_error(&err) == Some(libc::ESRCH) => continue,
                Err(err) => return Err(err),
            };

            if original_nice != desired_nice {
                summary.pending_nice += 1;
                pending_tids.insert(task.tid);
            }
        }

        if let Some(desired_ioprio) = rule.ionice {
            let original_ioprio = match read_ioprio(task.tid) {
                Ok(ioprio) => ioprio,
                Err(err) if anyhow_raw_os_error(&err) == Some(libc::ESRCH) => continue,
                Err(err) => return Err(err),
            };

            if original_ioprio != desired_ioprio.encode()? {
                summary.pending_ionice += 1;
                pending_tids.insert(task.tid);
            }
        }
    }

    summary.pending_changes = pending_tids.len();
    Ok(summary)
}

fn preflight_profile_plan(plan: &ProfileApplyPlan) -> anyhow::Result<()> {
    for (nice, targets) in &plan.nice_groups {
        NiceAction {
            targets: targets.clone(),
            nice: *nice,
            policy: NicePolicy::default(),
        }
        .preflight()
        .with_context(|| format!("nice profile action preflight failed for nice={nice}"))?;
    }

    for (ioprio, targets) in &plan.ionice_groups {
        IoPrioAction {
            targets: targets.clone(),
            ioprio: *ioprio,
            policy: profile_ioprio_policy(),
        }
        .preflight()
        .with_context(|| {
            format!(
                "I/O priority profile action preflight failed for ionice={}",
                ioprio.label()
            )
        })?;
    }

    Ok(())
}

fn apply_profile_plan(
    plan: &ProfileApplyPlan,
    result: &mut ProfileApplyResult,
) -> anyhow::Result<()> {
    for planned in &plan.affinity_changes {
        match affinity::set_affinity_raw(planned.record.tid, &planned.record.applied_mask) {
            Ok(()) => result.affinity_records.push(planned.record.clone()),
            Err(err) if err.raw_os_error() == Some(libc::ESRCH) => {}
            Err(err) => {
                anyhow::bail!(
                    "failed to set affinity for TID {}: {err}",
                    planned.record.tid
                );
            }
        }
    }

    for (nice, targets) in &plan.nice_groups {
        let action = NiceAction {
            targets: targets.clone(),
            nice: *nice,
            policy: NicePolicy::default(),
        };
        let run_policy = ActionRunPolicy::for_action(
            &action,
            false,
            crate::daemon_policy::ActionSource::ApplyProfileWatch,
        );
        run_audited_action("apply-profile nice", &action, run_policy)
            .with_context(|| format!("failed to apply profile nice={nice}"))?;
    }

    for (ioprio, targets) in &plan.ionice_groups {
        let action = IoPrioAction {
            targets: targets.clone(),
            ioprio: *ioprio,
            policy: profile_ioprio_policy(),
        };
        let run_policy = ActionRunPolicy::for_action(
            &action,
            false,
            crate::daemon_policy::ActionSource::ApplyProfileWatch,
        );
        run_audited_action("apply-profile ionice", &action, run_policy)
            .with_context(|| format!("failed to apply profile I/O priority={}", ioprio.label()))?;
    }

    Ok(())
}

fn verify_affinity_plan(plan: &ProfileApplyPlan) -> anyhow::Result<()> {
    for planned in &plan.affinity_changes {
        match affinity::read_allowed_mask_raw(planned.record.tid) {
            Ok(mask) if mask == planned.record.applied_mask => {}
            Ok(mask) => {
                anyhow::bail!(
                    "affinity verify failed for TID {}: requested={} actual={}",
                    planned.record.tid,
                    planned.record.applied_mask.to_range_string(),
                    mask.to_range_string()
                );
            }
            Err(err) if err.raw_os_error() == Some(libc::ESRCH) => {}
            Err(err) => {
                anyhow::bail!(
                    "failed to verify affinity for TID {}: {err}",
                    planned.record.tid
                );
            }
        }
    }

    Ok(())
}

fn result_from_plan(plan: &ProfileApplyPlan) -> ProfileApplyResult {
    let mut affinity_records = plan.affinity_records();
    affinity_records.sort_by_key(|record| record.tid);

    ProfileApplyResult {
        affinity_records,
        nice_records: plan.nice_records.clone(),
        ionice_records: plan.ionice_records.clone(),
        summary: plan.summary.clone(),
    }
}

fn profile_ioprio_policy() -> IoPrioPolicy {
    IoPrioPolicy {
        allow_ioprio_changes: true,
        allow_realtime_class: true,
        allow_none_class: true,
        max_best_effort_level: 7,
        require_strong_block_io_evidence: false,
        strong_block_io_evidence: true,
    }
}

fn task_has_complete_identity(task: &TaskInfo) -> bool {
    task.process_starttime_ticks.is_some() && task.task_starttime_ticks.is_some()
}

fn anyhow_raw_os_error(err: &anyhow::Error) -> Option<i32> {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<io::Error>())
        .and_then(io::Error::raw_os_error)
}

pub fn profile_matched_task_count_from_snapshots(
    tasks: &[crate::autotune::observation::ActiveTaskSnapshot],
    profile: &Profile,
) -> usize {
    tasks
        .iter()
        .filter(|task| {
            let info = task_info_from_active_snapshot(task);
            profile_matches_task(&info, profile)
        })
        .count()
}

fn task_info_from_active_snapshot(
    task: &crate::autotune::observation::ActiveTaskSnapshot,
) -> TaskInfo {
    TaskInfo {
        tid: task.tid,
        process_pid: task.process_pid,
        process_ppid: 0,
        comm: task.comm.clone(),
        process_comm: task.comm.clone(),
        process_starttime_ticks: task.process_starttime_ticks,
        task_starttime_ticks: task.task_starttime_ticks,
        exe_dev: None,
        exe_ino: None,
        class: task.class,
        sched_policy: None,
        from_cgroup: task.cgroup_path.is_some(),
    }
}

fn matching_profile_rule<'a>(task: &TaskInfo, profile: &'a Profile) -> Option<&'a ProfileRule> {
    matching_profile_rule_with_index(task, profile).map(|(_, rule)| rule)
}

fn matching_profile_rule_with_index<'a>(
    task: &TaskInfo,
    profile: &'a Profile,
) -> Option<(usize, &'a ProfileRule)> {
    for (index, rule) in profile.rules.iter().enumerate() {
        if !rule.match_class.is_empty() && !rule.match_class.contains(&task.class) {
            continue;
        }

        if !rule.match_comm.is_empty() {
            let comms = [&task.comm, task.process_comm.as_str()];
            let mut comm_match = false;

            for pattern in &rule.match_comm {
                if comms.iter().any(|comm| pattern.matches(comm)) {
                    comm_match = true;
                    break;
                }
            }

            if !comm_match {
                continue;
            }
        }

        return Some((index, rule));
    }

    None
}

pub fn profile_matched_task_count_for_tree(tree_pid: u32, profile: &Profile) -> usize {
    let snapshot = process_tree::target_snapshot(
        process_tree::TargetSnapshotInput::default().tree_pids(&[tree_pid]),
    );
    profile_matched_task_count(&snapshot.tasks, profile)
}

pub fn profile_matched_task_count(tasks: &BTreeMap<u32, TaskInfo>, profile: &Profile) -> usize {
    tasks
        .values()
        .filter(|task| profile_matches_task(task, profile))
        .count()
}

pub fn profile_matches_task(task: &TaskInfo, profile: &Profile) -> bool {
    profile
        .rules
        .iter()
        .any(|rule| profile_rule_matches_task(task, rule))
}

pub fn profile_rule_matches_task(task: &TaskInfo, rule: &ProfileRule) -> bool {
    if !rule.match_class.is_empty() && !rule.match_class.contains(&task.class) {
        return false;
    }

    if !rule.match_comm.is_empty() {
        let comms = [&task.comm, task.process_comm.as_str()];
        return rule
            .match_comm
            .iter()
            .any(|pattern| comms.iter().any(|comm| pattern.matches(comm)));
    }

    true
}

impl ProfileApplyCacheKey {
    fn new(task: &TaskInfo, rule: &ProfileRule) -> Self {
        Self {
            tid: task.tid,
            process_starttime_ticks: task.process_starttime_ticks,
            task_starttime_ticks: task.task_starttime_ticks,
            desired_affinity: rule.affinity.clone(),
            desired_nice: rule.nice,
            desired_ionice: rule.ionice,
        }
    }
}

#[cfg(test)]
mod tests;
