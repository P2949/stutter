//! Process target snapshot construction and expansion.
//!
//! Owns target snapshot selection, task expansion, diffing, auto-target discovery, and task-info
//! construction. Does not own raw procfs scanning, built-in classification policy, tree rendering,
//! or cgroup traversal.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
    time::Duration,
};

use crate::{
    community_rules::{
        CommunityProcessIdentity, CommunityRulesDb, classify_process_identity_with_db,
    },
    process::{
        cgroup::collect_cgroup_pids_at,
        classify::{classify_task_with_context, contains_likely_game_cmdline},
        model::{
            DEFAULT_MAX_PROC_SCAN_MS, DEFAULT_MAX_THREADS_PER_PROCESS, ProcInfo, ProcessCache,
            ScanBudget, ScanBudgetReport, TargetDiffAction, TargetDiffRef, TargetSnapshot,
            TargetSnapshotInput, TaskClass, TaskFilters, TaskInfo,
        },
    },
    process_tree::{
        descendants_of, read_proc_info_at, scan_processes_at, task_comm_at,
        thread_ids_of_at_limited,
    },
};

fn apply_community_rules_to_unknown_task(
    task: &mut TaskInfo,
    proc_info: &ProcInfo,
    db: &CommunityRulesDb,
) {
    if task.class != TaskClass::Unknown {
        return;
    }

    let identity = CommunityProcessIdentity {
        thread_comm: task.comm.as_str(),
        process_comm: proc_info.comm.as_str(),
        cmdline: proc_info.cmdline.as_str(),
        exe_path: proc_info.exe_path.as_str(),
        cgroup_path: proc_info.cgroup_path.as_str(),
    };

    if let Some(hit) = classify_process_identity_with_db(db, &identity) {
        log::debug!(
            "community_rules_classified tid={} pid={} class={} rule={} source={} reason={}",
            task.tid,
            task.process_pid,
            hit.class,
            hit.rule_name,
            hit.source_path,
            hit.reason
        );
        task.class = hit.class;
    }
}

