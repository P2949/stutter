mod process_tree;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

use aya::{
    Ebpf,
    maps::{HashMap as AyaHashMap, MapData, RingBuf},
    programs::TracePoint,
};
use log::{debug, info, warn};
use stutter_common::{EVENT_RUNNABLE_LATENCY, SchedulerEvent};
use tokio::{
    io::unix::AsyncFd,
    signal,
    time::{MissedTickBehavior, interval},
};

#[derive(Clone, Copy)]
struct SpikeRecord {
    latency_ns: u64,
    cpu: u32,
    prio: i32,
    wakeup_ns: u64,
    switch_ns: u64,
}

#[derive(Debug)]
struct Config {
    target_pids: Vec<u32>,
    tree_pids: Vec<u32>,
    summary_period_ms: u64,
    spike_threshold_ns: u64,
    verbose: bool,
}

struct ProcessStats {
    comm: String,

    interval_latency: LatencyStats,
    interval_cpu: CpuStatsSet,

    session_latency: LatencyStats,
    session_cpu: CpuStatsSet,
    top_spikes: Vec<SpikeRecord>,
}

struct LatencyStats {
    count: u64,
    min_ns: u64,
    max_ns: u64,
    sum_ns: u128,
    over_1ms: u64,
    over_2ms: u64,
    over_5ms: u64,
    samples_ns: Vec<u64>,
    samples_truncated: u64,
}

#[derive(Clone, Copy)]
struct CpuStats {
    samples: u64,
    max_ns: u64,
    spikes: u64,
}

struct CpuStatsSet {
    by_cpu: BTreeMap<u32, CpuStats>,
}

#[derive(Clone, Copy)]
struct LatencySnapshot {
    count: u64,
    min_ns: u64,
    avg_ns: u64,
    max_ns: u64,
    p95_ns: u64,
    p99_ns: u64,
    over_1ms: u64,
    over_2ms: u64,
    over_5ms: u64,
    samples_truncated: u64,
}

#[derive(Clone, Copy)]
struct CpuLine {
    cpu: u32,
    samples: u64,
    max_ns: u64,
    spikes: u64,
}

struct CpuSnapshot {
    busiest_cpu: Option<u32>,
    busiest_cpu_samples: u64,
    worst_cpu: Option<u32>,
    worst_cpu_max_ns: u64,
    spikiest_cpu: Option<u32>,
    spikiest_cpu_spikes: u64,
    per_cpu: Vec<CpuLine>,
}

const TARGET_PIDS_MAX: usize = 1024;
const MAX_EXACT_SAMPLES: usize = 65_536;

impl LatencyStats {
    fn new() -> Self {
        Self {
            count: 0,
            min_ns: 0,
            max_ns: 0,
            sum_ns: 0,
            over_1ms: 0,
            over_2ms: 0,
            over_5ms: 0,
            samples_ns: Vec::with_capacity(4096),
            samples_truncated: 0,
        }
    }

    fn record(&mut self, latency_ns: u64) {
        if self.count == 0 {
            self.min_ns = latency_ns;
            self.max_ns = latency_ns;
        } else {
            self.min_ns = self.min_ns.min(latency_ns);
            self.max_ns = self.max_ns.max(latency_ns);
        }

        self.count += 1;
        self.sum_ns += latency_ns as u128;

        if latency_ns >= 1_000_000 {
            self.over_1ms += 1;
        }
        if latency_ns >= 2_000_000 {
            self.over_2ms += 1;
        }
        if latency_ns >= 5_000_000 {
            self.over_5ms += 1;
        }

        if self.samples_ns.len() < MAX_EXACT_SAMPLES {
            self.samples_ns.push(latency_ns);
        } else {
            self.samples_truncated += 1;
        }
    }

    fn snapshot(&mut self) -> Option<LatencySnapshot> {
        if self.count == 0 {
            return None;
        }

        self.samples_ns.sort_unstable();

        Some(LatencySnapshot {
            count: self.count,
            min_ns: self.min_ns,
            avg_ns: (self.sum_ns / self.count as u128) as u64,
            max_ns: self.max_ns,
            p95_ns: percentile_from_sorted(&self.samples_ns, 0.95),
            p99_ns: percentile_from_sorted(&self.samples_ns, 0.99),
            over_1ms: self.over_1ms,
            over_2ms: self.over_2ms,
            over_5ms: self.over_5ms,
            samples_truncated: self.samples_truncated,
        })
    }

