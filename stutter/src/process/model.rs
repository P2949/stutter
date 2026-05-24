//! Process-tree data model.
//!
//! Owns scan budgets, process/task snapshots, task classes, target snapshot inputs,
//! compiled filters, and target diff records. Does not own procfs reads, classification,
//! target expansion, tree rendering, or cgroup traversal.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    path::Path,
    time::{Duration, Instant},
};

use regex::Regex;
use serde::{Deserialize, Serialize};
use stutter_core::ids::{Pid, Tid};

use crate::community_rules::CommunityRulesDb;

pub const DEFAULT_MAX_PROC_SCAN_MS: u64 = 50;
pub const DEFAULT_MAX_THREADS_PER_PROCESS: usize = 4096;

#[derive(Debug, Clone)]
pub struct ScanBudget {
    pub(crate) started_at: Instant,
    pub(crate) max_duration: Duration,
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
pub fn priority_band_for_class(class: TaskClass, sched_policy: Option<u32>) -> PriorityBand {
    if matches!(sched_policy, Some(1 | 2 | 6)) {
        match class {
            TaskClass::AudioRealtime | TaskClass::Input => {
                return PriorityBand::CriticalRealtime;
            }
            _ => {}
        }
    }

    match class {
        TaskClass::AudioRealtime | TaskClass::Input => PriorityBand::CriticalRealtime,

        TaskClass::Game
        | TaskClass::GameRenderThread
        | TaskClass::GameWorkerThread
        | TaskClass::GameScope
        | TaskClass::WineServer
        | TaskClass::Compositor
        | TaskClass::BrowserForeground => PriorityBand::ForegroundLatency,

        TaskClass::Editor
        | TaskClass::Terminal
        | TaskClass::Shell
        | TaskClass::BrowserRenderer
        | TaskClass::BrowserGpu
        | TaskClass::BrowserNetwork
        | TaskClass::Media => PriorityBand::Interactive,

        TaskClass::BuildJob
        | TaskClass::Compiler
        | TaskClass::Linker
        | TaskClass::Recorder
        | TaskClass::VirtualMachine => PriorityBand::Throughput,

        TaskClass::Indexer
        | TaskClass::PackageManager
        | TaskClass::StorageDaemon
        | TaskClass::NetworkDaemon
        | TaskClass::Service => PriorityBand::Background,

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
    pub process_comm: String,
    pub process_starttime_ticks: Option<u64>,
    pub task_starttime_ticks: Option<u64>,
    pub exe_dev: Option<u64>,
    pub exe_ino: Option<u64>,
    pub class: TaskClass,
    pub sched_policy: Option<u32>,
    pub from_cgroup: bool,
}

impl TaskInfo {
    pub fn task_id(&self) -> Tid {
        Tid::new(self.tid)
    }

    pub fn process_id(&self) -> Pid {
        Pid::new(self.process_pid)
    }

    pub fn parent_process_id(&self) -> Pid {
        Pid::new(self.process_ppid)
    }
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
    pub(crate) fn allows(&self, task: &TaskInfo) -> bool {
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
