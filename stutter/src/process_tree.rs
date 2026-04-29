use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fmt, fs, io,
    path::Path,
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    pub comm: String,
    pub cmdline: String,
    pub starttime_ticks: Option<u64>,
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
    pub class: TaskClass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetSnapshot {
    pub process_roots: BTreeSet<u32>,
    pub tasks: BTreeMap<u32, TaskInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskFilters {
    pub include_comm: Vec<String>,
    pub exclude_comm: Vec<String>,
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetDiffAction {
    Added,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetDiff {
    pub action: TargetDiffAction,
    pub task: TaskInfo,
}

pub fn scan_processes_at(proc_root: &Path) -> BTreeMap<u32, ProcInfo> {
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

        if let Some(info) = read_proc_info_at(proc_root, pid) {
            processes.insert(pid, info);
        }
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

    let starttime_ticks = stat_starttime_at(&proc_root.join(pid.to_string()).join("stat"));

    Some(ProcInfo {
        pid,
        ppid: ppid?,
        comm: comm?,
        cmdline,
        starttime_ticks,
    })
}

pub fn descendants_of(root_pid: u32, processes: &BTreeMap<u32, ProcInfo>) -> BTreeSet<u32> {
    let mut children_by_parent: BTreeMap<u32, Vec<u32>> = BTreeMap::new();

    for proc_info in processes.values() {
        children_by_parent
            .entry(proc_info.ppid)
            .or_default()
            .push(proc_info.pid);
    }

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

pub fn target_snapshot(manual_pids: &[u32], tree_pids: &[u32]) -> TargetSnapshot {
    target_snapshot_at(Path::new("/proc"), manual_pids, tree_pids)
}

pub fn target_snapshot_with_filters(
    manual_pids: &[u32],
    tree_pids: &[u32],
    filters: &TaskFilters,
) -> TargetSnapshot {
    target_snapshot_filtered_at(Path::new("/proc"), manual_pids, tree_pids, filters)
}

pub fn target_snapshot_at(
    proc_root: &Path,
    manual_pids: &[u32],
    tree_pids: &[u32],
) -> TargetSnapshot {
    target_snapshot_filtered_at(proc_root, manual_pids, tree_pids, &TaskFilters::default())
}

pub fn target_snapshot_filtered_at(
    proc_root: &Path,
    manual_pids: &[u32],
    tree_pids: &[u32],
    filters: &TaskFilters,
) -> TargetSnapshot {
    let processes = scan_processes_at(proc_root);
    let mut requested_roots = BTreeSet::new();
    let mut process_roots = BTreeSet::new();

    for pid in manual_pids {
        requested_roots.insert(*pid);
        process_roots.insert(*pid);
    }

    for root_pid in tree_pids {
        requested_roots.insert(*root_pid);
        process_roots.insert(*root_pid);
        process_roots.extend(descendants_of(*root_pid, &processes));
    }

    let mut tasks = expand_tasks_at(proc_root, &process_roots, &processes);
    let mut seen_process_pids = tasks
        .values()
        .map(|task| task.process_pid)
        .collect::<BTreeSet<_>>();

    for pid in &process_roots {
        if seen_process_pids.insert(*pid) {
            tasks.insert(*pid, fallback_task_info(*pid, processes.get(pid)));
        }
    }

    for task in tasks.values_mut() {
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

pub fn diff_tasks(
    old_tasks: &BTreeMap<u32, TaskInfo>,
    new_tasks: &BTreeMap<u32, TaskInfo>,
) -> Vec<TargetDiff> {
    let mut diffs = Vec::new();

    for tid in new_tasks.keys() {
        if !old_tasks.contains_key(tid) {
            diffs.push(TargetDiff {
                action: TargetDiffAction::Added,
                task: new_tasks[tid].clone(),
            });
        }
    }

    for tid in old_tasks.keys() {
        if !new_tasks.contains_key(tid) {
            diffs.push(TargetDiff {
                action: TargetDiffAction::Removed,
                task: old_tasks[tid].clone(),
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

pub fn expand_tasks_at(
    proc_root: &Path,
    process_pids: &BTreeSet<u32>,
    processes: &BTreeMap<u32, ProcInfo>,
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
            let comm = task_comm_at(proc_root, *pid, tid).unwrap_or_else(|| proc_info.comm.clone());
            tasks.insert(
                tid,
                task_info_from_proc(proc_root, tid, *pid, &comm, proc_info),
            );
        }
    }

    tasks
}

fn fallback_task_info(pid: u32, proc_info: Option<&ProcInfo>) -> TaskInfo {
    match proc_info {
        Some(proc_info) => {
            task_info_from_proc(Path::new("/proc"), pid, pid, &proc_info.comm, proc_info)
        }
        None => TaskInfo {
            tid: pid,
            process_pid: pid,
            process_ppid: 0,
            comm: "?".to_owned(),
            process_comm: "?".to_owned(),
            process_starttime_ticks: None,
            task_starttime_ticks: None,
            class: TaskClass::Unknown,
        },
    }
}

fn task_info_from_proc(
    proc_root: &Path,
    tid: u32,
    process_pid: u32,
    comm: &str,
    proc_info: &ProcInfo,
) -> TaskInfo {
    let task_starttime_ticks = if tid == process_pid {
        proc_info.starttime_ticks
    } else {
        stat_starttime_at(
            &proc_root
                .join(process_pid.to_string())
                .join("task")
                .join(tid.to_string())
                .join("stat"),
        )
    };

    TaskInfo {
        tid,
        process_pid,
        process_ppid: proc_info.ppid,
        comm: comm.to_owned(),
        process_comm: proc_info.comm.clone(),
        process_starttime_ticks: proc_info.starttime_ticks,
        task_starttime_ticks,
        class: classify_task(comm, &proc_info.comm, &proc_info.cmdline),
    }
}

fn stat_starttime_at(path: &Path) -> Option<u64> {
    let stat = fs::read_to_string(path).ok()?;
    parse_proc_stat_starttime(&stat)
}

pub fn parse_proc_stat_starttime(stat: &str) -> Option<u64> {
    let (_, after_comm) = stat.rsplit_once(") ")?;
    after_comm.split_whitespace().nth(19)?.parse().ok()
}

fn comm_pattern_matches(pattern: &str, task: &TaskInfo) -> bool {
    !pattern.is_empty() && (task.comm.contains(pattern) || task.process_comm.contains(pattern))
}

pub fn classify_task(comm: &str, process_comm: &str, cmdline: &str) -> TaskClass {
    let comm = comm.to_ascii_lowercase();
    let process_comm = process_comm.to_ascii_lowercase();
    let cmdline = cmdline.to_ascii_lowercase();
    let haystack = format!("{comm} {process_comm} {cmdline}");

    if contains_token(&haystack, &["gamescope"]) {
        return TaskClass::GameScope;
    }

    if contains_token(
        &haystack,
        &[
            "sway",
            "kwin",
            "kwin_wayland",
            "mutter",
            "gnome-shell",
            "steamcompmgr",
        ],
    ) {
        return TaskClass::Compositor;
    }

    if contains_token(&haystack, &["wineserver"]) {
        return TaskClass::WineServer;
    }

    if contains_token(
        &haystack,
        &[
            "pressure-vessel",
            "pv-",
            "steam-runtime",
            "soldier",
            "sniper",
            "srt-bwrap",
            "xalia",
        ],
    ) {
        return TaskClass::SteamRuntime;
    }

    if contains_token(
        &haystack,
        &[
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
        ],
    ) {
        return TaskClass::Launcher;
    }

    if contains_token(
        &haystack,
        &[
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
        ],
    ) {
        return TaskClass::Helper;
    }

    if contains_likely_game_cmdline(&cmdline) {
        return TaskClass::Game;
    }

    if contains_any_exe(&comm) || contains_any_exe(&process_comm) || contains_any_exe(&cmdline) {
        return TaskClass::GameHelper;
    }

    TaskClass::Unknown
}

fn contains_token(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
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

fn contains_any_exe(text: &str) -> bool {
    text.contains(".exe")
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

pub fn render_tree(root_pid: u32) -> io::Result<String> {
    render_tree_at(Path::new("/proc"), root_pid)
}

pub fn render_tree_at(proc_root: &Path, root_pid: u32) -> io::Result<String> {
    let processes = scan_processes_at(proc_root);
    let Some(root) = processes.get(&root_pid) else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("process {root_pid} not found under {}", proc_root.display()),
        ));
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