    fn snapshot_and_reset(&mut self) -> Option<LatencySnapshot> {
        let snapshot = self.snapshot()?;
        *self = Self::new();
        Some(snapshot)
    }
}

impl CpuStats {
    fn new() -> Self {
        Self {
            samples: 0,
            max_ns: 0,
            spikes: 0,
        }
    }

    fn record(&mut self, latency_ns: u64, spike_threshold_ns: u64) {
        self.samples += 1;
        self.max_ns = self.max_ns.max(latency_ns);

        if latency_ns >= spike_threshold_ns {
            self.spikes += 1;
        }
    }
}

impl CpuStatsSet {
    fn new() -> Self {
        Self {
            by_cpu: BTreeMap::new(),
        }
    }

    fn record(&mut self, cpu: u32, latency_ns: u64, spike_threshold_ns: u64) {
        self.by_cpu
            .entry(cpu)
            .or_insert_with(CpuStats::new)
            .record(latency_ns, spike_threshold_ns);
    }

    fn snapshot(&self) -> CpuSnapshot {
        let mut busiest_cpu = None;
        let mut busiest_cpu_samples = 0;

        let mut worst_cpu = None;
        let mut worst_cpu_max_ns = 0;

        let mut spikiest_cpu = None;
        let mut spikiest_cpu_spikes = 0;

        let mut per_cpu = Vec::with_capacity(self.by_cpu.len());

        for (cpu, stats) in &self.by_cpu {
            if stats.samples > busiest_cpu_samples {
                busiest_cpu = Some(*cpu);
                busiest_cpu_samples = stats.samples;
            }

            if stats.max_ns > worst_cpu_max_ns {
                worst_cpu = Some(*cpu);
                worst_cpu_max_ns = stats.max_ns;
            }

            if stats.spikes > spikiest_cpu_spikes {
                spikiest_cpu = Some(*cpu);
                spikiest_cpu_spikes = stats.spikes;
            }

            per_cpu.push(CpuLine {
                cpu: *cpu,
                samples: stats.samples,
                max_ns: stats.max_ns,
                spikes: stats.spikes,
            });
        }

        CpuSnapshot {
            busiest_cpu,
            busiest_cpu_samples,
            worst_cpu,
            worst_cpu_max_ns,
            spikiest_cpu,
            spikiest_cpu_spikes,
            per_cpu,
        }
    }

    fn snapshot_and_reset(&mut self) -> CpuSnapshot {
        let snapshot = self.snapshot();
        self.by_cpu.clear();
        snapshot
    }
}

impl ProcessStats {
    fn new(comm: String) -> Self {
        Self {
            comm,
            interval_latency: LatencyStats::new(),
            interval_cpu: CpuStatsSet::new(),
            session_latency: LatencyStats::new(),
            session_cpu: CpuStatsSet::new(),
            top_spikes: Vec::with_capacity(16),
        }
    }

    fn record(&mut self, event: &SchedulerEvent, spike_threshold_ns: u64) {
        self.interval_latency.record(event.latency_ns);
        self.interval_cpu
            .record(event.cpu, event.latency_ns, spike_threshold_ns);

        self.session_latency.record(event.latency_ns);
        self.session_cpu
            .record(event.cpu, event.latency_ns, spike_threshold_ns);

        if event.latency_ns >= spike_threshold_ns {
            self.top_spikes.push(SpikeRecord {
                latency_ns: event.latency_ns,
                cpu: event.cpu,
                prio: event.prio,
                wakeup_ns: event.wakeup_ns,
                switch_ns: event.switch_ns,
            });

            self.top_spikes
                .sort_unstable_by_key(|spike| std::cmp::Reverse(spike.latency_ns));

            self.top_spikes.truncate(16);
        }
    }
}

fn percentile_from_sorted(samples: &[u64], percentile: f64) -> u64 {
    if samples.is_empty() {
        return 0;
    }

    let rank = ((samples.len() as f64 * percentile).ceil() as usize).saturating_sub(1);
    let idx = rank.min(samples.len() - 1);

    samples[idx]
}

