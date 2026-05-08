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

    let groups = build_focus_groups(&focus_processes);

    FocusSnapshot {
        elapsed_ms,
        processes: focus_processes,
        children_by_parent,
        groups,
    }
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

fn build_focus_groups(processes: &BTreeMap<u32, FocusProcess>) -> Vec<FocusGroup> {
    const ORDER: [FocusGroupKind; 9] = [
        FocusGroupKind::Game,
        FocusGroupKind::Browser,
        FocusGroupKind::Compile,
        FocusGroupKind::Media,
        FocusGroupKind::Recording,
        FocusGroupKind::VirtualMachine,
        FocusGroupKind::Desktop,
        FocusGroupKind::Idle,
        FocusGroupKind::Unknown,
    ];

    let mut groups = Vec::new();

    for kind in ORDER {
        let member_pids = processes
            .values()
            .filter(|process| focus_group_kind_for_class(process.classification.class) == kind)
            .map(|process| process.pid)
            .collect::<Vec<_>>();

        if member_pids.is_empty() {
            continue;
        }

        let member_set = member_pids.iter().copied().collect::<BTreeSet<_>>();
        let root_pids = member_pids
            .iter()
            .copied()
            .filter(|pid| {
                processes
                    .get(pid)
                    .map(|process| !member_set.contains(&process.ppid))
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();

        let primary_pid = member_pids.iter().copied().max_by(|left, right| {
            let left_score = processes
                .get(left)
                .map(process_focus_score)
                .unwrap_or_default();
            let right_score = processes
                .get(right)
                .map(process_focus_score)
                .unwrap_or_default();

            left_score
                .partial_cmp(&right_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let score = member_pids
            .iter()
            .filter_map(|pid| processes.get(pid))
            .map(process_focus_score)
            .sum::<f32>();

        let confidence = member_pids
            .iter()
            .filter_map(|pid| processes.get(pid))
            .map(|process| process.classification.confidence)
            .fold(0.0_f32, f32::max);

        let priority_band = member_pids
            .iter()
            .filter_map(|pid| processes.get(pid))
            .map(|process| process.classification.priority_band)
            .max_by_key(|band| priority_band_rank(*band))
            .unwrap_or(PriorityBand::Unknown);

        let display_name =
            display_name_for_group(kind, primary_pid.and_then(|pid| processes.get(&pid)));

        let mut reasons = vec![format!(
            "{} process(es) classified as {:?}",
            member_pids.len(),
            kind
        )];
        if let Some(primary_pid) = primary_pid
            && let Some(primary) = processes.get(&primary_pid)
        {
            reasons.push(format!(
                "primary pid={} comm='{}' class={:?}",
                primary.pid, primary.comm, primary.classification.class
            ));
        }

        groups.push(FocusGroup {
            kind,
            root_pids,
            member_pids,
            primary_pid,
            display_name,
            score,
            confidence,
            priority_band,
            reasons,
        });
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

        SystemTaskClass::Compositor
        | SystemTaskClass::AudioRealtime
        | SystemTaskClass::Input
        | SystemTaskClass::Editor
        | SystemTaskClass::Terminal
        | SystemTaskClass::Shell => FocusGroupKind::Desktop,

        SystemTaskClass::StorageDaemon
        | SystemTaskClass::NetworkDaemon
        | SystemTaskClass::KernelThread
        | SystemTaskClass::IrqThread
        | SystemTaskClass::Service => FocusGroupKind::Idle,

        SystemTaskClass::Unknown => FocusGroupKind::Unknown,
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
