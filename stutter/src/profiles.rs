use std::{
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::Path,
};

use anyhow::Context;
use serde::Deserialize;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileCpuWarning {
    pub rule_index: usize,
    pub requested: String,
    pub online: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRuleOverlapWarning {
    pub earlier_rule: usize,
    pub later_rule: usize,
}

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

pub fn load_first_profile(path: &Path) -> anyhow::Result<Profile> {
    load_profiles(path)?.into_iter().next().ok_or_else(|| {
        anyhow::anyhow!(
            "profile file {} did not contain [[profile]]",
            path.display()
        )
    })
}

pub fn load_profiles(path: &Path) -> anyhow::Result<Vec<Profile>> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read profile {}", path.display()))?;
    parse_profiles(&data).with_context(|| format!("failed to parse profile {}", path.display()))
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
        process_comm: task.comm.clone().into(),
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
            let comms = [&task.comm, task.process_comm.as_ref()];
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
        let comms = [&task.comm, task.process_comm.as_ref()];
        return rule
            .match_comm
            .iter()
            .any(|pattern| comms.iter().any(|comm| pattern.matches(comm)));
    }

    true
}

fn class_dimension_may_overlap(a: &[TaskClass], b: &[TaskClass]) -> bool {
    a.is_empty() || b.is_empty() || a.iter().any(|left| b.contains(left))
}

fn comm_dimension_may_overlap(a: &[CompiledPattern], b: &[CompiledPattern]) -> bool {
    a.is_empty()
        || b.is_empty()
        || a.iter()
            .any(|left| b.iter().any(|right| left.raw() == right.raw()))
}

fn rule_may_overlap(earlier: &ProfileRule, later: &ProfileRule) -> bool {
    let class_overlap = class_dimension_may_overlap(&earlier.match_class, &later.match_class);
    let comm_overlap = comm_dimension_may_overlap(&earlier.match_comm, &later.match_comm);

    class_overlap && comm_overlap
}

pub fn profile_rule_overlap_warnings(rules: &[ProfileRule]) -> Vec<ProfileRuleOverlapWarning> {
    let mut warnings = Vec::new();

    for earlier_rule in 0..rules.len() {
        for later_rule in (earlier_rule + 1)..rules.len() {
            if rule_may_overlap(&rules[earlier_rule], &rules[later_rule]) {
                warnings.push(ProfileRuleOverlapWarning {
                    earlier_rule,
                    later_rule,
                });
            }
        }
    }

    warnings
}

pub fn profile_offline_cpu_warnings(profile: &Profile, online: &CpuMask) -> Vec<ProfileCpuWarning> {
    profile
        .rules
        .iter()
        .enumerate()
        .filter_map(|(idx, rule)| {
            let affinity = rule.affinity.as_ref()?;
            (!affinity.is_subset_of(online)).then(|| ProfileCpuWarning {
                rule_index: idx,
                requested: affinity.to_range_string(),
                online: online.to_range_string(),
            })
        })
        .collect()
}