fn parse_config() -> anyhow::Result<Config> {
    let mut target_pids = Vec::new();
    let mut tree_pids = Vec::new();
    let mut summary_period_ms = 1_000;
    let mut spike_threshold_ns = 1_000_000;
    let mut verbose = false;

    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--tree-pid" => {
                let Some(value) = args.next() else {
                    anyhow::bail!("{arg} requires a PID value");
                };

                let pid = value.parse::<u32>()?;

                if pid == 0 {
                    anyhow::bail!("--tree-pid must be greater than zero");
                }

                tree_pids.push(pid);
            }
            "--pid" | "-p" => {
                let Some(value) = args.next() else {
                    anyhow::bail!("{arg} requires a PID value");
                };

                let pid = value.parse::<u32>()?;

                if pid == 0 {
                    anyhow::bail!("--pid must be greater than zero");
                }

                target_pids.push(pid);
            }
            "--summary-ms" => {
                let Some(value) = args.next() else {
                    anyhow::bail!("{arg} requires a millisecond value");
                };

                summary_period_ms = value.parse::<u64>()?;

                if summary_period_ms == 0 {
                    anyhow::bail!("--summary-ms must be greater than zero");
                }
            }
            "--spike-us" => {
                let Some(value) = args.next() else {
                    anyhow::bail!("{arg} requires a microsecond value");
                };

                let spike_us = value.parse::<u64>()?;

                if spike_us == 0 {
                    anyhow::bail!("--spike-us must be greater than zero");
                }

                spike_threshold_ns = spike_us
                    .checked_mul(1_000)
                    .ok_or_else(|| anyhow::anyhow!("--spike-us value is too large"))?;
            }
            "--verbose" | "-v" => {
                verbose = true;
            }
            "--help" | "-h" => {
                println!(
                    "Usage: stutter --pid <PID> [--pid <PID> ...] [--tree-pid <PID> ...] [--summary-ms <MS>] [--spike-us <US>] [--verbose]"
                );
                std::process::exit(0);
            }
            other => {
                anyhow::bail!("unknown argument: {other}");
            }
        }
    }

    if target_pids.is_empty() && tree_pids.is_empty() {
        anyhow::bail!("at least one --pid <PID> or --tree-pid <PID> is required");
    }

    target_pids.sort_unstable();
    target_pids.dedup();

    tree_pids.sort_unstable();
    tree_pids.dedup();

    if target_pids.len() > TARGET_PIDS_MAX {
        anyhow::bail!(
            "too many unique target PIDs: got {}, but TARGET_PIDS supports at most {}",
            target_pids.len(),
            TARGET_PIDS_MAX
        );
    }

    Ok(Config {
        target_pids,
        tree_pids,
        summary_period_ms,
        spike_threshold_ns,
        verbose,
    })
}

fn attach_tracepoint(
    ebpf: &mut Ebpf,
    program_name: &str,
    category: &str,
    tracepoint_name: &str,
) -> anyhow::Result<()> {
    let program: &mut TracePoint = ebpf
        .program_mut(program_name)
        .ok_or_else(|| anyhow::anyhow!("{program_name} program not found"))?
        .try_into()?;

    program.load()?;
    program.attach(category, tracepoint_name)?;

    Ok(())
}

fn comm_to_string(comm: &[u8; 16]) -> String {
    std::str::from_utf8(comm)
        .unwrap_or("?")
        .trim_end_matches('\0')
        .to_owned()
}

fn format_latency(ns: u64) -> String {
    if ns >= 1_000_000 {
        format!("{:.3}ms", ns as f64 / 1_000_000.0)
    } else {
        format!("{:.3}us", ns as f64 / 1_000.0)
    }
}

fn format_cpu(cpu: Option<u32>) -> String {
    match cpu {
        Some(cpu) => cpu.to_string(),
        None => "-".to_owned(),
    }
}

fn print_event(event: &SchedulerEvent, comm: &str, label: &str) {
    let latency_us = event.latency_ns as f64 / 1_000.0;
    let latency_ms = event.latency_ns as f64 / 1_000_000.0;

    info!(
        "{label} runnable_latency={latency_us:.3}us ({latency_ms:.6}ms) pid={} cpu={} prio={} comm={} wakeup_ns={} switch_ns={}",
        event.pid, event.cpu, event.prio, comm, event.wakeup_ns, event.switch_ns,
    );
}

fn print_latency_line(
    label: &str,
    pid: u32,
    comm: &str,
    latency: LatencySnapshot,
    cpu: &CpuSnapshot,
) {
    info!(
        "{label} pid={} comm={} samples={} stored_samples={} truncated_samples={} min={} avg={} p95={} p99={} max={} over_1ms={} over_2ms={} over_5ms={} busiest_cpu={} busiest_cpu_samples={} worst_cpu={} worst_cpu_max={} spikiest_cpu={} spikiest_cpu_spikes={}",
        pid,
        comm,
        latency.count,
        latency.count.min(MAX_EXACT_SAMPLES as u64),
        latency.samples_truncated,
        format_latency(latency.min_ns),
        format_latency(latency.avg_ns),
        format_latency(latency.p95_ns),
        format_latency(latency.p99_ns),
        format_latency(latency.max_ns),
        latency.over_1ms,
        latency.over_2ms,
        latency.over_5ms,
        format_cpu(cpu.busiest_cpu),
        cpu.busiest_cpu_samples,
        format_cpu(cpu.worst_cpu),
        format_latency(cpu.worst_cpu_max_ns),
        format_cpu(cpu.spikiest_cpu),
        cpu.spikiest_cpu_spikes,
    );
}

