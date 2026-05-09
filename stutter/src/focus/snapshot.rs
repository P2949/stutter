use super::{groups::*, *};

#[derive(Debug, Clone)]
pub struct FocusSnapshot {
    pub elapsed_ms: u64,
    pub foreground: Option<ForegroundWindowSnapshot>,
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
    pub is_foreground_window_process: bool,

    pub classification: Classification,

    pub cpu_time_ticks_delta: u64,
    pub read_bytes_delta: u64,
    pub write_bytes_delta: u64,
    pub voluntary_ctxt_switches_delta: u64,
    pub nonvoluntary_ctxt_switches_delta: u64,
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
}

pub fn focus_snapshot_at(
    proc_root: &Path,
    cache: &mut FocusCache,
    elapsed_ms: u64,
    foreground: Option<&ForegroundWindowSnapshot>,
) -> FocusSnapshot {
    let budget = crate::process_tree::ScanBudget::default_proc_scan();
    let mut budget_report = crate::process_tree::ScanBudgetReport::default();
    let processes = crate::process_tree::scan_processes_at(
        proc_root,
        &mut cache.proc_cache,
        &budget,
        &mut budget_report,
    );

    build_focus_snapshot_from_processes(
        proc_root,
        cache,
        elapsed_ms,
        processes,
        foreground.cloned(),
    )
}

pub fn build_focus_snapshot_from_processes(
    proc_root: &Path,
    cache: &mut FocusCache,
    elapsed_ms: u64,
    processes: BTreeMap<u32, crate::process_tree::ProcInfo>,
    foreground: Option<ForegroundWindowSnapshot>,
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
        let is_foreground_window_process = foreground
            .as_ref()
            .and_then(|fg| fg.pid)
            .is_some_and(|pid| pid == proc_info.pid);

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
            is_foreground_window_process,
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
        foreground,
        processes: focus_processes,
        children_by_parent,
        groups: Vec::new(),
    };
    snapshot.groups = build_focus_groups(&snapshot);

    snapshot
}

pub(crate) fn read_focus_counters_at(proc_root: &Path, pid: u32) -> FocusCounters {
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

pub(crate) fn read_focus_stat_counters_at(path: &Path) -> Option<(u64, Option<u64>)> {
    let stat = fs::read_to_string(path).ok()?;
    let (_, after_comm) = stat.rsplit_once(") ")?;
    let fields = after_comm.split_whitespace().collect::<Vec<_>>();

    let utime = fields.get(11)?.parse::<u64>().ok()?;
    let stime = fields.get(12)?.parse::<u64>().ok()?;
    let starttime_ticks = fields.get(19).and_then(|value| value.parse::<u64>().ok());

    Some((utime.saturating_add(stime), starttime_ticks))
}

pub(crate) fn read_focus_io_counters_at(path: &Path) -> Option<(u64, u64)> {
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

pub(crate) fn read_focus_status_context_switches_at(path: &Path) -> Option<(u64, u64)> {
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

pub(crate) fn counter_deltas(
    previous: Option<&FocusCounters>,
    current: &FocusCounters,
) -> FocusCounters {
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

pub(crate) fn non_empty_str(value: &str) -> Option<&str> {
    if value.is_empty() { None } else { Some(value) }
}