fn warn_profile_offline_cpus(profile: &Profile) -> anyhow::Result<()> {
    let online =
        CpuMask::online_cpus().context("failed to read online CPU mask before applying profile")?;

    for warning in profile_offline_cpu_warnings(profile, &online) {
        log::warn!(
            "profile_rule_offline_cpus rule={} requested={} online={}",
            warning.rule_index,
            warning.requested,
            warning.online
        );
    }

    Ok(())
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

fn parse_profiles(data: &str) -> anyhow::Result<Vec<Profile>> {
    let file = toml::from_str::<ProfilesFile>(data)?;
    file.profile
        .into_iter()
        .map(ProfileToml::try_into_profile)
        .map(|profile| profile.and_then(validate_profile))
        .collect()
}

fn validate_profile(profile: Profile) -> anyhow::Result<Profile> {
    if profile.name.is_empty() {
        anyhow::bail!("profile name must not be empty");
    }

    let online = affinity::CpuMask::online_cpus()
        .context("failed to read online CPU mask while validating profile")?;

    for (i, rule) in profile.rules.iter().enumerate() {
        if rule.affinity.is_none() && rule.nice.is_none() && rule.ionice.is_none() {
            anyhow::bail!(
                "profile rule {} must specify at least one action field: affinity, nice, or ionice",
                i
            );
        }
        if let Some(affinity) = &rule.affinity {
            if affinity.is_empty() {
                anyhow::bail!("profile rule {} has empty affinity", i);
            }
            if !affinity.is_subset_of(&online) {
                anyhow::bail!(
                    "profile rule {} requests CPUs not currently online. Online: {}",
                    i,
                    online.to_range_string()
                );
            }
        }
    }

    for warning in profile_rule_overlap_warnings(&profile.rules) {
        log::warn!(
            "profile_rule_overlap profile={} earlier_rule={} later_rule={} message=\"rules are first-match-wins; later rule may be shadowed\"",
            profile.name,
            warning.earlier_rule,
            warning.later_rule
        );
    }

    Ok(profile)
}

#[derive(Deserialize)]
struct ProfilesFile {
    #[serde(default)]
    profile: Vec<ProfileToml>,
}

#[derive(Deserialize)]
struct ProfileToml {
    name: String,
    #[serde(default)]
    rules: Vec<ProfileRuleToml>,
}

#[derive(Deserialize)]
struct ProfileRuleToml {
    affinity: Option<String>,
    nice: Option<i32>,
    ionice: Option<String>,
    #[serde(default)]
    match_class: Vec<String>,
    #[serde(default)]
    match_comm: Vec<String>,
}

impl ProfileToml {
    fn try_into_profile(self) -> anyhow::Result<Profile> {
        let mut rules = Vec::new();

        for rule in self.rules {
            let mut match_class = Vec::new();
            for class_name in rule.match_class {
                match_class.push(parse_task_class(&class_name)?);
            }

            let match_comm = rule
                .match_comm
                .into_iter()
                .map(CompiledPattern::new)
                .collect::<anyhow::Result<Vec<_>>>()?;

            let affinity = rule
                .affinity
                .as_deref()
                .map(parse_affinity_value)
                .transpose()?;
            let ionice = rule.ionice.as_deref().map(parse_ionice_value).transpose()?;

            if rule.nice.is_none() && affinity.is_none() && ionice.is_none() {
                anyhow::bail!(
                    "profile rule must specify at least one action field: affinity, nice, or ionice"
                );
            }

            if let Some(nice) = rule.nice
                && !(-20..=19).contains(&nice)
            {
                anyhow::bail!("nice value {nice} is outside Linux range -20..=19");
            }

            rules.push(ProfileRule {
                affinity,
                nice: rule.nice,
                ionice,
                match_class,
                match_comm,
            });
        }

        Ok(Profile {
            name: self.name,
            rules,
        })
    }
}

fn parse_affinity_value(value: &str) -> anyhow::Result<CpuMask> {
    if value.trim() == "online" {
        CpuMask::online_cpus()
    } else {
        CpuMask::parse(value)
    }
}

fn parse_ionice_value(value: &str) -> anyhow::Result<IoPrioValue> {
    let trimmed = value.trim().to_ascii_lowercase();

    match trimmed.as_str() {
        "idle" => return Ok(IoPrioValue::idle()),
        "none" => return Ok(IoPrioValue::none()),
        "best-effort" | "be" | "realtime" | "rt" => {
            anyhow::bail!("ionice value {value:?} requires a level 0..=7")
        }
        _ => {}
    }

    let Some((class, level)) = trimmed.split_once(':') else {
        anyhow::bail!("invalid ionice value {value:?}");
    };
    let level = level
        .parse::<u8>()
        .with_context(|| format!("invalid ionice level in {value:?}"))?;

    let parsed = match class {
        "best-effort" | "be" => IoPrioValue::best_effort(level),
        "realtime" | "rt" => IoPrioValue::realtime(level),
        "idle" => anyhow::bail!("ionice class idle must not specify a level"),
        "none" => anyhow::bail!("ionice class none must not specify a level"),
        _ => anyhow::bail!("invalid ionice class {class:?}"),
    };

    parsed.encode()?;
    Ok(parsed)
}

fn parse_task_class(value: &str) -> anyhow::Result<TaskClass> {
    match value {
        "Game" => Ok(TaskClass::Game),
        "GameRenderThread" => Ok(TaskClass::GameRenderThread),
        "GameWorkerThread" => Ok(TaskClass::GameWorkerThread),
        "GameHelper" => Ok(TaskClass::GameHelper),
        "Launcher" => Ok(TaskClass::Launcher),
        "WineServer" => Ok(TaskClass::WineServer),
        "GameScope" => Ok(TaskClass::GameScope),
        "Compositor" => Ok(TaskClass::Compositor),
        "AudioRealtime" => Ok(TaskClass::AudioRealtime),
        "Input" => Ok(TaskClass::Input),
        "BrowserForeground" => Ok(TaskClass::BrowserForeground),
        "BrowserBackground" => Ok(TaskClass::BrowserBackground),
        "BrowserRenderer" => Ok(TaskClass::BrowserRenderer),
        "BrowserGpu" => Ok(TaskClass::BrowserGpu),
        "BrowserNetwork" => Ok(TaskClass::BrowserNetwork),
        "Compiler" => Ok(TaskClass::Compiler),
        "Linker" => Ok(TaskClass::Linker),
        "Indexer" => Ok(TaskClass::Indexer),
        "PackageManager" => Ok(TaskClass::PackageManager),
        "BuildJob" => Ok(TaskClass::BuildJob),
        "StorageDaemon" => Ok(TaskClass::StorageDaemon),
        "NetworkDaemon" => Ok(TaskClass::NetworkDaemon),
        "KernelThread" => Ok(TaskClass::KernelThread),
        "IrqThread" => Ok(TaskClass::IrqThread),
        "Editor" => Ok(TaskClass::Editor),
        "Terminal" => Ok(TaskClass::Terminal),
        "Shell" => Ok(TaskClass::Shell),
        "Media" => Ok(TaskClass::Media),
        "Recorder" => Ok(TaskClass::Recorder),
        "VirtualMachine" => Ok(TaskClass::VirtualMachine),
        "SteamRuntime" => Ok(TaskClass::SteamRuntime),
        "Render" => Ok(TaskClass::Render),
        "Helper" => Ok(TaskClass::Helper),
        "Service" => Ok(TaskClass::Service),
        "Unknown" => Ok(TaskClass::Unknown),
        _ => anyhow::bail!("unknown task class {value}"),
    }
}

pub fn render_profiles_toml(profiles: &[Profile]) -> String {
    let mut out = String::new();

    for profile in profiles {
        out.push_str("[[profile]]\n");
        out.push_str("name = ");
        out.push_str(&toml_quoted_string(&profile.name));
        out.push_str("\n\n");

        for rule in &profile.rules {
            out.push_str("[[profile.rules]]\n");
            if let Some(affinity) = &rule.affinity {
                out.push_str("affinity = ");
                out.push_str(&toml_quoted_string(&affinity.to_range_string()));
                out.push('\n');
            }

            if let Some(nice) = rule.nice {
                out.push_str("nice = ");
                out.push_str(&nice.to_string());
                out.push('\n');
            }

            if let Some(ionice) = rule.ionice {
                out.push_str("ionice = ");
                out.push_str(&toml_quoted_string(&ionice.label()));
                out.push('\n');
            }

            if !rule.match_class.is_empty() {
                out.push_str("match_class = [");
                for (idx, class) in rule.match_class.iter().enumerate() {
                    if idx > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&toml_quoted_string(task_class_toml_name(*class)));
                }
                out.push_str("]\n");
            }

            if !rule.match_comm.is_empty() {
                out.push_str("match_comm = [");
                for (idx, pattern) in rule.match_comm.iter().enumerate() {
                    if idx > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&toml_quoted_string(pattern.raw()));
                }
                out.push_str("]\n");
            }

            out.push('\n');
        }
    }

    out
}

