use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt, fs,
    os::unix::fs::MetadataExt,
    path::Path,
};

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default)]
pub struct ProcessCache {
    pub entries: BTreeMap<u32, CachedProcInfo>,
}

impl ProcessCache {
    pub fn invalidate(&mut self, pid: u32) {
        self.entries.remove(&pid);
    }
}

#[derive(Debug, Clone)]
pub struct CachedProcInfo {
    pub ctime_sec: i64,
    pub ctime_nsec: i64,
    pub info: ProcInfo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    pub comm: String,
    pub cmdline: String,
    pub starttime_ticks: Option<u64>,
    pub exe_dev: Option<u64>,
    pub exe_ino: Option<u64>,
    pub sched_policy: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum TaskClass {
    Game,
    GameHelper,
    Launcher,
    WineServer,
    GameScope,
    Compositor,
    SteamRuntime,
    Helper,
    #[default]
    Unknown,
}

impl TaskClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Game => "Game",
            Self::GameHelper => "GameHelper",
            Self::Launcher => "Launcher",
            Self::WineServer => "WineServer",
            Self::GameScope => "GameScope",
            Self::Compositor => "Compositor",
            Self::SteamRuntime => "SteamRuntime",
            Self::Helper => "Helper",
            Self::Unknown => "Unknown",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "game" => Some(Self::Game),
            "gamehelper" => Some(Self::GameHelper),
            "launcher" => Some(Self::Launcher),
            "wineserver" => Some(Self::WineServer),
            "gamescope" => Some(Self::GameScope),
            "compositor" => Some(Self::Compositor),
            "steamruntime" => Some(Self::SteamRuntime),
            "helper" => Some(Self::Helper),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

impl fmt::Display for TaskClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskInfo {
    pub tid: u32,
    pub process_pid: u32,
    pub process_ppid: u32,
    pub comm: String,
    pub process_comm: std::sync::Arc<str>,
    pub process_starttime_ticks: Option<u64>,
    pub task_starttime_ticks: Option<u64>,
    pub exe_dev: Option<u64>,
    pub exe_ino: Option<u64>,
    pub class: TaskClass,
    pub sched_policy: Option<u32>,
    pub from_cgroup: bool,
}

pub fn same_logical_task(left: &TaskInfo, right: &TaskInfo) -> bool {
    if left.process_pid != right.process_pid {
        return false;
    }

    if let (Some(lp), Some(rp), Some(lt), Some(rt)) = (
        left.process_starttime_ticks,
        right.process_starttime_ticks,
        left.task_starttime_ticks,
        right.task_starttime_ticks,
    ) {
        return lp == rp && lt == rt;
    }

    if let (Some(ld), Some(rd), Some(li), Some(ri)) =
        (left.exe_dev, right.exe_dev, left.exe_ino, right.exe_ino)
    {
        return ld == rd && li == ri && left.comm == right.comm;
    }

    false
}

pub fn sched_policy_name(policy: u32) -> Option<&'static str> {
    match policy {
        0 => Some("SCHED_NORMAL"),
        1 => Some("SCHED_FIFO"),
        2 => Some("SCHED_RR"),
        3 => Some("SCHED_BATCH"),
        5 => Some("SCHED_IDLE"),
        6 => Some("SCHED_DEADLINE"),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetSnapshot {
    pub process_roots: BTreeSet<u32>,
    pub tasks: BTreeMap<u32, TaskInfo>,
}

/// Input parameters for [target_snapshot].
///
/// This struct uses the builder pattern to allow flexible configuration of what
/// processes and tasks should be included in the snapshot.
pub struct TargetSnapshotInput<'a> {
    /// The root directory for the proc filesystem. Defaults to `/proc`.
    pub proc_root: &'a Path,
    /// A list of specific PIDs to include.
    pub manual_pids: &'a [u32],
    /// A list of root PIDs whose entire descendant trees should be included.
    pub tree_pids: &'a [u32],
    /// An optional cgroup path. All tasks in this cgroup and its sub-cgroups will be included.
    pub cgroup_path: Option<&'a Path>,
    /// A list of root PIDs whose descendant trees should be excluded, even if they would
    /// otherwise be included via [Self::tree_pids] or [Self::cgroup_path].
    pub exclude_tree_pids: &'a [u32],
    /// Optional filters to exclude tasks based on their command name.
    pub filters: Option<&'a TaskFilters>,
    /// If true, PIDs in [Self::manual_pids] that are not found will still be included
    /// in the output with [TaskClass::Unknown].
    pub keep_missing_pid: bool,
    /// An optional cache to speed up repeated scans.
    pub cache: Option<&'a mut ProcessCache>,
    /// An optional map of previous tasks to help preserve task information.
    pub previous_tasks: Option<&'a BTreeMap<u32, TaskInfo>>,
}

impl<'a> Default for TargetSnapshotInput<'a> {
    fn default() -> Self {
        Self {
            proc_root: Path::new("/proc"),
            manual_pids: &[],
            tree_pids: &[],
            cgroup_path: None,
            exclude_tree_pids: &[],
            filters: None,
            keep_missing_pid: false,
            cache: None,
            previous_tasks: None,
        }
    }
}

impl<'a> TargetSnapshotInput<'a> {
    #[cfg(test)]
    pub fn proc_root(mut self, path: &'a Path) -> Self {
        self.proc_root = path;
        self
    }