/// Collect a snapshot of tasks based on the provided input criteria.
///
/// This function scans the process tree and filters tasks according to the [TargetSnapshotInput].
pub fn target_snapshot(input: TargetSnapshotInput) -> TargetSnapshot {
    let TargetSnapshotInput {
        proc_root,
        manual_pids,
        tree_pids,
        cgroup_path,
        exclude_tree_pids,
        filters,
        keep_missing_pid,
        cache,
        previous_tasks,
        max_scan_duration,
        max_threads_per_process,
        community_rules,
    } = input;

    let max_scan_duration =
        max_scan_duration.unwrap_or(Duration::from_millis(DEFAULT_MAX_PROC_SCAN_MS));
    let max_threads_per_process =
        max_threads_per_process.unwrap_or(DEFAULT_MAX_THREADS_PER_PROCESS);
    let budget = ScanBudget::new(max_scan_duration, max_threads_per_process);
    let mut budget_report = ScanBudgetReport::default();

    let mut local_cache = ProcessCache::default();
    let cache = cache.unwrap_or(&mut local_cache);
    let default_filters = TaskFilters::default();
    let filters = filters.unwrap_or(&default_filters);

    let processes = scan_processes_at(proc_root, cache, &budget, &mut budget_report);
    let mut requested_roots = BTreeSet::new();
    let mut process_roots = BTreeSet::new();
    let mut cgroup_roots = BTreeSet::new();
    let mut unresolved_manual_pids = Vec::new();
    let mut missing_manual_pids = Vec::new();

    let mut children_by_parent: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for proc_info in processes.values() {
        children_by_parent
            .entry(proc_info.ppid)
            .or_default()
            .push(proc_info.pid);
    }

    if let Some(cgroup_path) = cgroup_path {
        collect_cgroup_pids_at(cgroup_path, &mut cgroup_roots);
        for pid in &cgroup_roots {
            if processes.contains_key(pid) {
                requested_roots.insert(*pid);
                process_roots.insert(*pid);
            } else {
                unresolved_manual_pids.push(*pid);
            }
        }
    }

    for pid in manual_pids {
        if processes.contains_key(pid) {
            requested_roots.insert(*pid);
            process_roots.insert(*pid);
        } else {
            unresolved_manual_pids.push(*pid);
        }
    }

    for root_pid in tree_pids {
        requested_roots.insert(*root_pid);
        if processes.contains_key(root_pid) {
            process_roots.insert(*root_pid);
            process_roots.extend(descendants_of(*root_pid, &children_by_parent));
        }
    }

    let mut excluded_pids = BTreeSet::new();
    for root_pid in exclude_tree_pids {
        if processes.contains_key(root_pid) {
            excluded_pids.insert(*root_pid);
            excluded_pids.extend(descendants_of(*root_pid, &children_by_parent));
        }
    }
    process_roots.retain(|pid| !excluded_pids.contains(pid));

    let mut tasks = expand_tasks_at(
        proc_root,
        &process_roots,
        &processes,
        previous_tasks,
        &budget,
        &mut budget_report,
    );

    if !unresolved_manual_pids.is_empty() {
        let thread_owner_by_tid =
            thread_owner_index_at(proc_root, &processes, &budget, &mut budget_report);
        for tid in unresolved_manual_pids {
            if let Some(process_pid) = thread_owner_by_tid.get(&tid).copied()
                && !excluded_pids.contains(&process_pid)
                && let Some(proc_info) = processes.get(&process_pid)
            {
                requested_roots.insert(process_pid);
                tasks.insert(
                    tid,
                    task_info_from_tid_at(proc_root, tid, process_pid, proc_info, previous_tasks),
                );
                continue;
            }

            log::warn!("manual_pid_not_found pid={tid}");
            if keep_missing_pid {
                requested_roots.insert(tid);
                missing_manual_pids.push(tid);
            }
        }
    }

    for pid in missing_manual_pids {
        let task = TaskInfo {
            tid: pid,
            process_pid: pid,
            process_ppid: 0,
            comm: "?".to_owned(),
            process_comm: std::sync::Arc::from("?"),
            process_starttime_ticks: None,
            task_starttime_ticks: None,
            exe_dev: None,
            exe_ino: None,
            class: TaskClass::Unknown,
            sched_policy: None,
            from_cgroup: false,
        };
        if filters.allows(&task) {
            tasks.insert(pid, task);
        }
    }

    for task in tasks.values_mut() {
        if cgroup_roots.contains(&task.tid) {
            task.from_cgroup = true;
        }

        if task.class == TaskClass::Unknown
            && let Some(db) = community_rules
            && let Some(proc_info) = processes.get(&task.process_pid)
        {
            apply_community_rules_to_unknown_task(task, proc_info, db);
        }

        if task.class == TaskClass::Unknown && !requested_roots.contains(&task.process_pid) {
            task.class = TaskClass::Helper;
        }
    }

    tasks.retain(|_, task| filters.allows(task));

    TargetSnapshot {
        process_roots,
        tasks,
        budget_report,
    }
}

pub fn diff_tasks_ref<'a>(
    old_tasks: &'a BTreeMap<u32, TaskInfo>,
    new_tasks: &'a BTreeMap<u32, TaskInfo>,
) -> Vec<TargetDiffRef<'a>> {
    let mut diffs = Vec::new();

    for (tid, task) in new_tasks {
        if !old_tasks.contains_key(tid) {
            diffs.push(TargetDiffRef {
                action: TargetDiffAction::Added,
                task,
            });
        }
    }

    for (tid, task) in old_tasks {
        if !new_tasks.contains_key(tid) {
            diffs.push(TargetDiffRef {
                action: TargetDiffAction::Removed,
                task,
            });
        }
    }

    diffs.sort_by_key(|diff| {
        let action_rank = match diff.action {
            TargetDiffAction::Removed => 0,
            TargetDiffAction::Added => 1,
        };
        (action_rank, diff.task.tid)
    });

    diffs
}