fn print_interval_summaries(stats_by_pid: &mut BTreeMap<u32, ProcessStats>) {
    for (pid, process) in stats_by_pid.iter_mut() {
        let Some(latency) = process.interval_latency.snapshot_and_reset() else {
            continue;
        };

        let cpu = process.interval_cpu.snapshot_and_reset();

        print_latency_line("summary", *pid, &process.comm, latency, &cpu);
    }
}

fn print_session_summaries(stats_by_pid: &mut BTreeMap<u32, ProcessStats>) {
    for (pid, process) in stats_by_pid.iter_mut() {
        let Some(latency) = process.session_latency.snapshot() else {
            continue;
        };

        let cpu = process.session_cpu.snapshot();

        print_latency_line("session", *pid, &process.comm, latency, &cpu);

        for cpu_line in cpu.per_cpu {
            info!(
                "session_cpu pid={} comm={} cpu={} samples={} max={} spikes={}",
                pid,
                process.comm,
                cpu_line.cpu,
                cpu_line.samples,
                format_latency(cpu_line.max_ns),
                cpu_line.spikes,
            );
        }

        for spike in &process.top_spikes {
            info!(
                "session_spike pid={} comm={} cpu={} prio={} latency={} wakeup_ns={} switch_ns={}",
                pid,
                process.comm,
                spike.cpu,
                spike.prio,
                format_latency(spike.latency_ns),
                spike.wakeup_ns,
                spike.switch_ns,
            );
        }
    }
}