fn toml_quoted_string(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');

    for ch in value.chars() {
        match ch {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            other => quoted.push(other),
        }
    }

    quoted.push('"');
    quoted
}

fn task_class_toml_name(class: TaskClass) -> &'static str {
    match class {
        TaskClass::Game => "Game",
        TaskClass::GameRenderThread => "GameRenderThread",
        TaskClass::GameWorkerThread => "GameWorkerThread",
        TaskClass::GameHelper => "GameHelper",
        TaskClass::Launcher => "Launcher",
        TaskClass::WineServer => "WineServer",
        TaskClass::GameScope => "GameScope",
        TaskClass::Compositor => "Compositor",
        TaskClass::AudioRealtime => "AudioRealtime",
        TaskClass::Input => "Input",
        TaskClass::BrowserForeground => "BrowserForeground",
        TaskClass::BrowserBackground => "BrowserBackground",
        TaskClass::BrowserRenderer => "BrowserRenderer",
        TaskClass::BrowserGpu => "BrowserGpu",
        TaskClass::BrowserNetwork => "BrowserNetwork",
        TaskClass::Compiler => "Compiler",
        TaskClass::Linker => "Linker",
        TaskClass::Indexer => "Indexer",
        TaskClass::PackageManager => "PackageManager",
        TaskClass::BuildJob => "BuildJob",
        TaskClass::StorageDaemon => "StorageDaemon",
        TaskClass::NetworkDaemon => "NetworkDaemon",
        TaskClass::KernelThread => "KernelThread",
        TaskClass::IrqThread => "IrqThread",
        TaskClass::Editor => "Editor",
        TaskClass::Terminal => "Terminal",
        TaskClass::Shell => "Shell",
        TaskClass::Media => "Media",
        TaskClass::Recorder => "Recorder",
        TaskClass::VirtualMachine => "VirtualMachine",
        TaskClass::SteamRuntime => "SteamRuntime",
        TaskClass::Render => "Render",
        TaskClass::Helper => "Helper",
        TaskClass::Service => "Service",
        TaskClass::Unknown => "Unknown",
    }
}

