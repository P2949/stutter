#![allow(dead_code)]
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

const SCHED_FIFO: u32 = 1;
const SCHED_RR: u32 = 2;
const SCHED_DEADLINE: u32 = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub enum SystemTaskClass {
    Game,
    GameRenderThread,
    GameWorkerThread,
    WineServer,
    GameScope,
    Compositor,

    AudioRealtime,
    Input,

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

    StorageDaemon,
    NetworkDaemon,
    KernelThread,
    IrqThread,

    Editor,
    Terminal,
    Shell,
    Media,
    Recorder,
    VirtualMachine,

    Service,

    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PriorityBand {
    CriticalRealtime,
    ForegroundLatency,
    Interactive,
    Throughput,
    Background,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Classification {
    pub class: SystemTaskClass,
    pub priority_band: PriorityBand,
    pub confidence: f32,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ProcessIdentity<'a> {
    pub pid: u32,
    pub ppid: u32,
    pub comm: &'a str,
    pub cmdline: &'a str,
    pub exe_path: Option<&'a str>,
    pub cgroup_path: Option<&'a str>,
    pub sched_policy: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct ThreadIdentity<'a> {
    pub tid: u32,
    pub process_pid: u32,
    pub process_class: SystemTaskClass,
    pub thread_comm: &'a str,
    pub process_comm: &'a str,
    pub sched_policy: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct FocusSnapshot {
    pub elapsed_ms: u64,
    pub processes: BTreeMap<u32, FocusProcess>,
    pub children_by_parent: BTreeMap<u32, Vec<u32>>,
    pub groups: Vec<FocusGroup>,
}

#[derive(Debug, Clone)]
pub struct FocusProcess {
    pub pid: u32,
    pub ppid: u32,
    pub comm: String,
    pub cmdline: String,
    pub cgroup_path: Option<PathBuf>,
    pub starttime_ticks: Option<u64>,
    pub sched_policy: Option<u32>,

    pub classification: Classification,

    pub cpu_time_ticks_delta: u64,
    pub read_bytes_delta: u64,
    pub write_bytes_delta: u64,
    pub voluntary_ctxt_switches_delta: u64,
    pub nonvoluntary_ctxt_switches_delta: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FocusGroupKind {
    Game,
    Browser,
    Compile,
    Media,
    Recording,
    VirtualMachine,
    Desktop,
    Idle,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FocusGroup {
    pub kind: FocusGroupKind,
    pub root_pids: Vec<u32>,
    pub member_pids: Vec<u32>,
    pub primary_pid: Option<u32>,
    pub display_name: String,
    pub score: f32,
    pub confidence: f32,
    pub priority_band: PriorityBand,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct FocusCache {
    previous: BTreeMap<u32, FocusCounters>,
    proc_cache: crate::process_tree::ProcessCache,
}

#[derive(Debug, Clone, Default)]
pub struct FocusCounters {
    pub starttime_ticks: Option<u64>,
    pub cpu_time_ticks: u64,
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub voluntary_ctxt_switches: u64,
    pub nonvoluntary_ctxt_switches: u64,
}

impl FocusCache {
    pub fn clear(&mut self) {
        self.previous.clear();
        self.proc_cache = crate::process_tree::ProcessCache::default();
    }

    pub fn previous_len(&self) -> usize {
        self.previous.len()
    }
}

pub fn focus_snapshot_at(
    proc_root: &Path,
    cache: &mut FocusCache,
    elapsed_ms: u64,
) -> FocusSnapshot {
    let budget = crate::process_tree::ScanBudget::default_proc_scan();
    let mut budget_report = crate::process_tree::ScanBudgetReport::default();
    let processes = crate::process_tree::scan_processes_at(
        proc_root,
        &mut cache.proc_cache,
        &budget,
        &mut budget_report,
    );

    build_focus_snapshot_from_processes(proc_root, cache, elapsed_ms, processes)
}

fn build_focus_snapshot_from_processes(
    proc_root: &Path,
    cache: &mut FocusCache,
    elapsed_ms: u64,
    processes: BTreeMap<u32, crate::process_tree::ProcInfo>,
) -> FocusSnapshot {
    let mut children_by_parent: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for proc_info in processes.values() {
        children_by_parent
            .entry(proc_info.ppid)
            .or_default()
            .push(proc_info.pid);
    }
    for children in children_by_parent.values_mut() {
        children.sort_unstable();
    }

    let mut focus_processes = BTreeMap::new();
    let mut next_counters = BTreeMap::new();

    for proc_info in processes.values() {
        let mut counters = read_focus_counters_at(proc_root, proc_info.pid);
        counters.starttime_ticks = proc_info.starttime_ticks.or(counters.starttime_ticks);

        let deltas = counter_deltas(cache.previous.get(&proc_info.pid), &counters);

        let exe_path = non_empty_str(&proc_info.exe_path);
        let cgroup_path = non_empty_str(&proc_info.cgroup_path);

        let classification = classify_process(&ProcessIdentity {
            pid: proc_info.pid,
            ppid: proc_info.ppid,
            comm: &proc_info.comm,
            cmdline: &proc_info.cmdline,
            exe_path,
            cgroup_path,
            sched_policy: proc_info.sched_policy,
        });

        let focus_process = FocusProcess {
            pid: proc_info.pid,
            ppid: proc_info.ppid,
            comm: proc_info.comm.clone(),
            cmdline: proc_info.cmdline.clone(),
            cgroup_path: cgroup_path.map(PathBuf::from),
            starttime_ticks: proc_info.starttime_ticks.or(counters.starttime_ticks),
            sched_policy: proc_info.sched_policy,
            classification,
            cpu_time_ticks_delta: deltas.cpu_time_ticks,
            read_bytes_delta: deltas.read_bytes,
            write_bytes_delta: deltas.write_bytes,
            voluntary_ctxt_switches_delta: deltas.voluntary_ctxt_switches,
            nonvoluntary_ctxt_switches_delta: deltas.nonvoluntary_ctxt_switches,
        };

        next_counters.insert(proc_info.pid, counters);
        focus_processes.insert(proc_info.pid, focus_process);
    }

    cache.previous = next_counters;

    let mut snapshot = FocusSnapshot {
        elapsed_ms,
        processes: focus_processes,
        children_by_parent,
        groups: Vec::new(),
    };
    snapshot.groups = build_focus_groups(&snapshot);

    snapshot
}

fn non_empty_str(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}

fn read_focus_counters_at(proc_root: &Path, pid: u32) -> FocusCounters {
    let mut counters = FocusCounters::default();
    let process_root = proc_root.join(pid.to_string());

    if let Some((cpu_time_ticks, starttime_ticks)) =
        read_focus_stat_counters_at(&process_root.join("stat"))
    {
        counters.cpu_time_ticks = cpu_time_ticks;
        counters.starttime_ticks = starttime_ticks;
    }

    if let Some((read_bytes, write_bytes)) = read_focus_io_counters_at(&process_root.join("io")) {
        counters.read_bytes = read_bytes;
        counters.write_bytes = write_bytes;
    }

    if let Some((voluntary, nonvoluntary)) =
        read_focus_status_context_switches_at(&process_root.join("status"))
    {
        counters.voluntary_ctxt_switches = voluntary;
        counters.nonvoluntary_ctxt_switches = nonvoluntary;
    }

    counters
}

fn read_focus_stat_counters_at(path: &Path) -> Option<(u64, Option<u64>)> {
    let stat = fs::read_to_string(path).ok()?;
    let (_, after_comm) = stat.rsplit_once(") ")?;
    let fields = after_comm.split_whitespace().collect::<Vec<_>>();

    let utime = fields.get(11)?.parse::<u64>().ok()?;
    let stime = fields.get(12)?.parse::<u64>().ok()?;
    let starttime_ticks = fields.get(19).and_then(|value| value.parse::<u64>().ok());

    Some((utime.saturating_add(stime), starttime_ticks))
}

fn read_focus_io_counters_at(path: &Path) -> Option<(u64, u64)> {
    let contents = fs::read_to_string(path).ok()?;
    let mut read_bytes = 0;
    let mut write_bytes = 0;

    for line in contents.lines() {
        if let Some(value) = line.strip_prefix("read_bytes:") {
            read_bytes = value.trim().parse::<u64>().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("write_bytes:") {
            write_bytes = value.trim().parse::<u64>().unwrap_or(0);
        }
    }

    Some((read_bytes, write_bytes))
}

fn read_focus_status_context_switches_at(path: &Path) -> Option<(u64, u64)> {
    let contents = fs::read_to_string(path).ok()?;
    let mut voluntary = 0;
    let mut nonvoluntary = 0;

    for line in contents.lines() {
        if let Some(value) = line.strip_prefix("voluntary_ctxt_switches:") {
            voluntary = value.trim().parse::<u64>().unwrap_or(0);
        } else if let Some(value) = line.strip_prefix("nonvoluntary_ctxt_switches:") {
            nonvoluntary = value.trim().parse::<u64>().unwrap_or(0);
        }
    }

    Some((voluntary, nonvoluntary))
}

fn counter_deltas(previous: Option<&FocusCounters>, current: &FocusCounters) -> FocusCounters {
    let Some(previous) = previous else {
        return FocusCounters {
            starttime_ticks: current.starttime_ticks,
            ..FocusCounters::default()
        };
    };

    if previous.starttime_ticks != current.starttime_ticks {
        return FocusCounters {
            starttime_ticks: current.starttime_ticks,
            ..FocusCounters::default()
        };
    }

    FocusCounters {
        starttime_ticks: current.starttime_ticks,
        cpu_time_ticks: current
            .cpu_time_ticks
            .saturating_sub(previous.cpu_time_ticks),
        read_bytes: current.read_bytes.saturating_sub(previous.read_bytes),
        write_bytes: current.write_bytes.saturating_sub(previous.write_bytes),
        voluntary_ctxt_switches: current
            .voluntary_ctxt_switches
            .saturating_sub(previous.voluntary_ctxt_switches),
        nonvoluntary_ctxt_switches: current
            .nonvoluntary_ctxt_switches
            .saturating_sub(previous.nonvoluntary_ctxt_switches),
    }
}

fn build_focus_groups(snapshot: &FocusSnapshot) -> Vec<FocusGroup> {
    let mut claimed_pids = BTreeSet::new();
    let mut groups = Vec::new();

    if let Some(group) = build_game_group(snapshot, &claimed_pids) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    for group in build_browser_groups(snapshot, &claimed_pids) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    for group in build_compile_groups(snapshot, &claimed_pids) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    for group in
        build_tree_groups_for_kind(snapshot, &claimed_pids, FocusGroupKind::Media, |process| {
            process.classification.class == SystemTaskClass::Media
        })
    {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    for group in build_tree_groups_for_kind(
        snapshot,
        &claimed_pids,
        FocusGroupKind::Recording,
        |process| process.classification.class == SystemTaskClass::Recorder,
    ) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    for group in build_tree_groups_for_kind(
        snapshot,
        &claimed_pids,
        FocusGroupKind::VirtualMachine,
        |process| process.classification.class == SystemTaskClass::VirtualMachine,
    ) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    if let Some(group) = build_desktop_group(snapshot, &claimed_pids) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    if let Some(group) = build_idle_group(snapshot, &claimed_pids) {
        claimed_pids.extend(group.member_pids.iter().copied());
        groups.push(group);
    }

    if let Some(group) = build_fallback_group(snapshot, &claimed_pids) {
        groups.push(group);
    }

    groups.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                priority_band_rank(right.priority_band).cmp(&priority_band_rank(left.priority_band))
            })
            .then_with(|| left.display_name.cmp(&right.display_name))
    });

    groups
}

fn build_game_group(snapshot: &FocusSnapshot, claimed_pids: &BTreeSet<u32>) -> Option<FocusGroup> {
    let game_like_pids = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| {
            is_game_class(process.classification.class) || is_game_runtime_process(process)
        })
        .map(|process| process.pid)
        .collect::<BTreeSet<_>>();

    if game_like_pids.is_empty() {
        return None;
    }

    let root_pid = game_like_pids
        .iter()
        .copied()
        .filter(|pid| {
            snapshot
                .processes
                .get(pid)
                .map(|process| process.classification.class == SystemTaskClass::GameScope)
                .unwrap_or(false)
        })
        .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        .or_else(|| {
            game_like_pids
                .iter()
                .copied()
                .filter(|pid| {
                    snapshot
                        .processes
                        .get(pid)
                        .map(|process| process.classification.class == SystemTaskClass::Game)
                        .unwrap_or(false)
                })
                .filter(|pid| !has_ancestor_in_set(snapshot, *pid, &game_like_pids))
                .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        })
        .or_else(|| {
            game_like_pids
                .iter()
                .copied()
                .filter(|pid| {
                    snapshot
                        .processes
                        .get(pid)
                        .map(|process| {
                            is_game_runtime_process(process)
                                && descendants_of_pid(snapshot, process.pid).iter().any(
                                    |child_pid| {
                                        snapshot
                                            .processes
                                            .get(child_pid)
                                            .map(|child| {
                                                child.classification.class == SystemTaskClass::Game
                                                    || child.classification.class
                                                        == SystemTaskClass::GameRenderThread
                                                    || child.classification.class
                                                        == SystemTaskClass::GameWorkerThread
                                            })
                                            .unwrap_or(false)
                                    },
                                )
                        })
                        .unwrap_or(false)
                })
                .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        })
        .or_else(|| {
            game_like_pids
                .iter()
                .copied()
                .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        })?;

    let mut member_pids = descendants_of_pid(snapshot, root_pid)
        .into_iter()
        .filter(|pid| !claimed_pids.contains(pid))
        .collect::<BTreeSet<_>>();

    for process in snapshot.processes.values() {
        if claimed_pids.contains(&process.pid) {
            continue;
        }

        if process.classification.class == SystemTaskClass::WineServer
            && process_appears_tied_to_root(snapshot, process.pid, root_pid)
        {
            member_pids.insert(process.pid);
        }
    }

    for pid in &game_like_pids {
        if !claimed_pids.contains(pid) && process_appears_tied_to_root(snapshot, *pid, root_pid) {
            member_pids.insert(*pid);
        }
    }

    make_focus_group(
        snapshot,
        FocusGroupKind::Game,
        vec![root_pid],
        member_pids.into_iter().collect(),
        Some(root_pid),
        vec![
            "game group selected from gamescope/game/runtime roots".to_owned(),
            "wineserver is included only when tied to the same parent/session/cgroup/runtime"
                .to_owned(),
        ],
    )
}

fn build_browser_groups(snapshot: &FocusSnapshot, claimed_pids: &BTreeSet<u32>) -> Vec<FocusGroup> {
    let browser_pids = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| is_browser_class(process.classification.class))
        .map(|process| process.pid)
        .collect::<BTreeSet<_>>();

    let mut roots = browser_pids
        .iter()
        .copied()
        .filter(|pid| {
            snapshot
                .processes
                .get(pid)
                .map(|process| {
                    process.classification.class == SystemTaskClass::BrowserForeground
                        || !has_ancestor_in_set(snapshot, *pid, &browser_pids)
                })
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    roots.sort_by(|left, right| compare_process_preference(snapshot, *right, *left));

    let mut used = BTreeSet::new();
    let mut groups = Vec::new();

    for root_pid in roots {
        if used.contains(&root_pid) {
            continue;
        }

        let mut member_pids = descendants_of_pid(snapshot, root_pid)
            .into_iter()
            .filter(|pid| !claimed_pids.contains(pid))
            .filter(|pid| {
                snapshot
                    .processes
                    .get(pid)
                    .map(|process| is_browser_class(process.classification.class))
                    .unwrap_or(false)
            })
            .collect::<BTreeSet<_>>();

        member_pids.insert(root_pid);

        if member_pids.is_empty() {
            continue;
        }

        let primary_pid = member_pids
            .iter()
            .copied()
            .filter(|pid| {
                snapshot
                    .processes
                    .get(pid)
                    .map(|process| {
                        process.classification.class == SystemTaskClass::BrowserForeground
                    })
                    .unwrap_or(false)
            })
            .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
            .or(Some(root_pid));

        if let Some(group) = make_focus_group(
            snapshot,
            FocusGroupKind::Browser,
            vec![root_pid],
            member_pids.iter().copied().collect(),
            primary_pid,
            vec![
                "browser group rooted at browser parent process".to_owned(),
                "renderer/GPU/network descendants are included under the browser parent".to_owned(),
            ],
        ) {
            used.extend(member_pids);
            groups.push(group);
        }
    }

    groups
}

fn build_compile_groups(snapshot: &FocusSnapshot, claimed_pids: &BTreeSet<u32>) -> Vec<FocusGroup> {
    let compile_pids = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| is_compile_class(process.classification.class))
        .map(|process| process.pid)
        .collect::<BTreeSet<_>>();

    if compile_pids.is_empty() {
        return Vec::new();
    }

    let mut roots = compile_pids
        .iter()
        .copied()
        .filter(|pid| {
            snapshot
                .processes
                .get(pid)
                .map(is_stable_build_root)
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();

    if roots.is_empty() {
        roots = compile_pids
            .iter()
            .copied()
            .filter_map(|pid| nearest_compile_session_root(snapshot, pid))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
    }

    if roots.is_empty() {
        roots = compile_pids
            .iter()
            .copied()
            .filter(|pid| !has_ancestor_in_set(snapshot, *pid, &compile_pids))
            .collect::<Vec<_>>();
    }

    if roots.is_empty() {
        roots = compile_pids.iter().copied().collect::<Vec<_>>();
    }

    roots.sort_by(|left, right| {
        stable_build_root_rank(snapshot, *right)
            .cmp(&stable_build_root_rank(snapshot, *left))
            .then_with(|| compare_process_preference(snapshot, *right, *left))
    });

    let mut used = BTreeSet::new();
    let mut groups = Vec::new();

    for root_pid in roots {
        if used.contains(&root_pid) || claimed_pids.contains(&root_pid) {
            continue;
        }

        let mut member_pids = descendants_of_pid(snapshot, root_pid)
            .into_iter()
            .filter(|pid| !claimed_pids.contains(pid))
            .filter(|pid| {
                snapshot
                    .processes
                    .get(pid)
                    .map(|process| {
                        is_compile_class(process.classification.class)
                            || process.pid == root_pid
                            || process.classification.class == SystemTaskClass::Terminal
                            || process.classification.class == SystemTaskClass::Shell
                    })
                    .unwrap_or(false)
            })
            .collect::<BTreeSet<_>>();

        if compile_pids.contains(&root_pid) {
            member_pids.insert(root_pid);
        }

        if !member_pids.iter().any(|pid| compile_pids.contains(pid)) {
            continue;
        }

        let primary_pid = member_pids
            .iter()
            .copied()
            .filter(|pid| {
                snapshot
                    .processes
                    .get(pid)
                    .map(is_stable_build_root)
                    .unwrap_or(false)
            })
            .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
            .or_else(|| {
                member_pids
                    .iter()
                    .copied()
                    .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
            });

        if let Some(group) = make_focus_group(
            snapshot,
            FocusGroupKind::Compile,
            vec![root_pid],
            member_pids.iter().copied().collect(),
            primary_pid,
            vec![
                "compile group prefers stable build roots such as cargo/ninja/make/cmake/meson"
                    .to_owned(),
                "compiler/linker descendants are grouped under the stable build/session root"
                    .to_owned(),
            ],
        ) {
            used.extend(member_pids);
            groups.push(group);
        }
    }

    groups
}

fn build_tree_groups_for_kind<F>(
    snapshot: &FocusSnapshot,
    claimed_pids: &BTreeSet<u32>,
    kind: FocusGroupKind,
    predicate: F,
) -> Vec<FocusGroup>
where
    F: Fn(&FocusProcess) -> bool,
{
    let matching_pids = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| predicate(process))
        .map(|process| process.pid)
        .collect::<BTreeSet<_>>();

    let mut roots = matching_pids
        .iter()
        .copied()
        .filter(|pid| !has_ancestor_in_set(snapshot, *pid, &matching_pids))
        .collect::<Vec<_>>();

    if roots.is_empty() {
        roots = matching_pids.iter().copied().collect::<Vec<_>>();
    }

    roots.sort_by(|left, right| compare_process_preference(snapshot, *right, *left));

    let mut used = BTreeSet::new();
    let mut groups = Vec::new();

    for root_pid in roots {
        if used.contains(&root_pid) {
            continue;
        }

        let member_pids = descendants_of_pid(snapshot, root_pid)
            .into_iter()
            .filter(|pid| !claimed_pids.contains(pid))
            .filter(|pid| matching_pids.contains(pid) || *pid == root_pid)
            .collect::<Vec<_>>();

        if member_pids.is_empty() {
            continue;
        }

        let primary_pid = member_pids
            .iter()
            .copied()
            .max_by(|left, right| compare_process_preference(snapshot, *left, *right));

        if let Some(group) = make_focus_group(
            snapshot,
            kind,
            vec![root_pid],
            member_pids.clone(),
            primary_pid,
            vec![format!("{kind:?} group rooted at stable process tree")],
        ) {
            used.extend(member_pids);
            groups.push(group);
        }
    }

    groups
}

fn build_desktop_group(
    snapshot: &FocusSnapshot,
    claimed_pids: &BTreeSet<u32>,
) -> Option<FocusGroup> {
    let desktop_pids = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| {
            focus_group_kind_for_class(process.classification.class) == FocusGroupKind::Desktop
        })
        .map(|process| process.pid)
        .collect::<BTreeSet<_>>();

    if desktop_pids.is_empty() {
        return None;
    }

    let primary_pid = desktop_pids
        .iter()
        .copied()
        .filter(|pid| {
            snapshot
                .processes
                .get(pid)
                .map(|process| process.classification.class == SystemTaskClass::Compositor)
                .unwrap_or(false)
        })
        .filter(|pid| {
            snapshot
                .processes
                .get(pid)
                .map(is_active_foreground_candidate)
                .unwrap_or(false)
        })
        .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        .or_else(|| {
            desktop_pids
                .iter()
                .copied()
                .filter(|pid| {
                    snapshot
                        .processes
                        .get(pid)
                        .map(is_active_foreground_candidate)
                        .unwrap_or(false)
                })
                .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        })
        .or_else(|| {
            desktop_pids
                .iter()
                .copied()
                .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
        });

    make_focus_group(
        snapshot,
        FocusGroupKind::Desktop,
        root_pids_from_members(snapshot, &desktop_pids),
        desktop_pids.into_iter().collect(),
        primary_pid,
        vec![
            "desktop group is supporting context".to_owned(),
            "compositor becomes primary only when it is an active foreground-latency candidate"
                .to_owned(),
        ],
    )
}

fn build_idle_group(snapshot: &FocusSnapshot, claimed_pids: &BTreeSet<u32>) -> Option<FocusGroup> {
    let idle_pids = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| {
            focus_group_kind_for_class(process.classification.class) == FocusGroupKind::Idle
        })
        .map(|process| process.pid)
        .collect::<BTreeSet<_>>();

    if idle_pids.is_empty() {
        return None;
    }

    let primary_pid = idle_pids
        .iter()
        .copied()
        .max_by(|left, right| compare_process_preference(snapshot, *left, *right));

    make_focus_group(
        snapshot,
        FocusGroupKind::Idle,
        root_pids_from_members(snapshot, &idle_pids),
        idle_pids.into_iter().collect(),
        primary_pid,
        vec!["idle/background service group is never allowed to outrank active foreground work by base score alone".to_owned()],
    )
}

fn build_fallback_group(
    snapshot: &FocusSnapshot,
    claimed_pids: &BTreeSet<u32>,
) -> Option<FocusGroup> {
    let root_pid = snapshot
        .processes
        .values()
        .filter(|process| !claimed_pids.contains(&process.pid))
        .filter(|process| is_non_service_interactive_class(process.classification.class))
        .max_by(|left, right| {
            left.cpu_time_ticks_delta
                .cmp(&right.cpu_time_ticks_delta)
                .then_with(|| {
                    process_focus_score(left)
                        .partial_cmp(&process_focus_score(right))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        })
        .map(|process| process.pid)?;

    let member_pids = descendants_of_pid(snapshot, root_pid)
        .into_iter()
        .filter(|pid| !claimed_pids.contains(pid))
        .collect::<Vec<_>>();

    if member_pids.is_empty() {
        return None;
    }

    let primary_pid = member_pids
        .iter()
        .copied()
        .max_by(|left, right| compare_process_preference(snapshot, *left, *right));

    make_focus_group(
        snapshot,
        FocusGroupKind::Unknown,
        vec![root_pid],
        member_pids,
        primary_pid,
        vec![
            "fallback selected highest non-service interactive process tree by recent CPU delta"
                .to_owned(),
        ],
    )
}

fn make_focus_group(
    snapshot: &FocusSnapshot,
    kind: FocusGroupKind,
    mut root_pids: Vec<u32>,
    mut member_pids: Vec<u32>,
    primary_pid: Option<u32>,
    mut reasons: Vec<String>,
) -> Option<FocusGroup> {
    root_pids.sort_unstable();
    root_pids.dedup();
    member_pids.sort_unstable();
    member_pids.dedup();

    if member_pids.is_empty() {
        return None;
    }

    let primary_pid = primary_pid.or_else(|| {
        member_pids
            .iter()
            .copied()
            .max_by(|left, right| compare_process_preference(snapshot, *left, *right))
    });

    let score = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .map(process_focus_score)
        .sum::<f32>();

    let confidence = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .map(|process| process.classification.confidence)
        .fold(0.0_f32, f32::max);

    let priority_band = member_pids
        .iter()
        .filter_map(|pid| snapshot.processes.get(pid))
        .map(|process| process.classification.priority_band)
        .max_by_key(|band| priority_band_rank(*band))
        .unwrap_or(PriorityBand::Unknown);

    let display_name = display_name_for_group(
        kind,
        primary_pid.and_then(|pid| snapshot.processes.get(&pid)),
    );

    if let Some(primary_pid) = primary_pid
        && let Some(primary) = snapshot.processes.get(&primary_pid)
    {
        reasons.push(format!(
            "primary pid={} comm='{}' class={:?}",
            primary.pid, primary.comm, primary.classification.class
        ));
    }

    Some(FocusGroup {
        kind,
        root_pids,
        member_pids,
        primary_pid,
        display_name,
        score,
        confidence,
        priority_band,
        reasons,
    })
}

fn root_pids_from_members(snapshot: &FocusSnapshot, member_pids: &BTreeSet<u32>) -> Vec<u32> {
    member_pids
        .iter()
        .copied()
        .filter(|pid| {
            snapshot
                .processes
                .get(pid)
                .map(|process| !member_pids.contains(&process.ppid))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>()
}

fn descendants_of_pid(snapshot: &FocusSnapshot, root_pid: u32) -> BTreeSet<u32> {
    let mut result = BTreeSet::new();
    let mut stack = vec![root_pid];

    while let Some(pid) = stack.pop() {
        if !result.insert(pid) {
            continue;
        }

        if let Some(children) = snapshot.children_by_parent.get(&pid) {
            for child in children.iter().rev() {
                stack.push(*child);
            }
        }
    }

    result
}

fn has_ancestor_in_set(snapshot: &FocusSnapshot, pid: u32, pids: &BTreeSet<u32>) -> bool {
    let mut current = pid;
    let mut seen = BTreeSet::new();

    while let Some(process) = snapshot.processes.get(&current) {
        if !seen.insert(current) {
            return false;
        }

        let parent = process.ppid;
        if parent == current || parent == 0 {
            return false;
        }

        if pids.contains(&parent) {
            return true;
        }

        current = parent;
    }

    false
}

fn compare_process_preference(
    snapshot: &FocusSnapshot,
    left_pid: u32,
    right_pid: u32,
) -> std::cmp::Ordering {
    let left_score = snapshot
        .processes
        .get(&left_pid)
        .map(process_focus_score)
        .unwrap_or_default();
    let right_score = snapshot
        .processes
        .get(&right_pid)
        .map(process_focus_score)
        .unwrap_or_default();

    left_score
        .partial_cmp(&right_score)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| left_pid.cmp(&right_pid))
}

fn process_appears_tied_to_root(snapshot: &FocusSnapshot, pid: u32, root_pid: u32) -> bool {
    if pid == root_pid {
        return true;
    }

    let Some(process) = snapshot.processes.get(&pid) else {
        return false;
    };
    let Some(root) = snapshot.processes.get(&root_pid) else {
        return false;
    };

    descendants_of_pid(snapshot, root_pid).contains(&pid)
        || process.ppid == root.ppid
        || same_non_empty_cgroup(process, root)
        || (contains_game_runtime_text(process) && contains_game_runtime_text(root))
}

fn same_non_empty_cgroup(left: &FocusProcess, right: &FocusProcess) -> bool {
    match (&left.cgroup_path, &right.cgroup_path) {
        (Some(left), Some(right)) => {
            !left.as_os_str().is_empty() && !right.as_os_str().is_empty() && left == right
        }
        _ => false,
    }
}

fn contains_game_runtime_text(process: &FocusProcess) -> bool {
    let text = process_identity_text(process);
    text.contains("steamapps")
        || text.contains("pressure-vessel")
        || text.contains("proton")
        || text.contains("wineserver")
}

fn is_game_runtime_process(process: &FocusProcess) -> bool {
    let text = process_identity_text(process);
    text.contains("pressure-vessel")
        || text.contains("steam-runtime")
        || text.contains("proton")
        || text.contains("steamapps")
}

fn is_game_class(class: SystemTaskClass) -> bool {
    matches!(
        class,
        SystemTaskClass::Game
            | SystemTaskClass::GameRenderThread
            | SystemTaskClass::GameWorkerThread
            | SystemTaskClass::WineServer
            | SystemTaskClass::GameScope
    )
}

fn is_browser_class(class: SystemTaskClass) -> bool {
    matches!(
        class,
        SystemTaskClass::BrowserForeground
            | SystemTaskClass::BrowserBackground
            | SystemTaskClass::BrowserRenderer
            | SystemTaskClass::BrowserGpu
            | SystemTaskClass::BrowserNetwork
    )
}

fn is_compile_class(class: SystemTaskClass) -> bool {
    matches!(
        class,
        SystemTaskClass::BuildJob
            | SystemTaskClass::Compiler
            | SystemTaskClass::Linker
            | SystemTaskClass::Indexer
            | SystemTaskClass::PackageManager
    )
}

fn is_non_service_interactive_class(class: SystemTaskClass) -> bool {
    matches!(
        class,
        SystemTaskClass::AudioRealtime
            | SystemTaskClass::Input
            | SystemTaskClass::Game
            | SystemTaskClass::GameRenderThread
            | SystemTaskClass::GameWorkerThread
            | SystemTaskClass::WineServer
            | SystemTaskClass::GameScope
            | SystemTaskClass::Compositor
            | SystemTaskClass::BrowserForeground
            | SystemTaskClass::BrowserRenderer
            | SystemTaskClass::BrowserGpu
            | SystemTaskClass::BrowserNetwork
            | SystemTaskClass::Editor
            | SystemTaskClass::Terminal
            | SystemTaskClass::Shell
            | SystemTaskClass::Media
            | SystemTaskClass::Recorder
            | SystemTaskClass::VirtualMachine
    )
}

fn is_active_foreground_candidate(process: &FocusProcess) -> bool {
    process.cpu_time_ticks_delta > 0
        || process.read_bytes_delta > 0
        || process.write_bytes_delta > 0
        || process.voluntary_ctxt_switches_delta > 0
        || process.nonvoluntary_ctxt_switches_delta > 0
}

fn is_stable_build_root(process: &FocusProcess) -> bool {
    let comm = process.comm.to_ascii_lowercase();
    matches!(
        comm.as_str(),
        "cargo" | "ninja" | "make" | "cmake" | "meson" | "scons"
    ) || process.classification.class == SystemTaskClass::BuildJob
}

fn stable_build_root_rank(snapshot: &FocusSnapshot, pid: u32) -> u8 {
    snapshot
        .processes
        .get(&pid)
        .map(|process| {
            if is_stable_build_root(process) {
                3
            } else if process.classification.class == SystemTaskClass::Terminal {
                2
            } else if process.classification.class == SystemTaskClass::Shell {
                1
            } else {
                0
            }
        })
        .unwrap_or(0)
}

fn nearest_compile_session_root(snapshot: &FocusSnapshot, pid: u32) -> Option<u32> {
    let mut current = pid;
    let mut nearest_shell_or_terminal = None;
    let process_cgroup = snapshot
        .processes
        .get(&pid)
        .and_then(|process| process.cgroup_path.clone());

    while let Some(process) = snapshot.processes.get(&current) {
        if is_stable_build_root(process) {
            return Some(process.pid);
        }

        if matches!(
            process.classification.class,
            SystemTaskClass::Terminal | SystemTaskClass::Shell
        ) {
            nearest_shell_or_terminal = Some(process.pid);
            break; // Stop at the NEAREST shell/terminal
        }

        let parent = process.ppid;
        if parent == current || parent == 0 {
            break;
        }

        current = parent;
    }

    nearest_shell_or_terminal.or_else(|| {
        process_cgroup.and_then(|cgroup| {
            snapshot
                .processes
                .values()
                .filter(|process| process.cgroup_path.as_ref() == Some(&cgroup))
                .filter(|process| {
                    matches!(
                        process.classification.class,
                        SystemTaskClass::Terminal | SystemTaskClass::Shell
                    )
                })
                .max_by(|left, right| {
                    process_focus_score(left)
                        .partial_cmp(&process_focus_score(right))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|process| process.pid)
        })
    })
}

fn process_identity_text(process: &FocusProcess) -> String {
    let cgroup_path = process
        .cgroup_path
        .as_ref()
        .map(|path| path.to_string_lossy())
        .unwrap_or_default();

    format!(
        "{} {} {}",
        process.comm.to_ascii_lowercase(),
        process.cmdline.to_ascii_lowercase(),
        cgroup_path.to_ascii_lowercase()
    )
}

fn focus_group_kind_for_class(class: SystemTaskClass) -> FocusGroupKind {
    match class {
        SystemTaskClass::Game
        | SystemTaskClass::GameRenderThread
        | SystemTaskClass::GameWorkerThread
        | SystemTaskClass::WineServer
        | SystemTaskClass::GameScope => FocusGroupKind::Game,

        SystemTaskClass::BrowserForeground
        | SystemTaskClass::BrowserBackground
        | SystemTaskClass::BrowserRenderer
        | SystemTaskClass::BrowserGpu
        | SystemTaskClass::BrowserNetwork => FocusGroupKind::Browser,

        SystemTaskClass::BuildJob
        | SystemTaskClass::Compiler
        | SystemTaskClass::Linker
        | SystemTaskClass::Indexer
        | SystemTaskClass::PackageManager => FocusGroupKind::Compile,

        SystemTaskClass::Media => FocusGroupKind::Media,
        SystemTaskClass::Recorder => FocusGroupKind::Recording,
        SystemTaskClass::VirtualMachine => FocusGroupKind::VirtualMachine,

        SystemTaskClass::Compositor | SystemTaskClass::AudioRealtime | SystemTaskClass::Input => {
            FocusGroupKind::Desktop
        }

        SystemTaskClass::Editor
        | SystemTaskClass::Terminal
        | SystemTaskClass::Shell
        | SystemTaskClass::Unknown => FocusGroupKind::Unknown,

        SystemTaskClass::StorageDaemon
        | SystemTaskClass::NetworkDaemon
        | SystemTaskClass::KernelThread
        | SystemTaskClass::IrqThread
        | SystemTaskClass::Service => FocusGroupKind::Idle,
    }
}

fn process_focus_score(process: &FocusProcess) -> f32 {
    let class_base = match process.classification.class {
        SystemTaskClass::AudioRealtime | SystemTaskClass::Input => 90.0,
        SystemTaskClass::Game
        | SystemTaskClass::GameRenderThread
        | SystemTaskClass::GameWorkerThread
        | SystemTaskClass::WineServer
        | SystemTaskClass::GameScope => 80.0,
        SystemTaskClass::Compositor | SystemTaskClass::BrowserForeground => 70.0,
        SystemTaskClass::BrowserRenderer
        | SystemTaskClass::BrowserGpu
        | SystemTaskClass::BrowserNetwork
        | SystemTaskClass::Editor
        | SystemTaskClass::Terminal
        | SystemTaskClass::Shell
        | SystemTaskClass::Media
        | SystemTaskClass::Recorder
        | SystemTaskClass::VirtualMachine => 50.0,
        SystemTaskClass::BuildJob
        | SystemTaskClass::Compiler
        | SystemTaskClass::Linker
        | SystemTaskClass::Indexer
        | SystemTaskClass::PackageManager => 35.0,
        SystemTaskClass::StorageDaemon
        | SystemTaskClass::NetworkDaemon
        | SystemTaskClass::KernelThread
        | SystemTaskClass::IrqThread
        | SystemTaskClass::Service
        | SystemTaskClass::BrowserBackground => 15.0,
        SystemTaskClass::Unknown => 0.0,
    };

    let cpu_score = process.cpu_time_ticks_delta as f32;
    let io_score = (process
        .read_bytes_delta
        .saturating_add(process.write_bytes_delta) as f32)
        / 1_048_576.0;
    let ctxt_score = (process
        .voluntary_ctxt_switches_delta
        .saturating_add(process.nonvoluntary_ctxt_switches_delta) as f32)
        * 0.05;

    class_base + process.classification.confidence + cpu_score + io_score + ctxt_score
}

fn display_name_for_group(kind: FocusGroupKind, primary: Option<&FocusProcess>) -> String {
    if let Some(primary) = primary
        && !primary.comm.is_empty()
    {
        return primary.comm.clone();
    }

    match kind {
        FocusGroupKind::Game => "Game".to_owned(),
        FocusGroupKind::Browser => "Browser".to_owned(),
        FocusGroupKind::Compile => "Compile".to_owned(),
        FocusGroupKind::Media => "Media".to_owned(),
        FocusGroupKind::Recording => "Recording".to_owned(),
        FocusGroupKind::VirtualMachine => "VirtualMachine".to_owned(),
        FocusGroupKind::Desktop => "Desktop".to_owned(),
        FocusGroupKind::Idle => "Idle".to_owned(),
        FocusGroupKind::Unknown => "Unknown".to_owned(),
    }
}

fn priority_band_rank(priority_band: PriorityBand) -> u8 {
    match priority_band {
        PriorityBand::Unknown => 0,
        PriorityBand::Background => 1,
        PriorityBand::Throughput => 2,
        PriorityBand::Interactive => 3,
        PriorityBand::ForegroundLatency => 4,
        PriorityBand::CriticalRealtime => 5,
    }
}

pub fn classify_process(identity: &ProcessIdentity<'_>) -> Classification {
    let comm = identity.comm.to_ascii_lowercase();
    let cmdline = identity.cmdline.to_ascii_lowercase();
    let exe_path = identity.exe_path.unwrap_or_default().to_ascii_lowercase();
    let cgroup_path = identity
        .cgroup_path
        .unwrap_or_default()
        .to_ascii_lowercase();

    let mut reasons = Vec::new();

    let (class, confidence) = if comm.starts_with("irq/")
        || comm.starts_with("irq-")
        || comm.contains("irq")
    {
        reasons.push(format!("comm '{}' looks like an IRQ thread", identity.comm));
        (SystemTaskClass::IrqThread, 0.95)
    } else if identity.ppid == 2 || comm.starts_with("kworker") || comm.starts_with("ksoftirqd") {
        reasons.push(format!(
            "pid={} ppid={} comm='{}' looks like a kernel thread",
            identity.pid, identity.ppid, identity.comm
        ));
        (SystemTaskClass::KernelThread, 0.9)
    } else if comm == "wineserver" {
        reasons.push("comm is exactly 'wineserver'".to_owned());
        (SystemTaskClass::WineServer, 0.98)
    } else if comm == "gamescope"
        || exe_path.contains("/gamescope")
        || cgroup_path.contains("gamescope")
    {
        reasons.push("process identity matches gamescope".to_owned());
        (SystemTaskClass::GameScope, 0.95)
    } else if comm.contains("pipewire")
        || comm.contains("wireplumber")
        || comm.contains("pulseaudio")
        || comm.contains("jackd")
    {
        reasons.push(format!("comm '{}' matches an audio service", identity.comm));
        (SystemTaskClass::AudioRealtime, 0.9)
    } else if comm.contains("sway")
        || comm.contains("kwin")
        || comm.contains("mutter")
        || comm.contains("gnome-shell")
        || comm.contains("weston")
        || comm.contains("hyprland")
    {
        reasons.push(format!("comm '{}' matches a compositor", identity.comm));
        (SystemTaskClass::Compositor, 0.9)
    } else if comm.contains("firefox")
        || comm.contains("chrome")
        || comm.contains("chromium")
        || comm.contains("brave")
        || comm.contains("browser")
        || exe_path.contains("/firefox")
        || exe_path.contains("/chrome")
        || exe_path.contains("/chromium")
    {
        reasons.push("process identity matches a browser family".to_owned());
        if cmdline.contains("--type=gpu-process") {
            reasons.push("cmdline contains '--type=gpu-process'".to_owned());
            (SystemTaskClass::BrowserGpu, 0.9)
        } else if cmdline.contains("--type=renderer") || cmdline.contains("web content") {
            reasons.push("cmdline indicates a browser renderer child".to_owned());
            (SystemTaskClass::BrowserRenderer, 0.85)
        } else if cmdline.contains("--type=utility") && cmdline.contains("network") {
            reasons.push("cmdline indicates a browser network utility child".to_owned());
            (SystemTaskClass::BrowserNetwork, 0.85)
        } else if cgroup_path.contains("background") {
            reasons.push("cgroup path contains 'background'".to_owned());
            (SystemTaskClass::BrowserBackground, 0.75)
        } else {
            reasons.push("browser process role is foreground or parent by default".to_owned());
            (SystemTaskClass::BrowserForeground, 0.7)
        }
    } else if comm == "clang"
        || comm == "clang++"
        || comm == "gcc"
        || comm == "g++"
        || comm == "rustc"
        || comm == "cc1"
        || comm == "cc1plus"
    {
        reasons.push(format!("comm '{}' matches a compiler", identity.comm));
        (SystemTaskClass::Compiler, 0.95)
    } else if comm == "ld" || comm == "ld.lld" || comm == "mold" || comm == "gold" || comm == "lld"
    {
        reasons.push(format!("comm '{}' matches a linker", identity.comm));
        (SystemTaskClass::Linker, 0.95)
    } else if comm == "clangd"
        || comm.contains("rust-analyzer")
        || comm.contains("ccls")
        || comm.contains("indexer")
    {
        reasons.push(format!(
            "comm '{}' matches an indexer/language server",
            identity.comm
        ));
        (SystemTaskClass::Indexer, 0.9)
    } else if comm == "cargo"
        || comm == "make"
        || comm == "ninja"
        || comm == "cmake"
        || comm == "meson"
        || cmdline.contains(" emerge ")
        || comm == "emerge"
        || comm == "portage"
        || comm == "pacman"
        || comm == "apt"
        || comm == "dnf"
    {
        reasons.push(format!(
            "comm '{}' or cmdline matches build/package work",
            identity.comm
        ));
        if comm == "emerge"
            || comm == "portage"
            || comm == "pacman"
            || comm == "apt"
            || comm == "dnf"
        {
            (SystemTaskClass::PackageManager, 0.9)
        } else {
            (SystemTaskClass::BuildJob, 0.85)
        }
    } else if comm.contains("udisks")
        || comm.contains("jbd2")
        || comm.contains("btrfs")
        || comm.contains("zfs")
        || comm.contains("io_uring")
    {
        reasons.push(format!(
            "comm '{}' matches storage daemon/kernel storage work",
            identity.comm
        ));
        (SystemTaskClass::StorageDaemon, 0.8)
    } else if comm.contains("networkmanager")
        || comm.contains("systemd-network")
        || comm.contains("dhcpcd")
        || comm.contains("wpa_supplicant")
    {
        reasons.push(format!(
            "comm '{}' matches network daemon work",
            identity.comm
        ));
        (SystemTaskClass::NetworkDaemon, 0.85)
    } else if comm.contains("code")
        || comm.contains("vscodium")
        || comm.contains("kate")
        || comm.contains("nvim")
        || comm.contains("vim")
        || comm.contains("emacs")
    {
        reasons.push(format!("comm '{}' matches an editor", identity.comm));
        (SystemTaskClass::Editor, 0.8)
    } else if comm.contains("alacritty")
        || comm.contains("kitty")
        || comm.contains("wezterm")
        || comm.contains("foot")
        || comm.contains("gnome-terminal")
        || comm.contains("konsole")
    {
        reasons.push(format!("comm '{}' matches a terminal", identity.comm));
        (SystemTaskClass::Terminal, 0.85)
    } else if comm == "bash" || comm == "zsh" || comm == "fish" || comm == "sh" {
        reasons.push(format!("comm '{}' matches a shell", identity.comm));
        (SystemTaskClass::Shell, 0.9)
    } else if comm.contains("vlc")
        || comm.contains("mpv")
        || comm.contains("spotify")
        || comm.contains("pipewire-media-session")
    {
        reasons.push(format!("comm '{}' matches media playback", identity.comm));
        (SystemTaskClass::Media, 0.8)
    } else if comm.contains("obs")
        || comm.contains("gpu-screen-recorder")
        || comm.contains("recorder")
    {
        reasons.push(format!(
            "comm '{}' matches recording software",
            identity.comm
        ));
        (SystemTaskClass::Recorder, 0.85)
    } else if comm.contains("qemu")
        || comm.contains("virt")
        || comm.contains("virtualbox")
        || comm.contains("vmware")
    {
        reasons.push(format!(
            "comm '{}' matches virtual machine software",
            identity.comm
        ));
        (SystemTaskClass::VirtualMachine, 0.85)
    } else if cgroup_path.contains("steam")
        || cgroup_path.contains("games")
        || cmdline.contains("steamapps")
        || cmdline.contains(".exe")
        || exe_path.contains("steamapps")
    {
        reasons.push("cgroup, cmdline, or exe path suggests a game process".to_owned());
        (SystemTaskClass::Game, 0.75)
    } else if comm == "steam" || comm.contains("electron") || comm == "python" {
        reasons.push(format!(
            "comm '{}' is ambiguous and needs more evidence before assigning a specific class",
            identity.comm
        ));
        (SystemTaskClass::Unknown, 0.35)
    } else if identity.pid == 1
        || cgroup_path.contains(".service")
        || cgroup_path.contains("/system.slice/")
    {
        reasons.push("pid/cgroup suggests a generic service".to_owned());
        (SystemTaskClass::Service, 0.6)
    } else {
        reasons.push("no strong process classification rule matched".to_owned());
        (SystemTaskClass::Unknown, 0.0)
    };

    Classification {
        class,
        priority_band: priority_band_for_class(class, identity.sched_policy),
        confidence,
        reasons,
    }
}

pub fn classify_thread(identity: &ThreadIdentity<'_>) -> Classification {
    let thread_comm = identity.thread_comm.to_ascii_lowercase();
    let process_comm = identity.process_comm.to_ascii_lowercase();
    let mut reasons = Vec::new();

    let (class, confidence) = if thread_comm.starts_with("irq/")
        || thread_comm.starts_with("irq-")
        || thread_comm.contains("irq")
    {
        reasons.push(format!(
            "thread_comm '{}' looks like an IRQ thread",
            identity.thread_comm
        ));
        (SystemTaskClass::IrqThread, 0.95)
    } else if thread_comm.contains("audio")
        || thread_comm.contains("pipewire")
        || thread_comm.contains("jack")
        || is_realtime_policy(identity.sched_policy)
    {
        reasons.push(format!(
            "thread_comm '{}' or sched_policy {:?} suggests realtime/audio work",
            identity.thread_comm, identity.sched_policy
        ));
        (SystemTaskClass::AudioRealtime, 0.85)
    } else if matches!(
        identity.process_class,
        SystemTaskClass::Game | SystemTaskClass::GameScope
    ) && (thread_comm.contains("render")
        || thread_comm.contains("rhi")
        || thread_comm.contains("dxvk")
        || thread_comm.contains("vulkan")
        || thread_comm.contains("gpu"))
    {
        reasons.push(format!(
            "game process thread_comm '{}' suggests render/GPU work",
            identity.thread_comm
        ));
        (SystemTaskClass::GameRenderThread, 0.85)
    } else if matches!(
        identity.process_class,
        SystemTaskClass::Game | SystemTaskClass::GameScope
    ) && (thread_comm.contains("worker")
        || thread_comm.contains("task")
        || thread_comm.contains("job")
        || thread_comm.contains("pool"))
    {
        reasons.push(format!(
            "game process thread_comm '{}' suggests worker/job work",
            identity.thread_comm
        ));
        (SystemTaskClass::GameWorkerThread, 0.8)
    } else if matches!(
        identity.process_class,
        SystemTaskClass::BrowserForeground
            | SystemTaskClass::BrowserBackground
            | SystemTaskClass::BrowserRenderer
            | SystemTaskClass::BrowserGpu
            | SystemTaskClass::BrowserNetwork
    ) && (thread_comm.contains("compositor") || thread_comm.contains("render"))
    {
        reasons.push(format!(
            "browser process thread_comm '{}' suggests renderer work",
            identity.thread_comm
        ));
        (SystemTaskClass::BrowserRenderer, 0.75)
    } else if matches!(identity.process_class, SystemTaskClass::BrowserForeground)
        && (thread_comm.contains("socket")
            || thread_comm.contains("network")
            || thread_comm.contains("dns"))
    {
        reasons.push(format!(
            "browser process thread_comm '{}' suggests network work",
            identity.thread_comm
        ));
        (SystemTaskClass::BrowserNetwork, 0.75)
    } else if identity.process_class != SystemTaskClass::Unknown {
        reasons.push(format!(
            "thread inherits process_class {:?} from process_comm '{}'",
            identity.process_class, identity.process_comm
        ));
        (identity.process_class, 0.6)
    } else if process_comm == "python"
        || process_comm.contains("electron")
        || process_comm == "steam"
    {
        reasons.push(format!(
            "process_comm '{}' is ambiguous and thread_comm '{}' did not disambiguate it",
            identity.process_comm, identity.thread_comm
        ));
        (SystemTaskClass::Unknown, 0.3)
    } else {
        reasons.push("no strong thread classification rule matched".to_owned());
        (SystemTaskClass::Unknown, 0.0)
    };

    Classification {
        class,
        priority_band: priority_band_for_class(class, identity.sched_policy),
        confidence,
        reasons,
    }
}

pub fn priority_band_for_class(class: SystemTaskClass, sched_policy: Option<u32>) -> PriorityBand {
    if is_realtime_policy(sched_policy) {
        return PriorityBand::CriticalRealtime;
    }

    match class {
        SystemTaskClass::AudioRealtime | SystemTaskClass::Input | SystemTaskClass::IrqThread => {
            PriorityBand::CriticalRealtime
        }
        SystemTaskClass::Game
        | SystemTaskClass::GameRenderThread
        | SystemTaskClass::GameScope
        | SystemTaskClass::Compositor
        | SystemTaskClass::BrowserForeground => PriorityBand::ForegroundLatency,
        SystemTaskClass::GameWorkerThread
        | SystemTaskClass::WineServer
        | SystemTaskClass::BrowserRenderer
        | SystemTaskClass::BrowserGpu
        | SystemTaskClass::Editor
        | SystemTaskClass::Terminal
        | SystemTaskClass::Shell
        | SystemTaskClass::Media
        | SystemTaskClass::Recorder
        | SystemTaskClass::VirtualMachine => PriorityBand::Interactive,
        SystemTaskClass::BuildJob
        | SystemTaskClass::Compiler
        | SystemTaskClass::Linker
        | SystemTaskClass::PackageManager => PriorityBand::Throughput,
        SystemTaskClass::BrowserBackground
        | SystemTaskClass::BrowserNetwork
        | SystemTaskClass::Indexer
        | SystemTaskClass::StorageDaemon
        | SystemTaskClass::NetworkDaemon
        | SystemTaskClass::KernelThread
        | SystemTaskClass::Service => PriorityBand::Background,
        SystemTaskClass::Unknown => PriorityBand::Unknown,
    }
}

pub fn legacy_task_class_for_system_class(
    class: SystemTaskClass,
) -> crate::process_tree::TaskClass {
    use crate::process_tree::TaskClass;

    match class {
        SystemTaskClass::Game
        | SystemTaskClass::GameRenderThread
        | SystemTaskClass::GameWorkerThread => TaskClass::Game,

        SystemTaskClass::WineServer => TaskClass::WineServer,
        SystemTaskClass::GameScope => TaskClass::GameScope,
        SystemTaskClass::Compositor => TaskClass::Compositor,

        SystemTaskClass::Service
        | SystemTaskClass::StorageDaemon
        | SystemTaskClass::NetworkDaemon
        | SystemTaskClass::KernelThread
        | SystemTaskClass::IrqThread => TaskClass::Service,

        _ => TaskClass::Helper,
    }
}

fn is_realtime_policy(sched_policy: Option<u32>) -> bool {
    matches!(sched_policy, Some(SCHED_FIFO | SCHED_RR | SCHED_DEADLINE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_tree::TaskClass;

    fn test_classification(
        class: SystemTaskClass,
        priority_band: PriorityBand,
        confidence: f32,
    ) -> Classification {
        Classification {
            class,
            priority_band,
            confidence,
            reasons: vec![format!("test classification {class:?}")],
        }
    }

    fn test_process(
        pid: u32,
        ppid: u32,
        comm: &str,
        class: SystemTaskClass,
        priority_band: PriorityBand,
        cpu_time_ticks_delta: u64,
    ) -> FocusProcess {
        FocusProcess {
            pid,
            ppid,
            comm: comm.to_owned(),
            cmdline: comm.to_owned(),
            cgroup_path: None,
            starttime_ticks: Some(pid as u64 * 10),
            sched_policy: None,
            classification: test_classification(class, priority_band, 0.9),
            cpu_time_ticks_delta,
            read_bytes_delta: 0,
            write_bytes_delta: 0,
            voluntary_ctxt_switches_delta: 0,
            nonvoluntary_ctxt_switches_delta: 0,
        }
    }

    fn test_snapshot(processes: Vec<FocusProcess>) -> FocusSnapshot {
        let mut process_map = BTreeMap::new();
        let mut children_by_parent: BTreeMap<u32, Vec<u32>> = BTreeMap::new();

        for process in processes {
            children_by_parent
                .entry(process.ppid)
                .or_default()
                .push(process.pid);
            process_map.insert(process.pid, process);
        }

        for children in children_by_parent.values_mut() {
            children.sort_unstable();
        }

        FocusSnapshot {
            elapsed_ms: 1000,
            processes: process_map,
            children_by_parent,
            groups: Vec::new(),
        }
    }

    #[test]
    fn focus_groups_prefer_stable_compile_root_over_compiler_children() {
        let snapshot = test_snapshot(vec![
            test_process(
                10,
                1,
                "foot",
                SystemTaskClass::Terminal,
                PriorityBand::Interactive,
                1,
            ),
            test_process(
                11,
                10,
                "cargo",
                SystemTaskClass::BuildJob,
                PriorityBand::Throughput,
                2,
            ),
            test_process(
                12,
                11,
                "rustc",
                SystemTaskClass::Compiler,
                PriorityBand::Throughput,
                80,
            ),
            test_process(
                13,
                11,
                "ld.lld",
                SystemTaskClass::Linker,
                PriorityBand::Throughput,
                25,
            ),
        ]);

        let groups = build_focus_groups(&snapshot);
        let compile = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Compile)
            .unwrap();

        assert_eq!(compile.root_pids, vec![11]);
        assert_eq!(compile.primary_pid, Some(11));
        assert_eq!(compile.member_pids, vec![11, 12, 13]);
    }

    #[test]
    fn focus_groups_group_orphan_compilers_under_nearest_terminal_session() {
        let snapshot = test_snapshot(vec![
            test_process(
                20,
                1,
                "kitty",
                SystemTaskClass::Terminal,
                PriorityBand::Interactive,
                3,
            ),
            test_process(
                21,
                20,
                "zsh",
                SystemTaskClass::Shell,
                PriorityBand::Interactive,
                4,
            ),
            test_process(
                22,
                21,
                "rustc",
                SystemTaskClass::Compiler,
                PriorityBand::Throughput,
                60,
            ),
            test_process(
                23,
                21,
                "clang",
                SystemTaskClass::Compiler,
                PriorityBand::Throughput,
                50,
            ),
        ]);

        let groups = build_focus_groups(&snapshot);
        let compile = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Compile)
            .unwrap();

        assert_eq!(compile.root_pids, vec![21]);
        assert_eq!(compile.primary_pid, Some(22));
        assert_eq!(compile.member_pids, vec![21, 22, 23]);
    }

    #[test]
    fn focus_groups_root_browser_at_parent_not_idle_renderer() {
        let snapshot = test_snapshot(vec![
            test_process(
                30,
                1,
                "firefox",
                SystemTaskClass::BrowserForeground,
                PriorityBand::ForegroundLatency,
                5,
            ),
            test_process(
                31,
                30,
                "Web Content",
                SystemTaskClass::BrowserRenderer,
                PriorityBand::Interactive,
                100,
            ),
            test_process(
                32,
                30,
                "GPU Process",
                SystemTaskClass::BrowserGpu,
                PriorityBand::Interactive,
                20,
            ),
        ]);

        let groups = build_focus_groups(&snapshot);
        let browser = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Browser)
            .unwrap();

        assert_eq!(browser.root_pids, vec![30]);
        assert_eq!(browser.primary_pid, Some(30));
        assert_eq!(browser.member_pids, vec![30, 31, 32]);
    }

    #[test]
    fn focus_groups_include_wineserver_tied_to_game_runtime() {
        let mut game = test_process(
            40,
            1,
            "pressure-vessel",
            SystemTaskClass::Game,
            PriorityBand::ForegroundLatency,
            10,
        );
        game.cmdline = "/home/user/.steam/steamapps/common/Game/pressure-vessel".to_owned();
        game.cgroup_path = Some(PathBuf::from("/user.slice/app-steam-game.scope"));

        let mut game_child = test_process(
            41,
            40,
            "Game.exe",
            SystemTaskClass::Game,
            PriorityBand::ForegroundLatency,
            120,
        );
        game_child.cmdline = "/home/user/.steam/steamapps/common/Game/Game.exe".to_owned();
        game_child.cgroup_path = Some(PathBuf::from("/user.slice/app-steam-game.scope"));

        let mut wineserver = test_process(
            42,
            1,
            "wineserver",
            SystemTaskClass::WineServer,
            PriorityBand::ForegroundLatency,
            15,
        );
        wineserver.cgroup_path = Some(PathBuf::from("/user.slice/app-steam-game.scope"));

        let snapshot = test_snapshot(vec![game, game_child, wineserver]);

        let groups = build_focus_groups(&snapshot);
        let game_group = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Game)
            .unwrap();

        assert_eq!(game_group.root_pids, vec![40]);
        assert_eq!(game_group.primary_pid, Some(40));
        assert_eq!(game_group.member_pids, vec![40, 41, 42]);
    }

    #[test]
    fn focus_groups_do_not_let_idle_steam_beat_active_compile() {
        let mut steam = test_process(
            50,
            1,
            "steam",
            SystemTaskClass::Service,
            PriorityBand::Background,
            0,
        );
        steam.cmdline = "steam".to_owned();

        let cargo = test_process(
            60,
            1,
            "cargo",
            SystemTaskClass::BuildJob,
            PriorityBand::Throughput,
            30,
        );
        let rustc = test_process(
            61,
            60,
            "rustc",
            SystemTaskClass::Compiler,
            PriorityBand::Throughput,
            90,
        );

        let snapshot = test_snapshot(vec![steam, cargo, rustc]);

        let groups = build_focus_groups(&snapshot);

        assert_eq!(groups.first().unwrap().kind, FocusGroupKind::Compile);
        assert!(
            groups
                .iter()
                .position(|group| group.kind == FocusGroupKind::Idle)
                .unwrap()
                > groups
                    .iter()
                    .position(|group| group.kind == FocusGroupKind::Compile)
                    .unwrap()
        );
    }

    #[test]
    fn focus_groups_fallback_selects_highest_non_service_interactive_tree_by_cpu() {
        let service = test_process(
            70,
            1,
            "systemd",
            SystemTaskClass::Service,
            PriorityBand::Background,
            500,
        );
        let editor = test_process(
            80,
            1,
            "nvim",
            SystemTaskClass::Editor,
            PriorityBand::Interactive,
            20,
        );
        let terminal = test_process(
            90,
            1,
            "foot",
            SystemTaskClass::Terminal,
            PriorityBand::Interactive,
            60,
        );

        let mut snapshot = test_snapshot(vec![service, editor, terminal]);
        for process in snapshot.processes.values_mut() {
            process.classification.class = SystemTaskClass::Unknown;
        }
        snapshot
            .processes
            .get_mut(&70)
            .unwrap()
            .classification
            .class = SystemTaskClass::Service;
        snapshot
            .processes
            .get_mut(&80)
            .unwrap()
            .classification
            .class = SystemTaskClass::Editor;
        snapshot
            .processes
            .get_mut(&90)
            .unwrap()
            .classification
            .class = SystemTaskClass::Terminal;

        let groups = build_focus_groups(&snapshot);
        let fallback = groups
            .iter()
            .find(|group| group.kind == FocusGroupKind::Unknown)
            .unwrap();

        assert_eq!(fallback.root_pids, vec![90]);
        assert_eq!(fallback.primary_pid, Some(90));
        assert_eq!(fallback.member_pids, vec![90]);
    }

    #[test]
    fn focus_counter_deltas_are_zero_on_first_seen_and_reset_on_pid_reuse() {
        let current = FocusCounters {
            starttime_ticks: Some(10),
            cpu_time_ticks: 100,
            read_bytes: 200,
            write_bytes: 300,
            voluntary_ctxt_switches: 40,
            nonvoluntary_ctxt_switches: 50,
        };

        let first_seen = counter_deltas(None, &current);
        assert_eq!(first_seen.starttime_ticks, Some(10));
        assert_eq!(first_seen.cpu_time_ticks, 0);
        assert_eq!(first_seen.read_bytes, 0);
        assert_eq!(first_seen.write_bytes, 0);
        assert_eq!(first_seen.voluntary_ctxt_switches, 0);
        assert_eq!(first_seen.nonvoluntary_ctxt_switches, 0);

        let previous = FocusCounters {
            starttime_ticks: Some(10),
            cpu_time_ticks: 70,
            read_bytes: 125,
            write_bytes: 250,
            voluntary_ctxt_switches: 35,
            nonvoluntary_ctxt_switches: 45,
        };

        let deltas = counter_deltas(Some(&previous), &current);
        assert_eq!(deltas.starttime_ticks, Some(10));
        assert_eq!(deltas.cpu_time_ticks, 30);
        assert_eq!(deltas.read_bytes, 75);
        assert_eq!(deltas.write_bytes, 50);
        assert_eq!(deltas.voluntary_ctxt_switches, 5);
        assert_eq!(deltas.nonvoluntary_ctxt_switches, 5);

        let reused_pid_previous = FocusCounters {
            starttime_ticks: Some(9),
            cpu_time_ticks: 500,
            read_bytes: 600,
            write_bytes: 700,
            voluntary_ctxt_switches: 80,
            nonvoluntary_ctxt_switches: 90,
        };

        let reused_pid_deltas = counter_deltas(Some(&reused_pid_previous), &current);
        assert_eq!(reused_pid_deltas.starttime_ticks, Some(10));
        assert_eq!(reused_pid_deltas.cpu_time_ticks, 0);
        assert_eq!(reused_pid_deltas.read_bytes, 0);
        assert_eq!(reused_pid_deltas.write_bytes, 0);
        assert_eq!(reused_pid_deltas.voluntary_ctxt_switches, 0);
        assert_eq!(reused_pid_deltas.nonvoluntary_ctxt_switches, 0);
    }

    #[test]
    fn focus_group_kind_maps_system_classes() {
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::GameRenderThread),
            FocusGroupKind::Game
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::BrowserGpu),
            FocusGroupKind::Browser
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::Compiler),
            FocusGroupKind::Compile
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::Media),
            FocusGroupKind::Media
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::Recorder),
            FocusGroupKind::Recording
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::VirtualMachine),
            FocusGroupKind::VirtualMachine
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::Compositor),
            FocusGroupKind::Desktop
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::Service),
            FocusGroupKind::Idle
        );
        assert_eq!(
            focus_group_kind_for_class(SystemTaskClass::Unknown),
            FocusGroupKind::Unknown
        );
    }

    #[test]
    fn legacy_task_class_maps_game_related_system_classes_to_game() {
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::Game),
            TaskClass::Game
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::GameRenderThread),
            TaskClass::Game
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::GameWorkerThread),
            TaskClass::Game
        );
    }

    #[test]
    fn legacy_task_class_preserves_special_foreground_classes() {
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::WineServer),
            TaskClass::WineServer
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::GameScope),
            TaskClass::GameScope
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::Compositor),
            TaskClass::Compositor
        );
    }

    #[test]
    fn legacy_task_class_maps_daemon_and_kernel_classes_to_service() {
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::Service),
            TaskClass::Service
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::StorageDaemon),
            TaskClass::Service
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::NetworkDaemon),
            TaskClass::Service
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::KernelThread),
            TaskClass::Service
        );
        assert_eq!(
            legacy_task_class_for_system_class(SystemTaskClass::IrqThread),
            TaskClass::Service
        );
    }

    #[test]
    fn legacy_task_class_maps_all_other_system_classes_to_helper() {
        let classes = [
            SystemTaskClass::AudioRealtime,
            SystemTaskClass::Input,
            SystemTaskClass::BrowserForeground,
            SystemTaskClass::BrowserBackground,
            SystemTaskClass::BrowserRenderer,
            SystemTaskClass::BrowserGpu,
            SystemTaskClass::BrowserNetwork,
            SystemTaskClass::BuildJob,
            SystemTaskClass::Compiler,
            SystemTaskClass::Linker,
            SystemTaskClass::Indexer,
            SystemTaskClass::PackageManager,
            SystemTaskClass::Editor,
            SystemTaskClass::Terminal,
            SystemTaskClass::Shell,
            SystemTaskClass::Media,
            SystemTaskClass::Recorder,
            SystemTaskClass::VirtualMachine,
            SystemTaskClass::Unknown,
        ];

        for class in classes {
            assert_eq!(legacy_task_class_for_system_class(class), TaskClass::Helper);
        }
    }
}