fn refresh_target_pids(
    config: &Config,
    active_target_pids: &mut BTreeSet<u32>,
    target_pid_map: &mut AyaHashMap<MapData, u32, u8>,
) -> anyhow::Result<()> {
    let processes = process_tree::scan_processes();

    let mut desired_processes = BTreeSet::new();

    for pid in &config.target_pids {
        desired_processes.insert(*pid);
    }

    for root_pid in &config.tree_pids {
        desired_processes.insert(*root_pid);
        desired_processes.extend(process_tree::descendants_of(*root_pid, &processes));
    }

    let mut desired_tasks = BTreeSet::new();
    let mut task_owner: BTreeMap<u32, u32> = BTreeMap::new();

    for pid in &desired_processes {
        let tids = process_tree::thread_ids_of(*pid);

        if tids.is_empty() {
            desired_tasks.insert(*pid);
            task_owner.insert(*pid, *pid);
        } else {
            for tid in tids {
                desired_tasks.insert(tid);
                task_owner.insert(tid, *pid);
            }
        }
    }

    if desired_tasks.len() > TARGET_PIDS_MAX {
        anyhow::bail!(
            "too many target tasks after tree/thread expansion: got {}, but TARGET_PIDS supports at most {}",
            desired_tasks.len(),
            TARGET_PIDS_MAX
        );
    }

    for tid in desired_tasks.difference(active_target_pids) {
        target_pid_map.insert(*tid, 1, 0).map_err(|e| {
            anyhow::anyhow!(
                "failed to insert TID {tid} into TARGET_PIDS during tree refresh \
                 (map full? eBPF TARGET_PIDS max_entries is {TARGET_PIDS_MAX}): {e}"
            )
        })?;

        if let Some(owner_pid) = task_owner.get(tid) {
            if let Some(proc_info) = processes.get(owner_pid) {
                info!(
                    "tree_target_added tid={} process_pid={} ppid={} comm={}",
                    tid, proc_info.pid, proc_info.ppid, proc_info.comm
                );
                debug!(
                    "tree_target_added_cmdline tid={} process_pid={} cmdline={}",
                    tid, proc_info.pid, proc_info.cmdline
                );
            } else {
                info!("tree_target_added tid={} process_pid={}", tid, owner_pid);
            }
        } else {
            info!("tree_target_added tid={tid}");
        }
    }

    for tid in active_target_pids.difference(&desired_tasks) {
        match target_pid_map.remove(tid) {
            Ok(()) => info!("tree_target_removed tid={tid}"),
            Err(e) => warn!("tree_target_remove_failed tid={tid} err={e}"),
        }
    }

    *active_target_pids = desired_tasks;

    debug!(
        "tree_refresh_complete active_tasks={} manual_roots={} tree_roots={} process_count={}",
        active_target_pids.len(),
        config.target_pids.len(),
        config.tree_pids.len(),
        desired_processes.len(),
    );

    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let config = parse_config()?;

    let rlim = libc::rlimit {
        rlim_cur: libc::RLIM_INFINITY,
        rlim_max: libc::RLIM_INFINITY,
    };

    let ret = unsafe { libc::setrlimit(libc::RLIMIT_MEMLOCK, &rlim) };
    if ret != 0 {
        eprintln!("warning: failed to raise RLIMIT_MEMLOCK");
    }

    let mut ebpf = Ebpf::load(aya::include_bytes_aligned!(concat!(
        env!("OUT_DIR"),
        "/stutter"
    )))?;

    attach_tracepoint(&mut ebpf, "sched_wakeup", "sched", "sched_wakeup")?;
    attach_tracepoint(&mut ebpf, "sched_switch", "sched", "sched_switch")?;

    let target_pid_map = ebpf
        .take_map("TARGET_PIDS")
        .expect("TARGET_PIDS map not found");
    let mut target_pid_map: AyaHashMap<MapData, u32, u8> = AyaHashMap::try_from(target_pid_map)?;

    let mut active_target_pids = BTreeSet::new();
    refresh_target_pids(&config, &mut active_target_pids, &mut target_pid_map)?;

    let ring_buf = ebpf.take_map("EVENTS").expect("EVENTS map not found");
    let ring_buf = RingBuf::try_from(ring_buf)?;
    let mut async_fd = AsyncFd::new(ring_buf)?;

    let mut stats_by_pid: BTreeMap<u32, ProcessStats> = BTreeMap::new();

    let mut summary_tick = interval(Duration::from_millis(config.summary_period_ms));
    summary_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    summary_tick.tick().await;

    let mut tree_tick = interval(Duration::from_secs(1));
    tree_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tree_tick.tick().await;

    println!("Attached to sched_wakeup + sched_switch.");
    println!("Tracking manual PIDs: {:?}", config.target_pids);
    println!("Tracking tree roots: {:?}", config.tree_pids);
    println!("Summary period: {}ms", config.summary_period_ms);
    println!(
        "Spike threshold: {}",
        format_latency(config.spike_threshold_ns)
    );
    println!("Verbose event logging: {}", config.verbose);
    println!("Press Ctrl-C to stop.");

    loop {
        tokio::select! {
            _ = tree_tick.tick(), if !config.tree_pids.is_empty() => {
                if let Err(e) = refresh_target_pids(&config, &mut active_target_pids, &mut target_pid_map) {
                    warn!("tree_refresh_failed err={e}");
                }
            }
            _ = signal::ctrl_c() => {
                print_interval_summaries(&mut stats_by_pid);
                print_session_summaries(&mut stats_by_pid);
                break;
            }

            _ = summary_tick.tick() => {
                print_interval_summaries(&mut stats_by_pid);
            }

            ready = async_fd.readable_mut() => {
                let mut guard = ready?;

                let rb = guard.get_inner_mut();

                while let Some(item) = rb.next() {
                    let event = unsafe {
                        (item.as_ptr() as *const SchedulerEvent).read_unaligned()
                    };

                    match event.kind {
                        EVENT_RUNNABLE_LATENCY => {
                            let comm = comm_to_string(&event.comm);

                            let process = stats_by_pid
                                .entry(event.pid)
                                .or_insert_with(|| ProcessStats::new(comm.clone()));

                            if process.comm != comm {
                                process.comm = comm.clone();
                            }

                            process.record(&event, config.spike_threshold_ns);

                            if config.verbose {
                                print_event(&event, &comm, "sample");
                            } else if event.latency_ns >= config.spike_threshold_ns {
                                print_event(&event, &comm, "spike");
                            }
                        }
                        other => {
                            let comm = comm_to_string(&event.comm);

                            info!(
                                "unknown_event kind={} pid={} cpu={} comm={}",
                                other,
                                event.pid,
                                event.cpu,
                                comm
                            );
                        }
                    }
                }

                guard.clear_ready();
            }
        }
    }

    println!("Exiting.");
    Ok(())
}