pub fn find_auto_target_pids(proc_root: &Path) -> Vec<(u32, TaskClass)> {
    let mut candidates: BTreeMap<TaskClass, Vec<u32>> = BTreeMap::new();

    if let Ok(entries) = fs::read_dir(proc_root) {
        for entry in entries.flatten() {
            let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else {
                continue;
            };

            let Some(proc_info) = read_proc_info_at(proc_root, pid) else {
                continue;
            };

            let class = classify_task_with_context(
                &proc_info.comm,
                &proc_info.comm,
                &proc_info.cmdline,
                &proc_info.exe_path,
                &proc_info.cgroup_path,
                proc_info.sched_policy,
            );
            match class {
                TaskClass::GameScope
                | TaskClass::SteamRuntime
                | TaskClass::Game
                | TaskClass::Launcher => {
                    candidates.entry(class).or_default().push(pid);
                }
                _ => {}
            }
        }
    }

    let priorities = [
        TaskClass::GameScope,
        TaskClass::SteamRuntime,
        TaskClass::Game,
        TaskClass::Launcher,
    ];

    for class in priorities {
        if let Some(pids) = candidates.get(&class) {
            // Special-case SteamRuntime: prefer an exact game-like cmdline under
            // steamapps/common, otherwise prefer the newest (highest) PID to
            // avoid selecting broad persistent runtime trees.
            if class == TaskClass::SteamRuntime {
                let mut game_like: Vec<(u32, TaskClass)> = Vec::new();
                for pid in pids {
                    if let Some(proc_info) = read_proc_info_at(proc_root, *pid)
                        && contains_likely_game_cmdline(&proc_info.cmdline)
                    {
                        game_like.push((*pid, class));
                    }
                }
                if !game_like.is_empty() {
                    game_like.sort_unstable_by_key(|(pid, _)| std::cmp::Reverse(*pid));
                    return game_like;
                }

                let mut result: Vec<_> = pids.iter().map(|pid| (*pid, class)).collect();
                result.sort_unstable_by_key(|(pid, _)| std::cmp::Reverse(*pid));
                // Return only the newest PID to avoid overly broad selections.
                if let Some(first) = result.into_iter().next() {
                    return vec![first];
                }
                return Vec::new();
            }

            let mut result: Vec<_> = pids.iter().map(|pid| (*pid, class)).collect();
            result.sort_unstable_by_key(|(pid, _)| std::cmp::Reverse(*pid));
            return result;
        }
    }

    Vec::new()
}

pub fn expand_tasks_at(
    proc_root: &Path,
    process_pids: &BTreeSet<u32>,
    processes: &BTreeMap<u32, ProcInfo>,
    previous_tasks: Option<&BTreeMap<u32, TaskInfo>>,
    budget: &ScanBudget,
    budget_report: &mut ScanBudgetReport,
) -> BTreeMap<u32, TaskInfo> {
    let mut tasks = BTreeMap::new();

    for pid in process_pids {
        let Some(proc_info) = processes.get(pid) else {
            continue;
        };

        let tids = thread_ids_of_at_limited(
            proc_root,
            *pid,
            budget.max_threads_per_process(),
            budget_report,
        );

        if tids.is_empty() {
            tasks.insert(
                *pid,
                task_info_from_proc(proc_root, *pid, *pid, &proc_info.comm, proc_info),
            );
            continue;
        }

        for tid in tids {
            tasks.insert(
                tid,
                task_info_from_tid_at(proc_root, tid, *pid, proc_info, previous_tasks),
            );
        }
    }

    tasks
}

fn thread_owner_index_at(
    proc_root: &Path,
    processes: &BTreeMap<u32, ProcInfo>,
    budget: &ScanBudget,
    budget_report: &mut ScanBudgetReport,
) -> BTreeMap<u32, u32> {
    let mut owners = BTreeMap::new();
    for pid in processes.keys() {
        let tids = thread_ids_of_at_limited(
            proc_root,
            *pid,
            budget.max_threads_per_process(),
            budget_report,
        );
        for tid in tids {
            owners.insert(tid, *pid);
        }
    }
    owners
}

