//! Procfs process and task readers.
//!
//! Owns bounded process scans, proc status/stat/cmdline/exe reads, thread ID discovery,
//! process-tree descendant expansion, and task comm reads. Does not own target snapshot selection,
//! built-in classification policy, tree rendering, or cgroup traversal.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    os::unix::fs::MetadataExt,
    path::Path,
};

use crate::{
    process::{
        model::{CachedProcInfo, ProcInfo, ProcessCache, ScanBudget, ScanBudgetReport},
        snapshot::stat_starttime_and_policy_at,
    },
    procfs as procfs_helpers,
};

pub fn scan_processes_at(
    proc_root: &Path,
    cache: &mut ProcessCache,
    budget: &ScanBudget,
    budget_report: &mut ScanBudgetReport,
) -> BTreeMap<u32, ProcInfo> {
    let start = std::time::Instant::now();
    let mut processes = BTreeMap::new();

    cache.generation = cache.generation.saturating_add(1);
    let generation = cache.generation;

    let Ok(entries) = fs::read_dir(proc_root) else {
        return processes;
    };

    for entry in entries.flatten() {
        if budget.expired() {
            budget_report.scan_timed_out = true;
            log::warn!(
                "process_scan_budget_exceeded elapsed_ms={} max_ms={} proc_entries_seen={}",
                budget.started_at.elapsed().as_millis(),
                budget.max_duration.as_millis(),
                budget_report.proc_entries_seen
            );
            break;
        }

        budget_report.proc_entries_seen += 1;

        if let Some(max_proc_entries) = budget.max_proc_entries()
            && budget_report.proc_entries_seen > max_proc_entries
        {
            budget_report.proc_entries_skipped += 1;
            break;
        }

        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };

        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };

        let Ok(metadata) = entry.metadata() else {
            continue;
        };

        let ctime_sec = metadata.ctime();
        let ctime_nsec = metadata.ctime_nsec();

        if let Some(cached) = cache.entries.get(&pid) {
            let cache_fresh_enough =
                generation.saturating_sub(cached.scan_generation) <= cache.max_cached_generations;

            if cache_fresh_enough
                && cached.ctime_sec == ctime_sec
                && cached.ctime_nsec == ctime_nsec
            {
                processes.insert(pid, cached.info.clone());
                continue;
            }
        }

        if let Some(info) = read_proc_info_at(proc_root, pid) {
            cache.entries.insert(
                pid,
                CachedProcInfo {
                    ctime_sec,
                    ctime_nsec,
                    scan_generation: generation,
                    info: info.clone(),
                },
            );
            processes.insert(pid, info);
        }
    }

    cache.entries.retain(|pid, _| processes.contains_key(pid));

    let elapsed = start.elapsed();
    if elapsed.as_millis() > 50 {
        log::warn!(
            "scan_processes_slow elapsed={:?} count={} cache_size={}",
            elapsed,
            processes.len(),
            cache.entries.len()
        );
    }

    processes
}

pub(crate) fn read_proc_info_at(proc_root: &Path, pid: u32) -> Option<ProcInfo> {
    let status_path = proc_root.join(pid.to_string()).join("status");
    let status = fs::read_to_string(status_path).ok()?;

    let mut ppid = None;
    let mut comm = None;

    for line in status.lines() {
        if let Some(value) = line.strip_prefix("Name:") {
            comm = Some(value.trim().to_owned());
        } else if let Some(value) = line.strip_prefix("PPid:") {
            ppid = value.trim().parse::<u32>().ok();
        }
    }

    let cmdline_path = proc_root.join(pid.to_string()).join("cmdline");
    let cmdline = fs::read(cmdline_path)
        .ok()
        .map(|bytes| {
            bytes
                .split(|b| *b == 0)
                .filter(|part| !part.is_empty())
                .map(|part| String::from_utf8_lossy(part))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();

    let stat_path = proc_root.join(pid.to_string()).join("stat");
    let (starttime_ticks, sched_policy) = stat_starttime_and_policy_at(&stat_path);
    let exe_path = exe_path_at(proc_root, pid).unwrap_or_default();
    let cgroup_path = read_proc_cgroup_path_at(proc_root, pid);
    let (exe_dev, exe_ino) = exe_metadata_at(proc_root, pid)
        .map(|metadata| (Some(metadata.dev()), Some(metadata.ino())))
        .unwrap_or((None, None));

    Some(ProcInfo {
        pid,
        ppid: ppid?,
        comm: comm?,
        cmdline,
        exe_path,
        cgroup_path,
        starttime_ticks,
        exe_dev,
        exe_ino,
        sched_policy,
    })
}

fn exe_path_at(proc_root: &Path, pid: u32) -> Option<String> {
    fs::read_link(proc_root.join(pid.to_string()).join("exe"))
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
}

fn read_proc_cgroup_path_at(proc_root: &Path, pid: u32) -> String {
    let cgroup_path = proc_root.join(pid.to_string()).join("cgroup");
    let Ok(contents) = fs::read_to_string(cgroup_path) else {
        return String::new();
    };

    contents
        .lines()
        .filter_map(|line| line.rsplit_once(':').map(|(_, path)| path.trim()))
        .filter(|path| !path.is_empty())
        .max_by_key(|path| path.len())
        .unwrap_or("")
        .to_owned()
}

fn exe_metadata_at(proc_root: &Path, pid: u32) -> Option<fs::Metadata> {
    fs::metadata(proc_root.join(pid.to_string()).join("exe")).ok()
}

pub fn descendants_of(
    root_pid: u32,
    children_by_parent: &BTreeMap<u32, Vec<u32>>,
) -> BTreeSet<u32> {
    let mut result = BTreeSet::new();
    let mut queue = VecDeque::new();

    result.insert(root_pid);
    queue.push_back(root_pid);

    while let Some(parent) = queue.pop_front() {
        if let Some(children) = children_by_parent.get(&parent) {
            for child in children {
                if result.insert(*child) {
                    queue.push_back(*child);
                }
            }
        }
    }

    result
}

pub fn thread_ids_of_at_limited(
    proc_root: &Path,
    pid: u32,
    max_threads: usize,
    report: &mut ScanBudgetReport,
) -> BTreeSet<u32> {
    let mut tids = BTreeSet::new();
    let task_path = proc_root.join(pid.to_string()).join("task");

    let Ok(entries) = fs::read_dir(task_path) else {
        return tids;
    };

    for entry in entries.flatten() {
        report.thread_entries_seen += 1;

        if tids.len() >= max_threads {
            report.thread_entries_skipped += 1;
            break;
        }

        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };

        let Ok(tid) = name.parse::<u32>() else {
            continue;
        };

        tids.insert(tid);
    }

    if report.thread_entries_skipped > 0 {
        report.processes_thread_limited += 1;
    }

    tids
}

pub fn thread_ids_of_at(proc_root: &Path, pid: u32) -> BTreeSet<u32> {
    let mut report = ScanBudgetReport::default();
    thread_ids_of_at_limited(proc_root, pid, usize::MAX, &mut report)
}

pub fn task_comm_at(proc_root: &Path, pid: u32, tid: u32) -> Option<String> {
    let comm_path = proc_root
        .join(pid.to_string())
        .join("task")
        .join(tid.to_string())
        .join("comm");

    procfs_helpers::read_procfs_to_string(&comm_path)
        .ok()
        .map(|comm| comm.trim().to_owned())
        .filter(|comm| !comm.is_empty())
}