    pub fn manual_pids(mut self, pids: &'a [u32]) -> Self {
        self.manual_pids = pids;
        self
    }

    pub fn tree_pids(mut self, pids: &'a [u32]) -> Self {
        self.tree_pids = pids;
        self
    }

    pub fn cgroup_path(mut self, path: Option<&'a Path>) -> Self {
        self.cgroup_path = path;
        self
    }

    pub fn exclude_tree_pids(mut self, pids: &'a [u32]) -> Self {
        self.exclude_tree_pids = pids;
        self
    }

    pub fn filters(mut self, filters: &'a TaskFilters) -> Self {
        self.filters = Some(filters);
        self
    }

    pub fn keep_missing_pid(mut self, keep: bool) -> Self {
        self.keep_missing_pid = keep;
        self
    }

    pub fn cache(mut self, cache: &'a mut ProcessCache) -> Self {
        self.cache = Some(cache);
        self
    }

    pub fn previous_tasks(mut self, tasks: Option<&'a BTreeMap<u32, TaskInfo>>) -> Self {
        self.previous_tasks = tasks;
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskFilters {
    pub include_comm: Vec<CompiledPattern>,
    pub exclude_comm: Vec<CompiledPattern>,
}

#[derive(Clone, Debug, Default)]
pub struct CompiledPattern {
    pub raw: String,
    pub regex: Option<Regex>,
}

impl CompiledPattern {
    pub fn new(raw: String) -> anyhow::Result<Self> {
        let regex = if let Some(inner) = raw.strip_prefix('/').and_then(|s| s.strip_suffix('/')) {
            Some(
                Regex::new(inner)
                    .map_err(|e| anyhow::anyhow!("invalid regex in pattern '{}': {e}", raw))?,
            )
        } else {
            None
        };

        Ok(Self { raw, regex })
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    pub fn matches(&self, value: &str) -> bool {
        if let Some(regex) = &self.regex {
            regex.is_match(value)
        } else {
            value
                .to_ascii_lowercase()
                .contains(&self.raw.to_ascii_lowercase())
        }
    }
}

impl PartialEq for CompiledPattern {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl Eq for CompiledPattern {}

impl TaskFilters {
    fn allows(&self, task: &TaskInfo) -> bool {
        if self
            .exclude_comm
            .iter()
            .any(|pattern| comm_pattern_matches(pattern, task))
        {
            return false;
        }

        self.include_comm.is_empty()
            || self
                .include_comm
                .iter()
                .any(|pattern| comm_pattern_matches(pattern, task))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetDiffAction {
    Added,
    Removed,
}

#[derive(Debug, Clone, Copy)]
pub struct TargetDiffRef<'a> {
    pub action: TargetDiffAction,
    pub task: &'a TaskInfo,
}

pub fn scan_processes_at(proc_root: &Path, cache: &mut ProcessCache) -> BTreeMap<u32, ProcInfo> {
    let start = std::time::Instant::now();
    let mut processes = BTreeMap::new();

    let Ok(entries) = fs::read_dir(proc_root) else {
        return processes;
    };

    for entry in entries.flatten() {
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

        if let Some(cached) = cache.entries.get(&pid)
            && cached.ctime_sec == ctime_sec
            && cached.ctime_nsec == ctime_nsec
        {
            processes.insert(pid, cached.info.clone());
            continue;
        }

        if let Some(info) = read_proc_info_at(proc_root, pid) {
            cache.entries.insert(
                pid,
                CachedProcInfo {
                    ctime_sec,
                    ctime_nsec,
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

fn read_proc_info_at(proc_root: &Path, pid: u32) -> Option<ProcInfo> {
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
    let (exe_dev, exe_ino) = exe_metadata_at(proc_root, pid)
        .map(|metadata| (Some(metadata.dev()), Some(metadata.ino())))
        .unwrap_or((None, None));

    Some(ProcInfo {
        pid,
        ppid: ppid?,
        comm: comm?,
        cmdline,
        starttime_ticks,
        exe_dev,
        exe_ino,
        sched_policy,
    })
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

pub fn thread_ids_of_at(proc_root: &Path, pid: u32) -> BTreeSet<u32> {
    let mut tids = BTreeSet::new();
    let task_path = proc_root.join(pid.to_string()).join("task");

    let Ok(entries) = fs::read_dir(task_path) else {
        return tids;
    };

    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };

        let Ok(tid) = name.parse::<u32>() else {
            continue;
        };

        tids.insert(tid);
    }

    tids
}

pub fn task_comm_at(proc_root: &Path, pid: u32, tid: u32) -> Option<String> {
    let comm_path = proc_root
        .join(pid.to_string())
        .join("task")
        .join(tid.to_string())
        .join("comm");

    fs::read_to_string(comm_path)
        .ok()
        .map(|comm| comm.trim().to_owned())
        .filter(|comm| !comm.is_empty())
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
    } = input;

    let mut local_cache = ProcessCache::default();
    let cache = cache.unwrap_or(&mut local_cache);
    let default_filters = TaskFilters::default();
    let filters = filters.unwrap_or(&default_filters);

    let processes = scan_processes_at(proc_root, cache);
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

    let mut tasks = expand_tasks_at(proc_root, &process_roots, &processes, previous_tasks);

    if !unresolved_manual_pids.is_empty() {
        let thread_owner_by_tid = thread_owner_index_at(proc_root, &processes);
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

        if task.class == TaskClass::Unknown && !requested_roots.contains(&task.process_pid) {
            task.class = TaskClass::Helper;
        }
    }

    tasks.retain(|_, task| filters.allows(task));

    TargetSnapshot {
        process_roots,
        tasks,
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

            let class = classify_task(&proc_info.comm, &proc_info.comm, &proc_info.cmdline);
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
) -> BTreeMap<u32, TaskInfo> {
    let mut tasks = BTreeMap::new();

    for pid in process_pids {
        let Some(proc_info) = processes.get(pid) else {
            continue;
        };

        let tids = thread_ids_of_at(proc_root, *pid);

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
) -> BTreeMap<u32, u32> {
    let mut owners = BTreeMap::new();
    for pid in processes.keys() {
        for tid in thread_ids_of_at(proc_root, *pid) {
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
        class: classify_task(comm, &proc_info.comm, &proc_info.cmdline),
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

fn stat_starttime_and_policy_at(path: &Path) -> (Option<u64>, Option<u32>) {
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

fn comm_pattern_matches(pattern: &CompiledPattern, task: &TaskInfo) -> bool {
    pattern.matches(&task.comm) || pattern.matches(&task.process_comm)
}

/// Classification priority (highest to lowest):
/// 1. GameScope: The compositor running the game.
/// 2. Compositor: System-level window managers and compositors.
/// 3. WineServer: The core Wine/Proton server process.
/// 4. SteamRuntime: Container and runtime tools (pressure-vessel, bwrap, etc).
/// 5. Launcher: Game-specific or platform launchers (Epic, EA, Ubisoft, etc).
/// 6. Helper: Known system or background helpers (svchost, steamwebhelper, etc).
/// 7. Game: Likely game process based on path patterns (steamapps/common, etc).
/// 8. GameHelper: Any other .exe process not caught by the above.
pub fn classify_task(comm: &str, process_comm: &str, cmdline: &str) -> TaskClass {
    let comm = comm.to_ascii_lowercase();
    let process_comm = process_comm.to_ascii_lowercase();
    let cmdline = cmdline.to_ascii_lowercase();

    for rule in CLASSIFICATION_RULES {
        if rule.matches(&comm, &process_comm, &cmdline) {
            return rule.class;
        }
    }

    TaskClass::Unknown
}

#[derive(Debug, Clone, Copy)]
enum ClassificationMatchType {
    /// Match any of the tokens in any of the fields (comm, process_comm, cmdline).
    AnyField(&'static [&'static str]),
    /// Match only if it looks like a game (specific path patterns).
    LikelyGame,
    /// Match any .exe process.
    AnyExe,
}

struct ClassificationRule {
    class: TaskClass,
    match_type: ClassificationMatchType,
}

impl ClassificationRule {
    fn matches(&self, comm: &str, process_comm: &str, cmdline: &str) -> bool {
        match self.match_type {
            ClassificationMatchType::AnyField(tokens) => tokens.iter().any(|&token| {
                comm.contains(token) || process_comm.contains(token) || cmdline.contains(token)
            }),
            ClassificationMatchType::LikelyGame => contains_likely_game_cmdline(cmdline),
            ClassificationMatchType::AnyExe => {
                comm.contains(".exe") || process_comm.contains(".exe") || cmdline.contains(".exe")
            }
        }
    }
}

const CLASSIFICATION_RULES: &[ClassificationRule] = &[
    ClassificationRule {
        class: TaskClass::GameScope,
        match_type: ClassificationMatchType::AnyField(&["gamescope"]),
    },
    ClassificationRule {
        class: TaskClass::Compositor,
        match_type: ClassificationMatchType::AnyField(&[
            "sway",
            "kwin",
            "kwin_wayland",
            "mutter",
            "gnome-shell",
            "steamcompmgr",
        ]),
    },
    ClassificationRule {
        class: TaskClass::WineServer,
        match_type: ClassificationMatchType::AnyField(&["wineserver"]),
    },
    ClassificationRule {
        class: TaskClass::SteamRuntime,
        match_type: ClassificationMatchType::AnyField(&[
            "pressure-vessel",
            "pv-",
            "steam-runtime",
            "soldier",
            "sniper",
            "srt-bwrap",
            "xalia",
        ]),
    },
    ClassificationRule {
        class: TaskClass::Launcher,
        match_type: ClassificationMatchType::AnyField(&[
            "launcher",
            "launch",
            "bootstrapper",
            "crashhandler",
            "crashreport",
            "redlauncher",
            "bethesdanetlauncher",
            "ealauncher",
            "ubisoftconnect",
            "uplay",
            "epicgameslauncher",
        ]),
    },
    ClassificationRule {
        class: TaskClass::Helper,
        match_type: ClassificationMatchType::AnyField(&[
            "reaper",
            "helper",
            "rundll32",
            "explorer.exe",
            "wine-preloader",
            "services.exe",
            "winedevice.exe",
            "svchost.exe",
            "plugplay.exe",
            "rpcss.exe",
            "tabtip.exe",
            "conhost.exe",
            "regsvr32.exe",
            "msiexec.exe",
            "steam.exe",
            "steamwebhelper.exe",
            "steamerrorreporter.exe",
        ]),
    },
    ClassificationRule {
        class: TaskClass::Game,
        match_type: ClassificationMatchType::LikelyGame,
    },
    ClassificationRule {
        class: TaskClass::GameHelper,
        match_type: ClassificationMatchType::AnyExe,
    },
];

fn contains_likely_game_cmdline(cmdline: &str) -> bool {
    if !cmdline.contains(".exe") {
        return false;
    }

    if !cmdline.contains("steamapps/common")
        && !cmdline.contains("\\steamapps\\common")
        && !cmdline.contains("/games/")
        && !cmdline.contains("\\games\\")
    {
        return false;
    }

    !contains_known_non_game_exe(cmdline)
}

fn contains_known_non_game_exe(text: &str) -> bool {
    [
        "steam.exe",
        "steamwebhelper.exe",
        "steamerrorreporter.exe",
        "xalia.exe",
        "explorer.exe",
        "services.exe",
        "winedevice.exe",
        "svchost.exe",
        "plugplay.exe",
        "rpcss.exe",
        "tabtip.exe",
        "rundll32.exe",
        "wineboot.exe",
        "winemenubuilder.exe",
        "conhost.exe",
        "regsvr32.exe",
        "msiexec.exe",
    ]
    .iter()
    .any(|name| text.contains(name))
}

pub fn render_tree(root_pid: u32) -> anyhow::Result<String> {
    render_tree_at(Path::new("/proc"), root_pid)
}

pub fn render_tree_at(proc_root: &Path, root_pid: u32) -> anyhow::Result<String> {
    let processes = scan_processes_at(proc_root, &mut ProcessCache::default());
    let Some(root) = processes.get(&root_pid) else {
        anyhow::bail!("process {root_pid} not found under {}", proc_root.display());
    };

    let mut children_by_parent: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for proc_info in processes.values() {
        children_by_parent
            .entry(proc_info.ppid)
            .or_default()
            .push(proc_info.pid);
    }

    let root_class = classify_task(&root.comm, &root.comm, &root.cmdline);
    let mut output = format!(
        "root {} {}{}\n",
        root.pid,
        root.comm,
        class_suffix(root_class)
    );

    render_children(
        proc_root,
        root_pid,
        &processes,
        &children_by_parent,
        "",
        false,
        &mut output,
    );

    Ok(output)
}

fn render_children(
    proc_root: &Path,
    pid: u32,
    processes: &BTreeMap<u32, ProcInfo>,
    children_by_parent: &BTreeMap<u32, Vec<u32>>,
    prefix: &str,
    mark_unknown_as_helper: bool,
    output: &mut String,
) {
    let mut entries = Vec::new();

    if let Some(children) = children_by_parent.get(&pid) {
        for child in children {
            if let Some(proc_info) = processes.get(child) {
                let mut class = classify_task(&proc_info.comm, &proc_info.comm, &proc_info.cmdline);
                if class == TaskClass::Unknown && mark_unknown_as_helper {
                    class = TaskClass::Helper;
                }
                entries.push(TreeEntry::Process {
                    pid: *child,
                    comm: proc_info.comm.clone(),
                    class,
                });
            }
        }
    }

    for tid in thread_ids_of_at(proc_root, pid) {
        if tid != pid && !processes.contains_key(&tid) {
            let comm = task_comm_at(proc_root, pid, tid).unwrap_or_else(|| {
                processes
                    .get(&pid)
                    .map(|proc_info| proc_info.comm.clone())
                    .unwrap_or_else(|| "?".to_owned())
            });

            let mut class = processes
                .get(&pid)
                .map(|proc_info| classify_task(&comm, &proc_info.comm, &proc_info.cmdline))
                .unwrap_or(TaskClass::Unknown);

            if class == TaskClass::Unknown && mark_unknown_as_helper {
                class = TaskClass::Helper;
            }

            entries.push(TreeEntry::Task { tid, comm, class });
        }
    }

    entries.sort_by_key(TreeEntry::id);

    for (idx, entry) in entries.iter().enumerate() {
        let is_last = idx + 1 == entries.len();
        let branch = if is_last { "└─ " } else { "├─ " };
        let child_prefix = if is_last {
            format!("{prefix}   ")
        } else {
            format!("{prefix}│  ")
        };

        match entry {
            TreeEntry::Process { pid, comm, class } => {
                output.push_str(&format!(
                    "{prefix}{branch}{pid} {comm}{}\n",
                    class_suffix(*class)
                ));
                render_children(
                    proc_root,
                    *pid,
                    processes,
                    children_by_parent,
                    &child_prefix,
                    true,
                    output,
                );
            }
            TreeEntry::Task { tid, comm, class } => {
                output.push_str(&format!(
                    "{prefix}{branch}{tid} {comm}{}\n",
                    class_suffix(*class)
                ));
            }
        }
    }
}

fn class_suffix(class: TaskClass) -> String {
    if class == TaskClass::Unknown {
        String::new()
    } else {
        format!(" [{class}]")
    }
}

enum TreeEntry {
    Process {
        pid: u32,
        comm: String,
        class: TaskClass,
    },
    Task {
        tid: u32,
        comm: String,
        class: TaskClass,
    },
}

impl TreeEntry {
    fn id(&self) -> u32 {
        match self {
            TreeEntry::Process { pid, .. } => *pid,
            TreeEntry::Task { tid, .. } => *tid,
        }
    }
}

pub fn collect_cgroup_pids_at(cgroup_path: &Path, pids: &mut BTreeSet<u32>) {
    let procs = cgroup_path.join("cgroup.procs");
    let threads = cgroup_path.join("cgroup.threads");

    let mut read_list = |file: &Path| {
        if let Ok(content) = fs::read_to_string(file) {
            for line in content.lines() {
                if let Ok(pid) = line.trim().parse::<u32>() {
                    pids.insert(pid);
                }
            }
        }
    };

    read_list(&procs);
    read_list(&threads);

    if let Ok(entries) = fs::read_dir(cgroup_path) {
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type()
                && file_type.is_dir()
            {
                collect_cgroup_pids_at(&entry.path(), pids);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn detects_gamescope_as_auto_target() {
        let dir = std::env::temp_dir().join(format!("stutter-auto-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let pid_dir = dir.join("123");
        fs::create_dir_all(&pid_dir).unwrap();
        fs::write(pid_dir.join("status"), "Name:\tgamescope\nPPid:\t1\n").unwrap();
        fs::write(pid_dir.join("cmdline"), "gamescope\0-f\0--\0steam\0").unwrap();
        fs::write(pid_dir.join("stat"), "123 (gamescope) S 1 123 0 0 -1 0 0 0 0 0 0 0 0 20 0 1 0 1000 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n").unwrap();

        let auto = find_auto_target_pids(&dir);
        assert_eq!(auto.len(), 1);
        assert_eq!(auto[0].0, 123);
        assert_eq!(auto[0].1, TaskClass::GameScope);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn prioritizes_gamescope_over_steam_runtime() {
        let dir =
            std::env::temp_dir().join(format!("stutter-auto-prio-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        // Steam Runtime (priority 2)
        let pid1 = dir.join("101");
        fs::create_dir_all(&pid1).unwrap();
        fs::write(pid1.join("status"), "Name:\tpressure-vessel\nPPid:\t1\n").unwrap();
        fs::write(pid1.join("cmdline"), "pressure-vessel-wrap\0").unwrap();
        fs::write(pid1.join("stat"), "101 (pressure-vessel) S 1 101 0 0 -1 0 0 0 0 0 0 0 0 20 0 1 0 1000 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n").unwrap();

        // GameScope (priority 1)
        let pid2 = dir.join("102");
        fs::create_dir_all(&pid2).unwrap();
        fs::write(pid2.join("status"), "Name:\tgamescope\nPPid:\t1\n").unwrap();
        fs::write(pid2.join("cmdline"), "gamescope\0").unwrap();
        fs::write(pid2.join("stat"), "102 (gamescope) S 1 102 0 0 -1 0 0 0 0 0 0 0 0 20 0 1 0 1000 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n").unwrap();

        let auto = find_auto_target_pids(&dir);
        assert_eq!(auto.len(), 1);
        assert_eq!(auto[0].0, 102);
        assert_eq!(auto[0].1, TaskClass::GameScope);

        fs::remove_dir_all(dir).ok();
    }
}
