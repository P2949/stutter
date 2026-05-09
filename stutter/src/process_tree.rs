use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt, fs,
    os::unix::fs::MetadataExt,
    path::Path,
    time::{Duration, Instant},
};

pub const DEFAULT_MAX_PROC_SCAN_MS: u64 = 50;
pub const DEFAULT_MAX_THREADS_PER_PROCESS: usize = 4096;

#[derive(Debug, Clone)]
pub struct ScanBudget {
    started_at: Instant,
    max_duration: Duration,
    max_proc_entries: Option<usize>,
    max_threads_per_process: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanBudgetReport {
    pub scan_timed_out: bool,
    pub proc_entries_seen: usize,
    pub proc_entries_skipped: usize,
    pub thread_entries_seen: usize,
    pub thread_entries_skipped: usize,
    pub processes_thread_limited: usize,
}

impl ScanBudget {
    pub fn new(max_duration: Duration, max_threads_per_process: usize) -> Self {
        Self {
            started_at: Instant::now(),
            max_duration,
            max_proc_entries: None,
            max_threads_per_process,
        }
    }

    #[allow(dead_code)]
    pub fn with_max_proc_entries(mut self, max_proc_entries: usize) -> Self {
        self.max_proc_entries = Some(max_proc_entries);
        self
    }

    pub fn default_proc_scan() -> Self {
        Self::new(
            Duration::from_millis(DEFAULT_MAX_PROC_SCAN_MS),
            DEFAULT_MAX_THREADS_PER_PROCESS,
        )
    }

    pub fn expired(&self) -> bool {
        self.started_at.elapsed() > self.max_duration
    }

    pub fn max_proc_entries(&self) -> Option<usize> {
        self.max_proc_entries
    }

    pub fn max_threads_per_process(&self) -> usize {
        self.max_threads_per_process
    }
}

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::community_rules::{
    CommunityProcessIdentity, CommunityRulesDb, classify_process_identity_with_db,
};

#[derive(Debug, Clone)]
pub struct ProcessCache {
    pub entries: BTreeMap<u32, CachedProcInfo>,
    pub generation: u64,
    pub max_cached_generations: u64,
}

impl Default for ProcessCache {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            generation: 0,
            max_cached_generations: 5,
        }
    }
}

impl ProcessCache {
    pub fn invalidate(&mut self, pid: u32) {
        self.entries.remove(&pid);
    }

    pub fn comm_for_tid(&self, tid: u32) -> Option<String> {
        self.entries.get(&tid).map(|e| e.info.comm.clone())
    }
}