fn task_info_from_tid_at(
    proc_root: &Path,
    tid: u32,
    process_pid: u32,
    proc_info: &ProcInfo,
    previous_tasks: Option<&BTreeMap<u32, TaskInfo>>,
) -> TaskInfo {
    let (task_starttime_ticks, sched_policy) =
        task_starttime_and_policy_at(proc_root, process_pid, tid, proc_info);
    let comm = task_comm_at(proc_root, process_pid, tid)
        .or_else(|| {
            previous_tasks
                .and_then(|tasks| tasks.get(&tid))
                .filter(|prev| {
                    prev.process_pid == process_pid
                        && prev.task_starttime_ticks == task_starttime_ticks
                })
                .map(|prev| prev.comm.clone())
        })
        .unwrap_or_else(|| proc_info.comm.clone());

    task_info_from_parts(
        tid,
        process_pid,
        &comm,
        proc_info,
        task_starttime_ticks,
        sched_policy,
    )
}

fn task_info_from_proc(
    proc_root: &Path,
    tid: u32,
    process_pid: u32,
    comm: &str,
    proc_info: &ProcInfo,
) -> TaskInfo {
    let (task_starttime_ticks, sched_policy) =
        task_starttime_and_policy_at(proc_root, process_pid, tid, proc_info);

    task_info_from_parts(
        tid,
        process_pid,
        comm,
        proc_info,
        task_starttime_ticks,
        sched_policy,
    )
}

fn task_info_from_parts(
    tid: u32,
    process_pid: u32,
    comm: &str,
    proc_info: &ProcInfo,
    task_starttime_ticks: Option<u64>,
    sched_policy: Option<u32>,
) -> TaskInfo {
    TaskInfo {
        tid,
        process_pid,
        process_ppid: proc_info.ppid,
        comm: comm.to_owned(),
        process_comm: std::sync::Arc::from(proc_info.comm.clone()),
        process_starttime_ticks: proc_info.starttime_ticks,
        task_starttime_ticks,
        exe_dev: proc_info.exe_dev,
        exe_ino: proc_info.exe_ino,
        class: classify_task_with_context(
            comm,
            &proc_info.comm,
            &proc_info.cmdline,
            &proc_info.exe_path,
            &proc_info.cgroup_path,
            sched_policy,
        ),
        sched_policy,
        from_cgroup: false,
    }
}

fn task_starttime_and_policy_at(
    proc_root: &Path,
    process_pid: u32,
    tid: u32,
    proc_info: &ProcInfo,
) -> (Option<u64>, Option<u32>) {
    if tid == process_pid {
        return (proc_info.starttime_ticks, proc_info.sched_policy);
    }

    let stat_path = proc_root
        .join(process_pid.to_string())
        .join("task")
        .join(tid.to_string())
        .join("stat");

    stat_starttime_and_policy_at(&stat_path)
}

fn stat_starttime_at(path: &Path) -> Option<u64> {
    stat_starttime_and_policy_at(path).0
}

pub fn process_starttime_at(proc_root: &Path, pid: u32) -> Option<u64> {
    stat_starttime_at(&proc_root.join(pid.to_string()).join("stat"))
}

pub fn parse_proc_stat_starttime(stat: &str) -> Option<u64> {
    let (_, after_comm) = stat.rsplit_once(") ")?;
    after_comm.split_whitespace().nth(19)?.parse().ok()
}

pub(crate) fn stat_starttime_and_policy_at(path: &Path) -> (Option<u64>, Option<u32>) {
    let Ok(stat) = fs::read_to_string(path) else {
        return (None, None);
    };
    (
        parse_proc_stat_starttime(&stat),
        parse_proc_stat_policy(&stat),
    )
}

pub fn parse_proc_stat_policy(stat: &str) -> Option<u32> {
    let (_, after_comm) = stat.rsplit_once(") ")?;
    after_comm.split_whitespace().nth(38)?.parse().ok()
}
