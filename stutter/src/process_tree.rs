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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum TaskClass {
    Game,
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
    pub class: TaskClass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetSnapshot {
    pub process_roots: BTreeSet<u32>,
    pub tasks: BTreeMap<u32, TaskInfo>,
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

    Some(ProcInfo {
        pid,
        ppid: ppid?,
        comm: comm?,
        cmdline,
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

pub fn target_snapshot_at(
    proc_root: &Path,
    manual_pids: &[u32],
    tree_pids: &[u32],
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

    for pid in &process_roots {
        if !tasks
            .values()
            .any(|task_info| task_info.process_pid == *pid)
        {
            tasks.insert(*pid, fallback_task_info(*pid, processes.get(pid)));
        }
    }

    for task in tasks.values_mut() {
        if task.class == TaskClass::Unknown && !requested_roots.contains(&task.process_pid) {
            task.class = TaskClass::Helper;
        }
    }

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
                task_info_from_proc(*pid, *pid, &proc_info.comm, proc_info),
            );
            continue;
        }

        for tid in tids {
            let comm = task_comm_at(proc_root, *pid, tid).unwrap_or_else(|| proc_info.comm.clone());

            tasks.insert(tid, task_info_from_proc(tid, *pid, &comm, proc_info));
        }
    }

    tasks
}

fn fallback_task_info(pid: u32, proc_info: Option<&ProcInfo>) -> TaskInfo {
    match proc_info {
        Some(proc_info) => task_info_from_proc(pid, pid, &proc_info.comm, proc_info),
        None => TaskInfo {
            tid: pid,
            process_pid: pid,
            process_ppid: 0,
            comm: "?".to_owned(),
            process_comm: "?".to_owned(),
            class: TaskClass::Unknown,
        },
    }
}

fn task_info_from_proc(tid: u32, process_pid: u32, comm: &str, proc_info: &ProcInfo) -> TaskInfo {
    TaskInfo {
        tid,
        process_pid,
        process_ppid: proc_info.ppid,
        comm: comm.to_owned(),
        process_comm: proc_info.comm.clone(),
        class: classify_task(comm, &proc_info.comm, &proc_info.cmdline),
    }
}

pub fn classify_task(comm: &str, process_comm: &str, cmdline: &str) -> TaskClass {
    let comm = comm.to_ascii_lowercase();
    let process_comm = process_comm.to_ascii_lowercase();
    let cmdline = cmdline.to_ascii_lowercase();
    let haystack = format!("{} {} {}", comm, process_comm, cmdline);

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
            "reaper",
            "launcher",
            "launch",
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
        ],
    ) {
        return TaskClass::Helper;
    }

    if contains_game_exe_candidate(&comm)
        || contains_game_exe_candidate(&process_comm)
        || contains_game_exe_candidate(&cmdline)
    {
        return TaskClass::Game;
    }

    if contains_token(
        &haystack,
        &[
            "steam",
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

    TaskClass::Unknown
}

fn contains_token(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn contains_game_exe_candidate(text: &str) -> bool {
    text.split(is_exe_separator)
        .filter_map(exe_candidate_name)
        .any(|name| !is_non_game_exe(name))
}

fn is_exe_separator(ch: char) -> bool {
    ch.is_ascii_whitespace()
        || matches!(
            ch,
            '\0' | '"'
                | '\''
                | '\\'
                | '/'
                | ':'
                | ';'
                | ','
                | '='
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
        )
}

fn exe_candidate_name(token: &str) -> Option<&str> {
    let exe_end = token.find(".exe")? + ".exe".len();
    Some(&token[..exe_end])
}

fn is_non_game_exe(name: &str) -> bool {
    matches!(
        name,
        "steam.exe"
            | "steamwebhelper.exe"
            | "steamerrorreporter.exe"
            | "xalia.exe"
            | "explorer.exe"
            | "services.exe"
            | "winedevice.exe"
            | "svchost.exe"
            | "plugplay.exe"
            | "rpcss.exe"
            | "tabtip.exe"
            | "rundll32.exe"
            | "wineboot.exe"
            | "winemenubuilder.exe"
            | "conhost.exe"
            | "regsvr32.exe"
            | "msiexec.exe"
    )
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
                if class == TaskClass::Unknown {
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
        let branch = if is_last {
            "\u{2514}\u{2500} "
        } else {
            "\u{251c}\u{2500} "
        };
        let child_prefix = if is_last {
            format!("{prefix}   ")
        } else {
            format!("{prefix}\u{2502}  ")
        };

        match entry {
            TreeEntry::Process {
                pid: child,
                comm,
                class,
            } => {
                output.push_str(&format!(
                    "{prefix}{branch}{child} {comm}{}\n",
                    class_suffix(*class)
                ));
                render_children(
                    proc_root,
                    *child,
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

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    fn finds_descendants_from_snapshot() {
        let processes = BTreeMap::from([
            (
                10,
                ProcInfo {
                    pid: 10,
                    ppid: 1,
                    comm: "root".to_owned(),
                    cmdline: String::new(),
                },
            ),
            (
                20,
                ProcInfo {
                    pid: 20,
                    ppid: 10,
                    comm: "child".to_owned(),
                    cmdline: String::new(),
                },
            ),
            (
                30,
                ProcInfo {
                    pid: 30,
                    ppid: 20,
                    comm: "grandchild".to_owned(),
                    cmdline: String::new(),
                },
            ),
            (
                40,
                ProcInfo {
                    pid: 40,
                    ppid: 1,
                    comm: "other".to_owned(),
                    cmdline: String::new(),
                },
            ),
        ]);

        assert_eq!(descendants_of(10, &processes), BTreeSet::from([10, 20, 30]));
    }

    #[test]
    fn expands_live_tasks_from_fake_proc() {
        let proc_root = fake_proc_root();
        write_process(&proc_root, 10, 1, "root", &[10, 11]);
        write_process(&proc_root, 20, 10, "child", &[20, 21]);

        let processes = scan_processes_at(&proc_root);
        let process_pids = descendants_of(10, &processes);
        let tasks = expand_tasks_at(&proc_root, &process_pids, &processes);

        assert_eq!(
            tasks.keys().copied().collect::<Vec<_>>(),
            vec![10, 11, 20, 21]
        );
        assert_eq!(tasks.get(&21).unwrap().process_pid, 20);

        fs::remove_dir_all(proc_root).unwrap();
    }

    #[test]
    fn renders_processes_and_threads() {
        let proc_root = fake_proc_root();
        write_process(&proc_root, 10, 1, "gamescope", &[10]);
        write_process(&proc_root, 20, 10, "wine", &[20, 22]);
        write_process(&proc_root, 30, 20, "game", &[30, 31]);

        let rendered = render_tree_at(&proc_root, 10).unwrap();

        assert!(rendered.contains("root 10 gamescope [GameScope]"));
        assert!(rendered.contains("20 wine [Helper]"));
        assert!(rendered.contains("22 wine-thread [Helper]"));
        assert!(rendered.contains("30 game [Helper]"));
        assert!(rendered.contains("31 game-thread [Helper]"));

        fs::remove_dir_all(proc_root).unwrap();
    }

    #[test]
    fn classifies_kcd_like_tree() {
        let proc_root = fake_proc_root();
        write_process(&proc_root, 100, 1, "gamescope", &[100]);
        write_process(&proc_root, 110, 100, "reaper", &[110]);
        write_process(&proc_root, 120, 100, "pv-adverb", &[120]);
        write_process(&proc_root, 130, 120, "wineserver", &[130]);
        write_process(&proc_root, 140, 120, "KingdomCome.exe", &[140, 141, 142]);
        write_process_with_cmdline_and_threads(
            &proc_root,
            145,
            120,
            "Main",
            "pressure-vessel steam-runtime Z:\\games\\KingdomCome.exe",
            &[(145, "Main"), (146, "RenderThread"), (147, "dxvk-cs")],
        );
        write_process(&proc_root, 150, 120, "services.exe", &[150]);
        write_process(&proc_root, 160, 120, "xalia.exe", &[160]);

        let snapshot = target_snapshot_at(&proc_root, &[], &[100]);

        assert_eq!(
            snapshot.tasks.get(&100).unwrap().class,
            TaskClass::GameScope
        );
        assert_eq!(snapshot.tasks.get(&110).unwrap().class, TaskClass::Helper);
        assert_eq!(
            snapshot.tasks.get(&120).unwrap().class,
            TaskClass::SteamRuntime
        );
        assert_eq!(
            snapshot.tasks.get(&130).unwrap().class,
            TaskClass::WineServer
        );
        assert_eq!(snapshot.tasks.get(&140).unwrap().class, TaskClass::Game);
        assert_eq!(snapshot.tasks.get(&141).unwrap().class, TaskClass::Game);
        assert_eq!(snapshot.tasks.get(&142).unwrap().class, TaskClass::Game);
        assert_eq!(snapshot.tasks.get(&145).unwrap().class, TaskClass::Game);
        assert_eq!(snapshot.tasks.get(&146).unwrap().class, TaskClass::Game);
        assert_eq!(snapshot.tasks.get(&147).unwrap().class, TaskClass::Game);
        assert_eq!(snapshot.tasks.get(&150).unwrap().class, TaskClass::Helper);
        assert_eq!(
            snapshot.tasks.get(&160).unwrap().class,
            TaskClass::SteamRuntime
        );

        let rendered = render_tree_at(&proc_root, 100).unwrap();
        assert!(rendered.contains("root 100 gamescope [GameScope]"));
        assert!(rendered.contains("110 reaper [Helper]"));
        assert!(rendered.contains("120 pv-adverb [SteamRuntime]"));
        assert!(rendered.contains("130 wineserver [WineServer]"));
        assert!(rendered.contains("140 KingdomCome.exe [Game]"));
        assert!(rendered.contains("141 KingdomCome.exe-thread [Game]"));
        assert!(rendered.contains("145 Main [Game]"));
        assert!(rendered.contains("146 RenderThread [Game]"));
        assert!(rendered.contains("150 services.exe [Helper]"));
        assert!(rendered.contains("160 xalia.exe [SteamRuntime]"));

        fs::remove_dir_all(proc_root).unwrap();
    }

    #[test]
    fn diffs_added_and_removed_tasks() {
        let old = BTreeMap::from([(
            10,
            TaskInfo {
                tid: 10,
                process_pid: 10,
                process_ppid: 1,
                comm: "old".to_owned(),
                process_comm: "old".to_owned(),
                class: TaskClass::Unknown,
            },
        )]);
        let new = BTreeMap::from([(
            20,
            TaskInfo {
                tid: 20,
                process_pid: 20,
                process_ppid: 1,
                comm: "new".to_owned(),
                process_comm: "new".to_owned(),
                class: TaskClass::Helper,
            },
        )]);

        let diffs = diff_tasks(&old, &new);

        assert_eq!(diffs.len(), 2);
        assert_eq!(diffs[0].action, TargetDiffAction::Added);
        assert_eq!(diffs[0].task.tid, 20);
        assert_eq!(diffs[1].action, TargetDiffAction::Removed);
        assert_eq!(diffs[1].task.tid, 10);
    }

    fn fake_proc_root() -> PathBuf {
        let mut path = std::env::temp_dir();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("stutter-fake-proc-{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_process(proc_root: &Path, pid: u32, ppid: u32, comm: &str, tids: &[u32]) {
        let thread_names = tids
            .iter()
            .map(|tid| {
                if *tid == pid {
                    (*tid, comm.to_owned())
                } else {
                    (*tid, format!("{comm}-thread"))
                }
            })
            .collect::<Vec<_>>();
        let thread_names = thread_names
            .iter()
            .map(|(tid, comm)| (*tid, comm.as_str()))
            .collect::<Vec<_>>();
        write_process_with_cmdline_and_threads(proc_root, pid, ppid, comm, comm, &thread_names);
    }

    fn write_process_with_cmdline_and_threads(
        proc_root: &Path,
        pid: u32,
        ppid: u32,
        comm: &str,
        cmdline: &str,
        threads: &[(u32, &str)],
    ) {
        let proc_dir = proc_root.join(pid.to_string());
        let task_dir = proc_dir.join("task");
        fs::create_dir_all(&task_dir).unwrap();
        fs::write(
            proc_dir.join("status"),
            format!("Name:\t{comm}\nPPid:\t{ppid}\n"),
        )
        .unwrap();
        fs::write(proc_dir.join("cmdline"), format!("{cmdline}\0")).unwrap();

        for (tid, thread_comm) in threads {
            let task = task_dir.join(tid.to_string());
            fs::create_dir_all(&task).unwrap();
            fs::write(task.join("comm"), format!("{thread_comm}\n")).unwrap();
        }
    }
}