#[derive(Debug, Clone)]
pub struct CachedProcInfo {
    pub ctime_sec: i64,
    pub ctime_nsec: i64,
    pub scan_generation: u64,
    pub info: ProcInfo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    pub comm: String,
    pub cmdline: String,
    pub exe_path: String,
    pub cgroup_path: String,
    pub starttime_ticks: Option<u64>,
    pub exe_dev: Option<u64>,
    pub exe_ino: Option<u64>,
    pub sched_policy: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum TaskClass {
    AudioRealtime,
    Input,
    Game,
    GameHelper,
    GameRenderThread,
    GameWorkerThread,
    Launcher,
    WineServer,
    GameScope,
    Compositor,
    BrowserForeground,
    BrowserBackground,
    BrowserRenderer,
    BrowserGpu,
    BrowserNetwork,
    BuildJob,
    Compiler,
    Linker,
    Indexer,
    PackageManager,
    Editor,
    Terminal,
    Shell,
    Media,
    Recorder,
    VirtualMachine,
    KernelThread,
    IrqThread,
    Service,
    NetworkDaemon,
    StorageDaemon,
    SteamRuntime,
    Render,
    Helper,
    #[default]
    Unknown,
}

impl TaskClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AudioRealtime => "AudioRealtime",
            Self::Input => "Input",
            Self::Game => "Game",
            Self::GameHelper => "GameHelper",
            Self::GameRenderThread => "GameRenderThread",
            Self::GameWorkerThread => "GameWorkerThread",
            Self::Launcher => "Launcher",
            Self::WineServer => "WineServer",
            Self::GameScope => "GameScope",
            Self::Compositor => "Compositor",
            Self::BrowserForeground => "BrowserForeground",
            Self::BrowserBackground => "BrowserBackground",
            Self::BrowserRenderer => "BrowserRenderer",
            Self::BrowserGpu => "BrowserGpu",
            Self::BrowserNetwork => "BrowserNetwork",
            Self::BuildJob => "BuildJob",
            Self::Compiler => "Compiler",
            Self::Linker => "Linker",
            Self::Indexer => "Indexer",
            Self::PackageManager => "PackageManager",
            Self::Editor => "Editor",
            Self::Terminal => "Terminal",
            Self::Shell => "Shell",
            Self::Media => "Media",
            Self::Recorder => "Recorder",
            Self::VirtualMachine => "VirtualMachine",
            Self::KernelThread => "KernelThread",
            Self::IrqThread => "IrqThread",
            Self::Service => "Service",
            Self::NetworkDaemon => "NetworkDaemon",
            Self::StorageDaemon => "StorageDaemon",
            Self::SteamRuntime => "SteamRuntime",
            Self::Render => "Render",
            Self::Helper => "Helper",
            Self::Unknown => "Unknown",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        let normalized = s
            .chars()
            .filter(|ch| *ch != '_' && *ch != '-' && !ch.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();

        match normalized.as_str() {
            "audiorealtime" => Some(Self::AudioRealtime),
            "input" => Some(Self::Input),
            "game" => Some(Self::Game),
            "gamehelper" => Some(Self::GameHelper),
            "gamerenderthread" => Some(Self::GameRenderThread),
            "gameworkerthread" => Some(Self::GameWorkerThread),
            "launcher" => Some(Self::Launcher),
            "wineserver" => Some(Self::WineServer),
            "gamescope" => Some(Self::GameScope),
            "compositor" => Some(Self::Compositor),
            "browserforeground" => Some(Self::BrowserForeground),
            "browserbackground" => Some(Self::BrowserBackground),
            "browserrenderer" => Some(Self::BrowserRenderer),
            "browsergpu" => Some(Self::BrowserGpu),
            "browsernetwork" => Some(Self::BrowserNetwork),
            "buildjob" => Some(Self::BuildJob),
            "compiler" => Some(Self::Compiler),
            "linker" => Some(Self::Linker),
            "indexer" => Some(Self::Indexer),
            "packagemanager" => Some(Self::PackageManager),
            "editor" => Some(Self::Editor),
            "terminal" => Some(Self::Terminal),
            "shell" => Some(Self::Shell),
            "media" => Some(Self::Media),
            "recorder" => Some(Self::Recorder),
            "virtualmachine" => Some(Self::VirtualMachine),
            "kernelthread" => Some(Self::KernelThread),
            "irqthread" => Some(Self::IrqThread),
            "service" => Some(Self::Service),
            "networkdaemon" => Some(Self::NetworkDaemon),
            "storagedaemon" => Some(Self::StorageDaemon),
            "steamruntime" => Some(Self::SteamRuntime),
            "render" => Some(Self::Render),
            "helper" => Some(Self::Helper),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

#[cfg(test)]
pub type SystemTaskClass = TaskClass;

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum PriorityBand {
    CriticalRealtime,
    ForegroundLatency,
    Interactive,
    Throughput,
    Background,
    #[default]
    Unknown,
}

#[cfg(test)]
pub fn priority_band_for_class(class: SystemTaskClass, sched_policy: Option<u32>) -> PriorityBand {
    if matches!(sched_policy, Some(1 | 2 | 6)) {
        match class {
            SystemTaskClass::AudioRealtime | SystemTaskClass::Input => {
                return PriorityBand::CriticalRealtime;
            }
            _ => {}
        }
    }

    match class {
        SystemTaskClass::AudioRealtime | SystemTaskClass::Input => PriorityBand::CriticalRealtime,

        SystemTaskClass::Game
        | SystemTaskClass::GameRenderThread
        | SystemTaskClass::GameWorkerThread
        | SystemTaskClass::GameScope
        | SystemTaskClass::WineServer
        | SystemTaskClass::Compositor
        | SystemTaskClass::BrowserForeground => PriorityBand::ForegroundLatency,

        SystemTaskClass::Editor
        | SystemTaskClass::Terminal
        | SystemTaskClass::Shell
        | SystemTaskClass::BrowserRenderer
        | SystemTaskClass::BrowserGpu
        | SystemTaskClass::BrowserNetwork
        | SystemTaskClass::Media => PriorityBand::Interactive,

        SystemTaskClass::BuildJob
        | SystemTaskClass::Compiler
        | SystemTaskClass::Linker
        | SystemTaskClass::Recorder
        | SystemTaskClass::VirtualMachine => PriorityBand::Throughput,

        SystemTaskClass::Indexer
        | SystemTaskClass::PackageManager
        | SystemTaskClass::StorageDaemon
        | SystemTaskClass::NetworkDaemon
        | SystemTaskClass::Service => PriorityBand::Background,

        _ => PriorityBand::Unknown,
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
    pub budget_report: ScanBudgetReport,
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
    /// Maximum duration allowed for scanning processes.
    pub max_scan_duration: Option<Duration>,
    /// Maximum number of threads to scan per process.
    pub max_threads_per_process: Option<usize>,
    /// Optional community rules database used only after local classification leaves a task unknown.
    pub community_rules: Option<&'a CommunityRulesDb>,
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
            max_scan_duration: None,
            max_threads_per_process: None,
            community_rules: None,
        }
    }
}

impl<'a> TargetSnapshotInput<'a> {
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

    pub fn community_rules(mut self, community_rules: Option<&'a CommunityRulesDb>) -> Self {
        self.community_rules = community_rules;
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
    pub lower_raw: String,
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

        let lower_raw = raw.to_ascii_lowercase();
        Ok(Self {
            raw,
            lower_raw,
            regex,
        })
    }

    pub fn raw(&self) -> &str {
        &self.raw
    }

    pub fn matches(&self, value: &str) -> bool {
        if let Some(regex) = &self.regex {
            regex.is_match(value)
        } else {
            value.to_ascii_lowercase().contains(&self.lower_raw)
        }
    }

    pub fn matches_with_lower(&self, value: &str, lower_value: &str) -> bool {
        if let Some(regex) = &self.regex {
            regex.is_match(value)
        } else {
            lower_value.contains(&self.lower_raw)
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
        if self.exclude_comm.is_empty() && self.include_comm.is_empty() {
            return true;
        }

        let lower_comm = task.comm.to_ascii_lowercase();
        let lower_process_comm = task.process_comm.to_ascii_lowercase();

        if self.exclude_comm.iter().any(|pattern| {
            pattern.matches_with_lower(&task.comm, &lower_comm)
                || pattern.matches_with_lower(&task.process_comm, &lower_process_comm)
        }) {
            return false;
        }

        self.include_comm.is_empty()
            || self.include_comm.iter().any(|pattern| {
                pattern.matches_with_lower(&task.comm, &lower_comm)
                    || pattern.matches_with_lower(&task.process_comm, &lower_process_comm)
            })
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

        #[allow(clippy::collapsible_if)]
        if let Some(max_proc_entries) = budget.max_proc_entries() {
            if budget_report.proc_entries_seen > max_proc_entries {
                budget_report.proc_entries_skipped += 1;
                break;
            }
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

    fs::read_to_string(comm_path)
        .ok()
        .map(|comm| comm.trim().to_owned())
        .filter(|comm| !comm.is_empty())
}

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

#[cfg(test)]
mod community_rules_process_tree_tests {
    use std::{fs, os::unix::fs::symlink, path::Path};

    use tempfile::tempdir;

    use super::*;
    use crate::community_rules::{CommunityRule, CommunityRulesFile, CommunityRulesSource};

    fn write_fake_proc_task(proc_root: &Path, pid: u32, comm: &str, exe_path: &str) {
        let proc_dir = proc_root.join(pid.to_string());
        let task_dir = proc_dir.join("task").join(pid.to_string());

        fs::create_dir_all(&task_dir).unwrap();
        fs::write(
            proc_dir.join("status"),
            format!("Name:\t{comm}\nPPid:\t1\n"),
        )
        .unwrap();
        fs::write(proc_dir.join("cmdline"), format!("{exe_path}\0--test\0")).unwrap();
        fs::write(
            proc_dir.join("cgroup"),
            "0::/user.slice/app-mystery-123.scope\n",
        )
        .unwrap();
        fs::write(task_dir.join("comm"), format!("{comm}\n")).unwrap();
        fs::write(
            proc_dir.join("stat"),
            format!(
                "{pid} ({comm}) S 1 0 0 0 0 0 0 0 0 0 0 0 0 0 20 0 1 0 12345 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n"
            ),
        )
        .unwrap();

        let _ = fs::remove_file(proc_dir.join("exe"));
        symlink(exe_path, proc_dir.join("exe")).unwrap();
    }

    fn community_rules_db_for_exe(name: &str) -> CommunityRulesDb {
        CommunityRulesDb::from_file(CommunityRulesFile {
            schema_version: 1,
            source: CommunityRulesSource {
                name: "test community rules".to_owned(),
                repo: None,
                commit: None,
                generated_at: "2026-05-09T00:00:00Z".to_owned(),
            },
            rules: vec![CommunityRule {
                name: name.to_owned(),
                normalized_name: name.to_ascii_lowercase(),
                r#type: "Game".to_owned(),
                stutter_class: "Game".to_owned(),
                confidence: 0.90,
                source_path: "test.rules".to_owned(),
                context: vec!["none".to_owned()],
                title: Some("Community Rule Test Game".to_owned()),
                source_url: None,
                comment: None,
                ambiguous: false,
            }],
        })
        .unwrap()
    }

    #[test]
    fn validation_corpus_community_rules_classifies_unknown_game() {
        let dir = tempdir().unwrap();
        let proc_root = dir.path();
        let unknown_pid = 6101;
        let already_classified_pid = 6102;
        let manual_pids = vec![unknown_pid, already_classified_pid];

        write_fake_proc_task(proc_root, unknown_pid, "mysteryproc", "/tmp/community-game");
        write_fake_proc_task(
            proc_root,
            already_classified_pid,
            "wineserver",
            "/tmp/community-game",
        );

        let without_rules = target_snapshot(
            TargetSnapshotInput::default()
                .proc_root(proc_root)
                .manual_pids(&manual_pids),
        );

        assert_eq!(
            without_rules.tasks.get(&unknown_pid).map(|task| task.class),
            Some(TaskClass::Unknown)
        );
        assert_eq!(
            without_rules
                .tasks
                .get(&already_classified_pid)
                .map(|task| task.class),
            Some(TaskClass::WineServer)
        );

        let db = community_rules_db_for_exe("community-game");
        let with_rules = target_snapshot(
            TargetSnapshotInput::default()
                .proc_root(proc_root)
                .manual_pids(&manual_pids)
                .community_rules(Some(&db)),
        );

        assert_eq!(
            with_rules.tasks.get(&unknown_pid).map(|task| task.class),
            Some(TaskClass::Game),
            "community rules should classify locally-Unknown task as Game"
        );
        assert_eq!(
            with_rules
                .tasks
                .get(&already_classified_pid)
                .map(|task| task.class),
            Some(TaskClass::WineServer),
            "community rules must not overwrite tasks already classified by local rules"
        );
    }

    #[test]
    fn target_snapshot_uses_community_rules_only_for_unknown_tasks() {
        let dir = tempdir().unwrap();
        let proc_root = dir.path();
        let pid = 4242;
        let manual_pids = vec![pid];

        write_fake_proc_task(proc_root, pid, "mysteryproc", "/tmp/community-game");

        let without_rules = target_snapshot(
            TargetSnapshotInput::default()
                .proc_root(proc_root)
                .manual_pids(&manual_pids),
        );

        assert_eq!(
            without_rules.tasks.get(&pid).map(|task| task.class),
            Some(TaskClass::Unknown)
        );

        let db = community_rules_db_for_exe("community-game");
        let input_with_rules = TargetSnapshotInput::default()
            .proc_root(proc_root)
            .manual_pids(&manual_pids)
            .community_rules(Some(&db));
        let with_rules = target_snapshot(input_with_rules);

        assert_eq!(
            with_rules.tasks.get(&pid).map(|task| task.class),
            Some(TaskClass::Game)
        );
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
    classify_task_with_context(comm, process_comm, cmdline, "", "", None)
}

pub fn classify_task_with_context(
    comm: &str,
    process_comm: &str,
    cmdline: &str,
    exe_path: &str,
    cgroup_path: &str,
    sched_policy: Option<u32>,
) -> TaskClass {
    let lower_comm = comm.to_ascii_lowercase();
    let lower_process_comm = process_comm.to_ascii_lowercase();
    let lower_cmdline = cmdline.to_ascii_lowercase();
    let lower_exe_path = exe_path.to_ascii_lowercase();
    let lower_cgroup_path = cgroup_path.to_ascii_lowercase();

    let combined = [
        lower_comm.as_str(),
        lower_process_comm.as_str(),
        lower_cmdline.as_str(),
        lower_exe_path.as_str(),
        lower_cgroup_path.as_str(),
    ]
    .join(" ");

    // 1. Critical System Threads (Kernel, Input, IRQ)
    if is_bracketed_kernel_comm(&lower_comm) {
        return TaskClass::KernelThread;
    }
    if is_input_thread(&lower_comm, &combined) {
        return TaskClass::Input;
    }
    if is_irq_thread(&lower_comm, &combined) {
        return TaskClass::IrqThread;
    }

    // 2. Audio (Realtime)
    if is_audio_realtime_comm(&lower_comm)
        || is_audio_realtime_comm(&lower_process_comm)
        || contains_any(
            &combined,
            &[
                "pipewire",
                "wireplumber",
                "pulseaudio",
                "jackd",
                "easyeffects",
            ],
        )
        || (matches!(sched_policy, Some(1 | 2 | 6)) && is_audio_looking_process(&combined))
    {
        return TaskClass::AudioRealtime;
    }

    // 3. Infrastructure (Compositors, WineServer, Steam Services)
    if lower_comm == "gamescope" || lower_process_comm == "gamescope" {
        return TaskClass::GameScope;
    }
    if lower_comm == "wineserver" || lower_process_comm == "wineserver" {
        return TaskClass::WineServer;
    }
    if contains_any(
        &lower_comm,
        &[
            "sway",
            "kwin",
            "mutter",
            "gnome-shell",
            "weston",
            "hyprland",
        ],
    ) {
        return TaskClass::Compositor;
    }
    if lower_comm == "steam"
        || lower_process_comm == "steam"
        || lower_comm == "steamwebhelper"
        || lower_process_comm == "steamwebhelper"
    {
        return TaskClass::Service;
    }

    // 4. Specialized Threads in Game/Browser (Must come before generic process match)
    let is_game = is_game_exe(&lower_exe_path)
        || is_game_comm(&lower_process_comm)
        || is_game_cgroup(&lower_cgroup_path);

    if is_game {
        if is_game_render_comm(&lower_comm) {
            return TaskClass::GameRenderThread;
        }
        if lower_comm.contains("worker")
            || lower_comm.contains("task")
            || lower_comm.contains("job")
        {
            return TaskClass::GameWorkerThread;
        }
    }

    if is_browser_process(&lower_comm, &lower_process_comm, &combined) {
        if contains_any(&combined, &["gpu process", "--type=gpu-process"])
            || lower_comm.contains("gpu process")
        {
            return TaskClass::BrowserGpu;
        }
        if contains_any(
            &combined,
            &[
                "utility process",
                "--type=utility",
                "network service",
                "socket process",
            ],
        ) {
            return TaskClass::BrowserNetwork;
        }
        if contains_any(
            &combined,
            &[
                "web content",
                "isolated web co",
                "rdd process",
                "--type=renderer",
                "renderer",
            ],
        ) {
            return TaskClass::BrowserRenderer;
        }
        if contains_any(
            &combined,
            &[
                "--background",
                "background",
                "crashpad",
                "updater",
                "extension process",
            ],
        ) {
            return TaskClass::BrowserBackground;
        }
        return TaskClass::BrowserForeground;
    }

    // 5. Development & System Work
    if is_indexer_comm(&lower_comm) {
        return TaskClass::Indexer;
    }
    if is_compiler_comm(&lower_comm) {
        return TaskClass::Compiler;
    }
    if is_linker_comm(&lower_comm) {
        return TaskClass::Linker;
    }
    if is_package_manager_comm(&lower_comm) || lower_cmdline.contains(" emerge ") {
        return TaskClass::PackageManager;
    }
    if is_build_job_comm(&lower_comm) {
        return TaskClass::BuildJob;
    }

    // 6. Daemons & Services
    if is_storage_daemon_comm(&lower_comm) {
        return TaskClass::StorageDaemon;
    }
    if is_network_daemon_comm(&lower_comm) {
        return TaskClass::NetworkDaemon;
    }

    // 7. Community app-name hints, then generic process fallbacks.
    #[cfg(test)]
    if !is_service_looking_process(&lower_process_comm, &lower_cgroup_path)
        && let Some(hit) = crate::community_rules::classify_process_identity(
            &crate::community_rules::CommunityProcessIdentity {
                thread_comm: comm,
                process_comm,
                cmdline,
                exe_path,
                cgroup_path,
            },
        )
    {
        return hit.class;
    }

    if is_game {
        return TaskClass::Game;
    }
    if is_steam_runtime_comm(&lower_process_comm) || lower_exe_path.contains("pressure-vessel") {
        return TaskClass::SteamRuntime;
    }
    if is_launcher_comm(&lower_comm) || is_launcher_comm(&lower_process_comm) {
        return TaskClass::Launcher;
    }
    if is_service_looking_process(&lower_process_comm, &lower_cgroup_path)
        || lower_process_comm.contains("helper")
    {
        return TaskClass::Service;
    }

    // 8. Other App Categories
    if is_editor_comm(&lower_comm) {
        return TaskClass::Editor;
    }
    if is_terminal_comm(&lower_comm) {
        return TaskClass::Terminal;
    }
    if is_shell_comm(&lower_comm) {
        return TaskClass::Shell;
    }
    if is_media_comm(&lower_comm) {
        return TaskClass::Media;
    }
    if is_recorder_comm(&lower_comm) {
        return TaskClass::Recorder;
    }
    if is_vm_comm(&lower_comm) {
        return TaskClass::VirtualMachine;
    }

    if lower_exe_path.ends_with(".exe") {
        return TaskClass::GameHelper;
    }

    TaskClass::Unknown
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn is_bracketed_kernel_comm(comm: &str) -> bool {
    comm.starts_with('[') && comm.ends_with(']')
}

fn is_irq_thread(comm: &str, combined: &str) -> bool {
    comm.starts_with("irq/")
        || comm.starts_with("irq-")
        || combined.contains(" irq/")
        || combined.contains(" irq-")
}

fn is_input_thread(comm: &str, combined: &str) -> bool {
    comm.contains("input") || combined.contains("libinput")
}

fn is_audio_realtime_comm(comm: &str) -> bool {
    matches!(
        comm,
        "pipewire" | "wireplumber" | "pulseaudio" | "jackd" | "easyeffects"
    )
}

fn is_audio_looking_process(combined: &str) -> bool {
    contains_any(
        combined,
        &[
            "audio",
            "alsa",
            "jack",
            "pipewire",
            "pulseaudio",
            "pulse",
            "easyeffects",
        ],
    )
}

fn is_game_render_comm(comm: &str) -> bool {
    contains_any(comm, &["render", "rhi", "dxvk", "vulkan", "gpu"])
}

fn is_game_exe(exe_path: &str) -> bool {
    exe_path.contains("steamapps/common")
        || exe_path.contains("/games/")
        || exe_path.contains("pressure-vessel")
}

fn is_game_comm(comm: &str) -> bool {
    contains_any(comm, &["steam", "proton", "wine"])
}

fn is_game_cgroup(cgroup: &str) -> bool {
    cgroup.contains("steam") || cgroup.contains("games")
}

fn is_browser_process(comm: &str, process_comm: &str, combined: &str) -> bool {
    let names = ["firefox", "chrome", "chromium", "brave", "browser"];
    contains_any(comm, &names)
        || contains_any(process_comm, &names)
        || contains_any(combined, &["--type=renderer", "--type=gpu-process"])
}

fn is_compiler_comm(comm: &str) -> bool {
    contains_any(comm, &["rustc", "gcc", "g++", "clang", "cc1", "cc1plus"])
}

fn is_linker_comm(comm: &str) -> bool {
    matches!(comm, "ld" | "ld.lld" | "ld.gold" | "mold" | "gold" | "lld")
}

fn is_indexer_comm(comm: &str) -> bool {
    contains_any(comm, &["clangd", "rust-analyzer", "ccls", "indexer"])
}

fn is_package_manager_comm(comm: &str) -> bool {
    contains_any(comm, &["emerge", "portage", "pacman", "apt", "dnf"])
}

fn is_build_job_comm(comm: &str) -> bool {
    contains_any(comm, &["cargo", "make", "ninja", "cmake", "meson"])
}

fn is_storage_daemon_comm(comm: &str) -> bool {
    contains_any(comm, &["udisks", "jbd2", "btrfs", "zfs", "io_uring"])
}

fn is_network_daemon_comm(comm: &str) -> bool {
    contains_any(
        comm,
        &[
            "networkmanager",
            "systemd-network",
            "dhcpcd",
            "wpa_supplicant",
        ],
    )
}

fn is_steam_runtime_comm(comm: &str) -> bool {
    contains_any(comm, &["pressure-vessel", "bwrap"])
}

fn is_launcher_comm(comm: &str) -> bool {
    contains_any(
        comm,
        &["epicgames", "origin", "uplay", "battle.net", "lutris"],
    )
}

fn is_service_looking_process(comm: &str, cgroup: &str) -> bool {
    comm == "systemd"
        || cgroup.contains(".service")
        || cgroup.contains("/system.slice/")
        || comm.ends_with("d")
}

fn is_editor_comm(comm: &str) -> bool {
    contains_any(comm, &["code", "vscodium", "kate", "nvim", "vim", "emacs"])
}

fn is_terminal_comm(comm: &str) -> bool {
    contains_any(
        comm,
        &[
            "alacritty",
            "kitty",
            "wezterm",
            "foot",
            "gnome-terminal",
            "konsole",
        ],
    )
}

fn is_shell_comm(comm: &str) -> bool {
    matches!(comm, "bash" | "zsh" | "fish" | "sh")
}

fn is_media_comm(comm: &str) -> bool {
    contains_any(comm, &["vlc", "mpv", "spotify"])
}

fn is_recorder_comm(comm: &str) -> bool {
    contains_any(comm, &["obs", "gpu-screen-recorder", "recorder"])
}

fn is_vm_comm(comm: &str) -> bool {
    contains_any(comm, &["qemu", "virt", "virtualbox", "vmware"])
}

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
    let budget = ScanBudget::default_proc_scan();
    let mut budget_report = ScanBudgetReport::default();
    let processes = scan_processes_at(
        proc_root,
        &mut ProcessCache::default(),
        &budget,
        &mut budget_report,
    );
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

    #[test]
    fn process_cache_ttl_refresh() {
        let dir = std::env::temp_dir().join(format!("stutter-ttl-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let pid_dir = dir.join("100");
        fs::create_dir_all(&pid_dir).unwrap();
        fs::write(pid_dir.join("status"), "Name:\ttest\nPPid:\t1\n").unwrap();
        fs::write(pid_dir.join("cmdline"), "test\0").unwrap();
        fs::write(
            pid_dir.join("stat"),
            "100 (test) S 1 100 0 0 -1 0 0 0 0 0 0 0 0 20 0 1 0 1000 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
        )
        .unwrap();

        let mut cache = ProcessCache {
            max_cached_generations: 1,
            ..Default::default()
        };

        // scan 1: inserts at generation 1
        let budget = ScanBudget::default_proc_scan();
        let mut budget_report1 = ScanBudgetReport::default();
        let p1 = scan_processes_at(&dir, &mut cache, &budget, &mut budget_report1);
        assert_eq!(p1.len(), 1);
        assert_eq!(cache.generation, 1);
        assert_eq!(cache.entries.get(&100).unwrap().scan_generation, 1);

        // scan 2: delta is 1, cache is still reused (generation 2, cached 1, 2-1=1 <= 1)
        let mut budget_report2 = ScanBudgetReport::default();
        let p2 = scan_processes_at(&dir, &mut cache, &budget, &mut budget_report2);
        assert_eq!(p2.len(), 1);
        assert_eq!(cache.generation, 2);
        assert_eq!(cache.entries.get(&100).unwrap().scan_generation, 1);

        // scan 3: delta is 2, cache is refreshed (generation 3, cached 1, 3-1=2 > 1)
        let mut budget_report3 = ScanBudgetReport::default();
        let p3 = scan_processes_at(&dir, &mut cache, &budget, &mut budget_report3);
        assert_eq!(p3.len(), 1);
        assert_eq!(cache.generation, 3);
        assert_eq!(cache.entries.get(&100).unwrap().scan_generation, 3);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn test_thread_ids_of_at_limited() {
        let dir =
            std::env::temp_dir().join(format!("stutter-test-thread-limit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let pid_dir = dir.join("100");
        fs::create_dir_all(&pid_dir).unwrap();

        let task_dir = pid_dir.join("task");
        fs::create_dir_all(&task_dir).unwrap();

        for tid in 100..110 {
            fs::create_dir_all(task_dir.join(tid.to_string())).unwrap();
        }

        let mut report = ScanBudgetReport::default();
        let tids = thread_ids_of_at_limited(&dir, 100, 5, &mut report);

        assert_eq!(tids.len(), 5);
        assert_eq!(report.thread_entries_seen, 6);
        assert_eq!(report.thread_entries_skipped, 1);
        assert_eq!(report.processes_thread_limited, 1);

        let _ = fs::remove_dir_all(&dir);
    }

    fn write_fake_process(
        proc_root: &Path,
        pid: u32,
        comm: &str,
        cmdline: &str,
    ) -> std::io::Result<()> {
        let dir = proc_root.join(pid.to_string());
        fs::create_dir_all(&dir)?;

        fs::write(
            dir.join("status"),
            format!("Name:\t{comm}\nPid:\t{pid}\nPPid:\t1\nThreads:\t1\n"),
        )?;

        fs::write(dir.join("cmdline"), format!("{cmdline}\0"))?;

        fs::write(
            dir.join("stat"),
            format!(
                "{pid} ({comm}) S 1 {pid} {pid} 0 -1 4194304 0 0 0 0 0 0 0 0 20 0 1 0 12345 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n"
            ),
        )?;

        Ok(())
    }

    #[test]
    fn community_rule_cmdline_basename_classifies_truncated_proton_game() {
        let class = classify_task_with_context(
            "KingdomCome",
            "KingdomCome",
            "/home/me/.steam/steamapps/compatdata/379430/pfx/drive_c/KingdomCome.exe --windowed",
            "/usr/bin/wine",
            "/user.slice/app-mystery-379430.scope",
            None,
        );

        assert_eq!(class, TaskClass::Game);
    }

    #[test]
    fn ambiguous_community_rule_outside_steam_context_is_not_game() {
        let class = classify_task_with_context(
            "build.exe",
            "build.exe",
            "/tmp/build.exe --compile",
            "/tmp/build.exe",
            "/user.slice/app-builder.scope",
            None,
        );

        assert_ne!(class, TaskClass::Game);
    }

    #[test]
    fn ambiguous_community_rule_inside_compatdata_context_can_be_game() {
        let class = classify_task_with_context(
            "build.exe",
            "build.exe",
            "/home/me/.steam/steamapps/compatdata/123/pfx/drive_c/build.exe",
            "/usr/bin/wine",
            "/user.slice/app-mystery-123.scope",
            None,
        );

        assert_eq!(class, TaskClass::Game);
    }

    #[test]
    fn hardcoded_audio_classification_wins_over_game_like_context() {
        let class = classify_task_with_context(
            "pipewire",
            "pipewire",
            "/home/me/.steam/steamapps/compatdata/379430/pfx/drive_c/KingdomCome.exe",
            "/home/me/.steam/steamapps/common/KingdomCome/KingdomCome.exe",
            "/user.slice/app-mystery-379430.scope",
            Some(2),
        );

        assert_eq!(class, TaskClass::AudioRealtime);
    }

    #[test]
    fn process_cache_evicts_pid_missing_from_next_scan() {
        let temp = tempfile::tempdir().unwrap();
        let proc_root = temp.path();

        write_fake_process(proc_root, 100, "old", "old-cmd").unwrap();

        let mut cache = ProcessCache::default();
        let budget = ScanBudget::default_proc_scan();
        let mut budget_report = ScanBudgetReport::default();

        let first = scan_processes_at(proc_root, &mut cache, &budget, &mut budget_report);
        assert!(first.contains_key(&100));
        assert!(cache.entries.contains_key(&100));

        fs::remove_dir_all(proc_root.join("100")).unwrap();

        let mut budget_report2 = ScanBudgetReport::default();
        let second = scan_processes_at(proc_root, &mut cache, &budget, &mut budget_report2);
        assert!(!second.contains_key(&100));
        assert!(!cache.entries.contains_key(&100));
    }

    #[test]
    fn process_cache_replaced_when_pid_recreated_with_new_comm() {
        let temp = tempfile::tempdir().unwrap();
        let proc_root = temp.path();

        write_fake_process(proc_root, 100, "old", "old-cmd").unwrap();

        let mut cache = ProcessCache::default();
        let budget = ScanBudget::default_proc_scan();

        let mut budget_report1 = ScanBudgetReport::default();
        let first = scan_processes_at(proc_root, &mut cache, &budget, &mut budget_report1);
        assert_eq!(first.get(&100).unwrap().comm, "old");
        assert_eq!(cache.entries.get(&100).unwrap().info.comm, "old");

        fs::remove_dir_all(proc_root.join("100")).unwrap();

        let mut budget_report2 = ScanBudgetReport::default();
        let _ = scan_processes_at(proc_root, &mut cache, &budget, &mut budget_report2);
        assert!(!cache.entries.contains_key(&100));

        write_fake_process(proc_root, 100, "new", "new-cmd").unwrap();

        let mut budget_report3 = ScanBudgetReport::default();
        let third = scan_processes_at(proc_root, &mut cache, &budget, &mut budget_report3);
        assert_eq!(third.get(&100).unwrap().comm, "new");
        assert_eq!(cache.entries.get(&100).unwrap().info.comm, "new");
    }
}