pub fn generate_topology_template() -> String {
    let mut out = String::new();
    out.push_str("[[profile]]\n");
    out.push_str("name = \"baseline-online\"\n\n");
    out.push_str("[[profile.rules]]\n");
    out.push_str("affinity = \"online\"\n");
    out.push_str("match_class = [\"Game\", \"GameRenderThread\", \"GameWorkerThread\", \"GameHelper\", \"WineServer\", \"GameScope\", \"Compositor\", \"AudioRealtime\", \"Input\", \"BrowserForeground\", \"BrowserBackground\", \"BrowserRenderer\", \"BrowserGpu\", \"BrowserNetwork\", \"Compiler\", \"Linker\", \"Indexer\", \"PackageManager\", \"BuildJob\", \"StorageDaemon\", \"NetworkDaemon\", \"KernelThread\", \"IrqThread\", \"Editor\", \"Terminal\", \"Shell\", \"Media\", \"Recorder\", \"VirtualMachine\", \"SteamRuntime\", \"Helper\", \"Service\", \"Unknown\"]\n\n");
    out.push_str("[[profile]]\n");
    out.push_str("name = \"game-main-suggested\"\n\n");
    out.push_str("[[profile.rules]]\n");
    out.push_str("affinity = \"<edit-me>\"\n");
    out.push_str("match_class = [\"Game\", \"GameRenderThread\", \"GameWorkerThread\", \"GameHelper\", \"WineServer\"]\n\n");
    out.push_str("[[profile.rules]]\n");
    out.push_str("affinity = \"<edit-me>\"\n");
    out.push_str("match_class = [\"GameScope\", \"Compositor\"]\n");
    out.push_str("\n[[profile.rules]]\n");
    out.push_str("nice = 10\n");
    out.push_str("ionice = \"idle\"\n");
    out.push_str("match_class = [\"BuildJob\", \"Indexer\", \"PackageManager\"]\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_task(tid: u32, class: TaskClass, comm: &str) -> TaskInfo {
        TaskInfo {
            tid,
            process_pid: tid,
            process_ppid: 1,
            comm: comm.into(),
            process_comm: "process".into(),
            process_starttime_ticks: Some(u64::from(tid) * 10),
            task_starttime_ticks: Some(u64::from(tid) * 10 + 1),
            exe_dev: None,
            exe_ino: None,
            class,
            sched_policy: None,
            from_cgroup: false,
        }
    }

    #[test]
    fn parses_minimal_profile() {
        let profiles = parse_profiles(
            r#"
            [[profile]]
            name = "kcd # not a comment"

            [[profile.rules]]
            affinity = "0-3"
            match_class = ["Game"]
            match_comm = ["RenderThread", "Main"]
            "#,
        )
        .unwrap();

        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "kcd # not a comment");
        let rule = &profiles[0].rules[0];
        assert_eq!(rule.affinity.as_ref().unwrap().to_range_string(), "0-3");
        assert_eq!(rule.match_class, vec![TaskClass::Game]);
        assert_eq!(
            rule.match_comm
                .iter()
                .map(CompiledPattern::raw)
                .collect::<Vec<_>>(),
            vec!["RenderThread", "Main"]
        );
    }

    #[test]
    fn render_profiles_toml_outputs_profile_rules() {
        let profile = Profile {
            name: "generated \"profile\"".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0-1").unwrap()),
                nice: Some(5),
                ionice: Some(IoPrioValue::idle()),
                match_class: vec![TaskClass::Game, TaskClass::GameRenderThread],
                match_comm: vec![
                    CompiledPattern::new("RenderThread".to_owned()).unwrap(),
                    CompiledPattern::new("Main".to_owned()).unwrap(),
                ],
            }],
        };

        let toml = render_profiles_toml(&[profile]);

        assert!(toml.contains("[[profile]]"));
        assert!(toml.contains("name = \"generated \\\"profile\\\"\""));
        assert!(toml.contains("[[profile.rules]]"));
        assert!(toml.contains("affinity = \"0-1\""));
        assert!(toml.contains("nice = 5"));
        assert!(toml.contains("ionice = \"idle\""));
        assert!(toml.contains("match_class = [\"Game\", \"GameRenderThread\"]"));
        assert!(toml.contains("match_comm = [\"RenderThread\", \"Main\"]"));
    }

    #[test]
    fn profile_parser_accepts_online_affinity() {
        let profiles = parse_profiles(
            r#"
            [[profile]]
            name = "baseline-online"

            [[profile.rules]]
            affinity = "online"
            match_class = ["Game"]
            "#,
        )
        .unwrap();

        assert_eq!(profiles.len(), 1);
        assert!(!profiles[0].rules[0].affinity.as_ref().unwrap().is_empty());
    }

    #[test]
    fn profile_parser_accepts_nice_only_rule() {
        let profiles = parse_profiles(
            r#"
            [[profile]]
            name = "background"

            [[profile.rules]]
            match_class = ["Indexer"]
            nice = 10
            "#,
        )
        .unwrap();

        let rule = &profiles[0].rules[0];
        assert!(rule.affinity.is_none());
        assert_eq!(rule.nice, Some(10));
        assert_eq!(rule.ionice, None);
    }

    #[test]
    fn profile_parser_accepts_ionice_only_rule() {
        let profiles = parse_profiles(
            r#"
            [[profile]]
            name = "background"

            [[profile.rules]]
            match_class = ["PackageManager"]
            ionice = "idle"
            "#,
        )
        .unwrap();

        let rule = &profiles[0].rules[0];
        assert!(rule.affinity.is_none());
        assert_eq!(rule.nice, None);
        assert_eq!(rule.ionice, Some(IoPrioValue::idle()));
    }

    #[test]
    fn profile_parser_accepts_combined_affinity_nice_ionice_rule() {
        let profiles = parse_profiles(
            r#"
            [[profile]]
            name = "game-latency"

            [[profile.rules]]
            match_class = ["Game", "GameRenderThread"]
            affinity = "0-3"
            nice = -5
            ionice = "be:2"
            "#,
        )
        .unwrap();

        let rule = &profiles[0].rules[0];
        assert_eq!(rule.affinity.as_ref().unwrap().to_range_string(), "0-3");
        assert_eq!(rule.nice, Some(-5));
        assert_eq!(rule.ionice, Some(IoPrioValue::best_effort(2)));
    }

    #[test]
    fn profile_parser_rejects_invalid_nice_range() {
        let err = parse_profiles(
            r#"
            [[profile]]
            name = "bad"

            [[profile.rules]]
            nice = 20
            "#,
        )
        .unwrap_err();

        assert!(format!("{err:#}").contains("outside Linux range"));
    }

    #[test]
    fn profile_parser_rejects_invalid_ionice_strings() {
        for ionice in ["best-effort", "realtime", "be:8", "rt:9", "idle:4"] {
            let err = parse_profiles(&format!(
                r#"
                [[profile]]
                name = "bad"

                [[profile.rules]]
                ionice = "{ionice}"
                "#
            ))
            .unwrap_err();

            assert!(
                format!("{err:#}").contains("ionice")
                    || format!("{err:#}").contains("I/O priority")
            );
        }
    }

    #[test]
    fn profile_parser_rejects_rule_with_no_action_fields() {
        let err = parse_profiles(
            r#"
            [[profile]]
            name = "bad"

            [[profile.rules]]
            match_class = ["Game"]
            "#,
        )
        .unwrap_err();

        assert!(format!("{err:#}").contains("at least one action field"));
    }

    #[test]
    fn invalid_symbolic_affinity_fails_clearly() {
        let err = parse_profiles(
            r#"
            [[profile]]
            name = "bad"

            [[profile.rules]]
            affinity = "all"
            match_class = ["Game"]
            "#,
        )
        .unwrap_err();

        assert!(err.to_string().contains("invalid CPU id"));
    }

    #[test]
    fn examples_profile_file_parses() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let path = manifest_dir
            .parent()
            .unwrap()
            .join("examples/profiles/common-game-layouts.toml");
        let profiles = load_profiles(&path).unwrap();

        assert!(!profiles.is_empty());
        assert!(
            profiles
                .iter()
                .any(|profile| profile.name == "baseline-online")
        );
    }

    #[test]
    fn match_comm_treats_metacharacters_as_literals_unless_slash_delimited() {
        let literal = CompiledPattern::new("KingdomCome.exe".to_owned()).unwrap();
        assert!(literal.matches("KingdomCome.exe"));
        assert!(literal.matches("kingdomcome.exe"));
        assert!(!literal.matches("KingdomComeXexe"));

        let regex = CompiledPattern::new("/KingdomCome[.]exe$/".to_owned()).unwrap();
        assert!(regex.matches("KingdomCome.exe"));
        assert!(!regex.matches("kingdomcome.exe"));
        assert!(!regex.matches("KingdomComeXexe"));

        let literal_bracket = CompiledPattern::new("[".to_owned()).unwrap();
        assert!(literal_bracket.matches("renderer[0]"));
        assert!(CompiledPattern::new("/[/".to_owned()).is_err());
    }

    #[test]
    fn profile_match_class_sees_community_rule_game_class() {
        let class = process_tree::classify_task_with_context(
            "KingdomCome",
            "KingdomCome",
            "/home/me/.steam/steamapps/compatdata/379430/pfx/drive_c/KingdomCome.exe",
            "/usr/bin/wine",
            "/user.slice/app-steam-379430.scope",
            None,
        );
        let task = TaskInfo {
            tid: 379430,
            process_pid: 379430,
            process_ppid: 1,
            comm: "KingdomCome".into(),
            process_comm: "KingdomCome".into(),
            process_starttime_ticks: Some(379430),
            task_starttime_ticks: Some(379430),
            exe_dev: None,
            exe_ino: None,
            class,
            sched_policy: None,
            from_cgroup: false,
        };
        let profile = Profile {
            name: "game".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0-1").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        };

        assert_eq!(task.class, TaskClass::Game);
        assert!(profile_matches_task(&task, &profile));
    }

    #[test]
    fn profile_apply_summary_counts_matching_tasks_and_pending_changes() {
        let task_correct = TaskInfo {
            tid: 7,
            process_pid: 7,
            process_ppid: 1,
            comm: "RenderThread".into(),
            process_comm: "game".into(),
            process_starttime_ticks: Some(70),
            task_starttime_ticks: Some(70),
            exe_dev: None,
            exe_ino: None,
            class: TaskClass::Game,
            sched_policy: None,
            from_cgroup: false,
        };
        let task_pending = TaskInfo {
            tid: 8,
            process_pid: 8,
            process_ppid: 1,
            comm: "WorkerThread".into(),
            process_comm: "game".into(),
            process_starttime_ticks: Some(80),
            task_starttime_ticks: Some(80),
            exe_dev: None,
            exe_ino: None,
            class: TaskClass::Game,
            sched_policy: None,
            from_cgroup: false,
        };
        let task_unmatched = TaskInfo {
            tid: 9,
            process_pid: 9,
            process_ppid: 1,
            comm: "Compositor".into(),
            process_comm: "sway".into(),
            process_starttime_ticks: Some(90),
            task_starttime_ticks: Some(90),
            exe_dev: None,
            exe_ino: None,
            class: TaskClass::Compositor,
            sched_policy: None,
            from_cgroup: false,
        };
        let tasks = BTreeMap::from([(7, task_correct), (8, task_pending), (9, task_unmatched)]);
        let profile = Profile {
            name: "test".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0-1").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        };

        let summary = profile_apply_summary_with_reader(&tasks, &profile, |tid| match tid {
            7 => Ok(CpuMask::parse("0-1").unwrap()),
            8 => Ok(CpuMask::parse("0").unwrap()),
            9 => Ok(CpuMask::parse("0-1").unwrap()),
            other => panic!("unexpected TID {other}"),
        })
        .unwrap();

        assert_eq!(
            summary,
            ProfileApplySummary {
                checked_tasks: 2,
                pending_changes: 1,
                pending_affinity: 1,
                pending_nice: 0,
                pending_ionice: 0,
            }
        );
    }

    #[test]
    fn profile_plan_constructs_task_identity_for_priority_targets() {
        let task = test_task(42, TaskClass::Indexer, "indexer");
        let tasks = BTreeMap::from([(42, task)]);
        let profile = Profile {
            name: "priority".to_owned(),
            rules: vec![ProfileRule {
                affinity: None,
                nice: Some(10),
                ionice: None,
                match_class: vec![TaskClass::Indexer],
                match_comm: Vec::new(),
            }],
        };

        let plan = planned_profile_apply_with_readers(
            &tasks,
            &profile,
            None,
            |_| panic!("affinity should not be read"),
            |_| Ok(0),
            |_| Ok(0),
        )
        .unwrap();
        let identity = &plan.nice_groups.get(&10).unwrap()[0];

        assert_eq!(identity.tid, 42);
        assert_eq!(identity.process_pid, Some(42));
        assert_eq!(identity.comm.as_deref(), Some("indexer"));
        assert_eq!(identity.starttime_ticks, Some(421));
    }

    #[test]
    fn profile_apply_summary_counts_priority_actions_without_double_counting_tasks() {
        let task = test_task(42, TaskClass::PackageManager, "pacman");
        let tasks = BTreeMap::from([(42, task)]);
        let profile = Profile {
            name: "priority".to_owned(),
            rules: vec![ProfileRule {
                affinity: None,
                nice: Some(10),
                ionice: Some(IoPrioValue::idle()),
                match_class: vec![TaskClass::PackageManager],
                match_comm: Vec::new(),
            }],
        };

        let summary = profile_apply_summary_with_readers(
            &tasks,
            &profile,
            |_| panic!("affinity should not be read"),
            |_| Ok(0),
            |_| Ok(IoPrioValue::best_effort(4).encode().unwrap()),
        )
        .unwrap();

        assert_eq!(
            summary,
            ProfileApplySummary {
                checked_tasks: 1,
                pending_changes: 1,
                pending_affinity: 0,
                pending_nice: 1,
                pending_ionice: 1,
            }
        );
    }

    #[test]
    fn profile_apply_cache_invalidates_when_desired_nice_changes() {
        let task = test_task(42, TaskClass::Indexer, "indexer");
        let tasks = BTreeMap::from([(42, task)]);
        let mut cache = ProfileApplyCache::default();
        let mut nice_reads = 0;

        let profile_nice_5 = Profile {
            name: "priority".to_owned(),
            rules: vec![ProfileRule {
                affinity: None,
                nice: Some(5),
                ionice: None,
                match_class: vec![TaskClass::Indexer],
                match_comm: Vec::new(),
            }],
        };
        let profile_nice_6 = Profile {
            name: "priority".to_owned(),
            rules: vec![ProfileRule {
                affinity: None,
                nice: Some(6),
                ionice: None,
                match_class: vec![TaskClass::Indexer],
                match_comm: Vec::new(),
            }],
        };

        let first = planned_profile_apply_with_readers(
            &tasks,
            &profile_nice_5,
            Some(&mut cache),
            |_| panic!("affinity should not be read"),
            |_| {
                nice_reads += 1;
                Ok(5)
            },
            |_| Ok(0),
        )
        .unwrap();
        assert!(first.is_empty());
        assert_eq!(nice_reads, 1);

        let second = planned_profile_apply_with_readers(
            &tasks,
            &profile_nice_5,
            Some(&mut cache),
            |_| panic!("affinity should not be read"),
            |_| {
                nice_reads += 1;
                Ok(5)
            },
            |_| Ok(0),
        )
        .unwrap();
        assert!(second.is_empty());
        assert_eq!(nice_reads, 1);

        let third = planned_profile_apply_with_readers(
            &tasks,
            &profile_nice_6,
            Some(&mut cache),
            |_| panic!("affinity should not be read"),
            |_| {
                nice_reads += 1;
                Ok(5)
            },
            |_| Ok(0),
        )
        .unwrap();
        assert_eq!(third.summary.pending_nice, 1);
        assert_eq!(nice_reads, 2);
    }

    #[test]
    fn profile_apply_cache_invalidates_when_desired_ionice_changes() {
        let task = test_task(42, TaskClass::PackageManager, "pacman");
        let tasks = BTreeMap::from([(42, task)]);
        let mut cache = ProfileApplyCache::default();
        let mut ionice_reads = 0;
        let idle = IoPrioValue::idle();
        let best_effort = IoPrioValue::best_effort(6);

        let profile_idle = Profile {
            name: "priority".to_owned(),
            rules: vec![ProfileRule {
                affinity: None,
                nice: None,
                ionice: Some(idle),
                match_class: vec![TaskClass::PackageManager],
                match_comm: Vec::new(),
            }],
        };
        let profile_be = Profile {
            name: "priority".to_owned(),
            rules: vec![ProfileRule {
                affinity: None,
                nice: None,
                ionice: Some(best_effort),
                match_class: vec![TaskClass::PackageManager],
                match_comm: Vec::new(),
            }],
        };

        let first = planned_profile_apply_with_readers(
            &tasks,
            &profile_idle,
            Some(&mut cache),
            |_| panic!("affinity should not be read"),
            |_| Ok(0),
            |_| {
                ionice_reads += 1;
                Ok(idle.encode().unwrap())
            },
        )
        .unwrap();
        assert!(first.is_empty());
        assert_eq!(ionice_reads, 1);

        let second = planned_profile_apply_with_readers(
            &tasks,
            &profile_idle,
            Some(&mut cache),
            |_| panic!("affinity should not be read"),
            |_| Ok(0),
            |_| {
                ionice_reads += 1;
                Ok(idle.encode().unwrap())
            },
        )
        .unwrap();
        assert!(second.is_empty());
        assert_eq!(ionice_reads, 1);

        let third = planned_profile_apply_with_readers(
            &tasks,
            &profile_be,
            Some(&mut cache),
            |_| panic!("affinity should not be read"),
            |_| Ok(0),
            |_| {
                ionice_reads += 1;
                Ok(idle.encode().unwrap())
            },
        )
        .unwrap();
        assert_eq!(third.summary.pending_ionice, 1);
        assert_eq!(ionice_reads, 2);
    }

    #[test]
    fn profile_matched_task_count_counts_only_matching_rules() {
        let game_task = TaskInfo {
            tid: 7,
            process_pid: 7,
            process_ppid: 1,
            comm: "RenderThread".into(),
            process_comm: "game".into(),
            process_starttime_ticks: Some(70),
            task_starttime_ticks: Some(70),
            exe_dev: None,
            exe_ino: None,
            class: TaskClass::Game,
            sched_policy: None,
            from_cgroup: false,
        };
        let compositor_task = TaskInfo {
            tid: 8,
            process_pid: 8,
            process_ppid: 1,
            comm: "Compositor".into(),
            process_comm: "sway".into(),
            process_starttime_ticks: Some(80),
            task_starttime_ticks: Some(80),
            exe_dev: None,
            exe_ino: None,
            class: TaskClass::Compositor,
            sched_policy: None,
            from_cgroup: false,
        };
        let tasks = BTreeMap::from([(7, game_task), (8, compositor_task)]);
        let profile = Profile {
            name: "game-render".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: vec![CompiledPattern::new("RenderThread".to_owned()).unwrap()],
            }],
        };

        assert_eq!(profile_matched_task_count(&tasks, &profile), 1);
        assert!(profile_matches_task(tasks.get(&7).unwrap(), &profile));
        assert!(!profile_matches_task(tasks.get(&8).unwrap(), &profile));
    }

    #[test]
    fn profile_apply_cache_skips_unchanged_known_correct_tasks() {
        let task = TaskInfo {
            tid: 7,
            process_pid: 7,
            process_ppid: 1,
            comm: "RenderThread".into(),
            process_comm: "game".into(),
            process_starttime_ticks: Some(70),
            task_starttime_ticks: Some(70),
            exe_dev: None,
            exe_ino: None,
            class: TaskClass::Game,
            sched_policy: None,
            from_cgroup: false,
        };
        let tasks = BTreeMap::from([(7, task)]);
        let profile = Profile {
            name: "test".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0-1").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        };
        let mut cache = ProfileApplyCache::default();
        let mut reads = 0;

        let first = planned_profile_apply_with_readers(
            &tasks,
            &profile,
            Some(&mut cache),
            |tid| {
                reads += 1;
                assert_eq!(tid, 7);
                Ok(CpuMask::parse("0-1").unwrap())
            },
            |_| Ok(0),
            |_| Ok(0),
        )
        .unwrap();
        assert!(first.is_empty());
        assert_eq!(reads, 1);

        let second = planned_profile_apply_with_readers(
            &tasks,
            &profile,
            Some(&mut cache),
            |_| {
                reads += 1;
                Ok(CpuMask::parse("0-1").unwrap())
            },
            |_| Ok(0),
            |_| Ok(0),
        )
        .unwrap();
        assert!(second.is_empty());
        assert_eq!(reads, 1);

        cache.clear();
        let third = planned_profile_apply_with_readers(
            &tasks,
            &profile,
            Some(&mut cache),
            |_| {
                reads += 1;
                Ok(CpuMask::parse("0-1").unwrap())
            },
            |_| Ok(0),
            |_| Ok(0),
        )
        .unwrap();
        assert!(third.is_empty());
        assert_eq!(reads, 2);
    }

    #[test]
    fn profile_offline_cpu_warnings_detects_rule_with_offline_cpus() {
        let profile = Profile {
            name: "test".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0-3").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        };
        let online = CpuMask::parse("0-1").unwrap();

        let warnings = profile_offline_cpu_warnings(&profile, &online);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].rule_index, 0);
        assert_eq!(warnings[0].requested, "0-3");
        assert_eq!(warnings[0].online, "0-1");
    }

    #[test]
    fn profile_offline_cpu_warnings_empty_when_subset() {
        let profile = Profile {
            name: "test".to_owned(),
            rules: vec![ProfileRule {
                affinity: Some(CpuMask::parse("0-1").unwrap()),
                nice: None,
                ionice: None,
                match_class: vec![TaskClass::Game],
                match_comm: Vec::new(),
            }],
        };
        let online = CpuMask::parse("0-3").unwrap();

        let warnings = profile_offline_cpu_warnings(&profile, &online);

        assert!(warnings.is_empty());
    }

    #[test]
    fn profile_offline_cpu_warnings_multiple_rules_report_correct_indexes() {
        let profile = Profile {
            name: "test".to_owned(),
            rules: vec![
                ProfileRule {
                    affinity: Some(CpuMask::parse("0-1").unwrap()),
                    nice: None,
                    ionice: None,
                    match_class: vec![TaskClass::Game],
                    match_comm: Vec::new(),
                },
                ProfileRule {
                    affinity: Some(CpuMask::parse("2-3").unwrap()),
                    nice: None,
                    ionice: None,
                    match_class: vec![TaskClass::GameHelper],
                    match_comm: Vec::new(),
                },
            ],
        };
        let online = CpuMask::parse("0-1").unwrap();

        let warnings = profile_offline_cpu_warnings(&profile, &online);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].rule_index, 1);
        assert_eq!(warnings[0].requested, "2-3");
        assert_eq!(warnings[0].online, "0-1");
    }

    #[test]
    fn profile_rule_overlap_warnings_broad_game_before_specific_render_thread_warns() {
        let profile = parse_profiles(
            r#"
            [[profile]]
            name = "test"

            [[profile.rules]]
            match_class = ["Game"]
            affinity = "0-7"

            [[profile.rules]]
            match_comm = ["RenderThread"]
            affinity = "2-5"
            "#,
        )
        .unwrap()
        .pop()
        .unwrap();

        let warnings = profile_rule_overlap_warnings(&profile.rules);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].earlier_rule, 0);
        assert_eq!(warnings[0].later_rule, 1);
    }

    #[test]
    fn profile_rule_overlap_warnings_disjoint_classes_do_not_warn() {
        let profile = parse_profiles(
            r#"
            [[profile]]
            name = "test"

            [[profile.rules]]
            match_class = ["Game"]
            affinity = "0-7"

            [[profile.rules]]
            match_class = ["Compositor"]
            affinity = "8-11"
            "#,
        )
        .unwrap()
        .pop()
        .unwrap();

        let warnings = profile_rule_overlap_warnings(&profile.rules);
        assert!(warnings.is_empty());
    }

    #[test]
    fn profile_rule_overlap_warnings_catch_all_before_anything_warns() {
        let profile = parse_profiles(
            r#"
            [[profile]]
            name = "test"

            [[profile.rules]]
            affinity = "0-7"

            [[profile.rules]]
            match_class = ["Game"]
            affinity = "2-5"
            "#,
        )
        .unwrap()
        .pop()
        .unwrap();

        let warnings = profile_rule_overlap_warnings(&profile.rules);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].earlier_rule, 0);
        assert_eq!(warnings[0].later_rule, 1);
    }

    #[test]
    fn profile_rule_overlap_warnings_exact_same_comm_warns() {
        let profile = parse_profiles(
            r#"
            [[profile]]
            name = "test"

            [[profile.rules]]
            match_comm = ["RenderThread"]
            affinity = "0-3"

            [[profile.rules]]
            match_comm = ["RenderThread"]
            affinity = "4-7"
            "#,
        )
        .unwrap()
        .pop()
        .unwrap();

        let warnings = profile_rule_overlap_warnings(&profile.rules);
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].earlier_rule, 0);
        assert_eq!(warnings[0].later_rule, 1);
    }
}
