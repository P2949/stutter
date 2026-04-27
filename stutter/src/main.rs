mod process_tree;

use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    fmt::Write as FmtWrite,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use aya::{
    Ebpf,
    maps::{HashMap as AyaHashMap, MapData, RingBuf},
    programs::TracePoint,
};
use clap::{Args, Parser, Subcommand};
use log::{debug, info, warn};
use process_tree::{TargetDiffAction, TaskClass, TaskInfo};
use serde::{Deserialize, Serialize};
use stutter_common::{EVENT_RUNNABLE_LATENCY, SchedulerEvent};
use tokio::{
    io::unix::AsyncFd,
    signal,
    time::{MissedTickBehavior, interval, sleep},
};

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Profile scheduler runnable latency for selected tasks"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    legacy_monitor: MonitorArgs,
}

#[derive(Subcommand, Debug)]
enum Command {
    Monitor(MonitorArgs),
    Record(RecordArgs),
    InspectTree(InspectTreeArgs),
    Report(ReportArgs),
}

#[derive(Args, Debug, Clone, Default)]
struct MonitorArgs {
    #[arg(long = "pid", short = 'p', value_name = "PID")]
    target_pids: Vec<u32>,

    #[arg(long = "tree-pid", value_name = "PID")]
    tree_pids: Vec<u32>,

    #[arg(long = "summary-ms", default_value_t = 1_000)]
    summary_period_ms: u64,

    #[arg(long = "spike-us", default_value_t = 1_000)]
    spike_threshold_us: u64,

    #[arg(long, short = 'v')]
    verbose: bool,

    #[arg(long = "run-name", value_name = "NAME")]
    run_name: Option<String>,

    #[arg(long = "out-dir", alias = "out", value_name = "PATH")]
    out_dir: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
struct RecordArgs {
    #[command(flatten)]
    monitor: MonitorArgs,

    #[arg(long, value_name = "SECONDS")]
    duration: Option<u64>,
}

#[derive(Args, Debug, Clone)]
struct InspectTreeArgs {
    #[arg(long = "tree-pid", value_name = "PID")]
    tree_pid: u32,
}

#[derive(Args, Debug, Clone)]
struct ReportArgs {
    #[arg(long)]
    json: bool,

    #[arg(long, default_value_t = 10, value_name = "N")]
    top: usize,

    path: PathBuf,
}

enum AppCommand {
    Monitor(Config),
    InspectTree {
        tree_pid: u32,
    },
    Report {
        path: PathBuf,
        json: bool,
        top: usize,
    },
}

#[derive(Clone, Copy, Serialize, Deserialize)]
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
    recording: Option<RecordingConfig>,
    max_duration: Option<Duration>,
}

#[derive(Debug, Clone)]
struct RecordingConfig {
    run_name: Option<String>,
    out_dir: Option<PathBuf>,
}

struct RecordingRun {
    run_name: Option<String>,
    run_dir: PathBuf,
    started_at: SystemTime,
    started_instant: Instant,
    monotonic_start_ns: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
struct TreeEvent {
    elapsed_ms: u128,
    action: String,
    tid: u32,
    process_pid: u32,
    process_ppid: u32,
    comm: String,
    process_comm: String,
    class: TaskClass,
}

#[derive(Clone, Serialize, Deserialize)]
struct IntervalRecord {
    elapsed_ms: u128,
    task: u32,
    class: TaskClass,
    comm: String,
    samples: u64,
    stored_samples: u64,
    truncated_samples: u64,
    min_ns: u64,
    avg_ns: u64,
    p95_ns: u64,
    p99_ns: u64,
    max_ns: u64,
    over_1ms: u64,
    over_2ms: u64,
    over_5ms: u64,
    busiest_cpu: Option<u32>,
    busiest_cpu_samples: u64,
    worst_cpu: Option<u32>,
    worst_cpu_max_ns: u64,
    spikiest_cpu: Option<u32>,
    spikiest_cpu_spikes: u64,
}

struct RunArtifacts<'a> {
    active_targets: &'a BTreeMap<u32, TaskInfo>,
    known_targets: &'a BTreeMap<u32, TaskInfo>,
    interval_records: &'a [IntervalRecord],
    tree_events: &'a [TreeEvent],
}

struct ProcessStats {
    comm: String,
    class: TaskClass,
    process_pid: Option<u32>,
    process_comm: String,

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

#[derive(Clone, Copy, Serialize, Deserialize)]
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

#[derive(Serialize, Deserialize)]
struct SessionFile {
    schema_version: u32,
    run_name: Option<String>,
    started_at: RecordedTime,
    ended_at: RecordedTime,
    #[serde(default)]
    monotonic_start_ns: Option<u64>,
    #[serde(default)]
    monotonic_end_ns: Option<u64>,
    duration_ms: u128,
    stop_reason: String,
    config: RecordedConfig,
    metadata: SystemMetadata,
    target_pids_max: usize,
    active_target_pids_count: usize,
    active_expanded_tasks: Vec<u32>,
    tasks: Vec<SessionTask>,
    top_spikes: Vec<SessionSpike>,
}

#[derive(Serialize, Deserialize)]
struct MetadataFile {
    schema_version: u32,
    run_name: Option<String>,
    started_at: RecordedTime,
    ended_at: RecordedTime,
    monotonic_start_ns: Option<u64>,
    monotonic_end_ns: Option<u64>,
    duration_ms: u128,
    metadata: SystemMetadata,
    target_pids_max: usize,
    active_target_pids_count: usize,
    active_expanded_tasks: Vec<u32>,
}

#[derive(Serialize, Deserialize)]
struct RecordedConfig {
    manual_pids: Vec<u32>,
    tree_roots: Vec<u32>,
    summary_period_ms: u64,
    spike_threshold_ns: u64,
    verbose: bool,
}

#[derive(Serialize, Deserialize)]
struct RecordedTime {
    unix_seconds: u64,
    unix_nanos: u32,
    local: String,
}

#[derive(Serialize, Deserialize)]
struct SystemMetadata {
    kernel_osrelease: Option<String>,
    kernel_version: Option<String>,
    cpu_online: Option<String>,
    cpu_possible: Option<String>,
    scx_state: Option<String>,
    scx_ops: Option<String>,
    scx_enable_seq: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct SessionTask {
    task: u32,
    #[serde(default)]
    class: TaskClass,
    #[serde(default)]
    process_pid: Option<u32>,
    #[serde(default)]
    process_comm: String,
    comm: String,
    latency: RecordedLatency,
    cpu: RecordedCpuSnapshot,
    top_spikes: Vec<RecordedSpike>,
}

#[derive(Serialize, Deserialize)]
struct RecordedLatency {
    samples: u64,
    stored_samples: u64,
    truncated_samples: u64,
    min_ns: u64,
    avg_ns: u64,
    p95_ns: u64,
    p99_ns: u64,
    max_ns: u64,
    over_1ms: u64,
    over_2ms: u64,
    over_5ms: u64,
}

#[derive(Serialize, Deserialize)]
struct RecordedCpuSnapshot {
    busiest_cpu: Option<u32>,
    busiest_cpu_samples: u64,
    worst_cpu: Option<u32>,
    worst_cpu_max_ns: u64,
    spikiest_cpu: Option<u32>,
    spikiest_cpu_spikes: u64,
    per_cpu: Vec<CpuLine>,
}

#[derive(Clone, Serialize, Deserialize)]
struct RecordedSpike {
    #[serde(default)]
    class: TaskClass,
    #[serde(default)]
    process_pid: Option<u32>,
    #[serde(default)]
    process_comm: String,
    cpu: u32,
    prio: i32,
    latency_ns: u64,
    wakeup_ns: u64,
    switch_ns: u64,
}

#[derive(Clone, Serialize, Deserialize)]
struct SessionSpike {
    task: u32,
    #[serde(default)]
    class: TaskClass,
    #[serde(default)]
    process_pid: Option<u32>,
    #[serde(default)]
    process_comm: String,
    comm: String,
    cpu: u32,
    prio: i32,
    latency_ns: u64,
    wakeup_ns: u64,
    switch_ns: u64,
}

const TARGET_PIDS_MAX: usize = 1024;
const MAX_EXACT_SAMPLES: usize = 65_536;
const SESSION_SCHEMA_VERSION: u32 = 3;
const REPORT_SCHEMA_VERSION: u32 = 1;
const CLUSTER_WINDOW_NS: u64 = 2_000_000;

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
        let class = process_tree::classify_task(&comm, &comm, "");
        Self {
            comm,
            class,
            process_pid: None,
            process_comm: String::new(),
            interval_latency: LatencyStats::new(),
            interval_cpu: CpuStatsSet::new(),
            session_latency: LatencyStats::new(),
            session_cpu: CpuStatsSet::new(),
            top_spikes: Vec::with_capacity(16),
        }
    }

    fn apply_task_info(&mut self, task_info: &TaskInfo) {
        self.class = task_info.class;
        self.process_pid = Some(task_info.process_pid);
        self.process_comm = task_info.process_comm.clone();
        if self.comm == "?" || self.comm.is_empty() {
            self.comm = task_info.comm.clone();
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

fn parse_app_command() -> anyhow::Result<AppCommand> {
    parse_app_command_from(env::args_os())
}

fn parse_app_command_from<I, T>(args: I) -> anyhow::Result<AppCommand>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::try_parse_from(args)?;

    match cli.command {
        Some(Command::Monitor(args)) => Ok(AppCommand::Monitor(config_from_monitor_args(
            args, false, None,
        )?)),
        Some(Command::Record(args)) => {
            let max_duration = args.duration.map(Duration::from_secs);
            Ok(AppCommand::Monitor(config_from_monitor_args(
                args.monitor,
                true,
                max_duration,
            )?))
        }
        Some(Command::InspectTree(args)) => {
            if args.tree_pid == 0 {
                anyhow::bail!("--tree-pid must be greater than zero");
            }
            Ok(AppCommand::InspectTree {
                tree_pid: args.tree_pid,
            })
        }
        Some(Command::Report(args)) => {
            if args.top == 0 {
                anyhow::bail!("--top must be greater than zero");
            }
            Ok(AppCommand::Report {
                path: args.path,
                json: args.json,
                top: args.top,
            })
        }
        None => Ok(AppCommand::Monitor(config_from_monitor_args(
            cli.legacy_monitor,
            false,
            None,
        )?)),
    }
}

fn config_from_monitor_args(
    mut args: MonitorArgs,
    force_recording: bool,
    max_duration: Option<Duration>,
) -> anyhow::Result<Config> {
    validate_pids("--pid", &args.target_pids)?;
    validate_pids("--tree-pid", &args.tree_pids)?;

    if args.summary_period_ms == 0 {
        anyhow::bail!("--summary-ms must be greater than zero");
    }

    if args.spike_threshold_us == 0 {
        anyhow::bail!("--spike-us must be greater than zero");
    }

    if args.target_pids.is_empty() && args.tree_pids.is_empty() {
        anyhow::bail!("at least one --pid <PID> or --tree-pid <PID> is required");
    }

    args.target_pids.sort_unstable();
    args.target_pids.dedup();
    args.tree_pids.sort_unstable();
    args.tree_pids.dedup();

    if args.target_pids.len() > TARGET_PIDS_MAX {
        anyhow::bail!(
            "too many unique target PIDs: got {}, but TARGET_PIDS supports at most {}",
            args.target_pids.len(),
            TARGET_PIDS_MAX
        );
    }

    let spike_threshold_ns = args
        .spike_threshold_us
        .checked_mul(1_000)
        .ok_or_else(|| anyhow::anyhow!("--spike-us value is too large"))?;

    let recording = if force_recording || args.run_name.is_some() || args.out_dir.is_some() {
        Some(RecordingConfig {
            run_name: args
                .run_name
                .or_else(|| force_recording.then(|| "record".to_owned())),
            out_dir: args.out_dir,
        })
    } else {
        None
    };

    Ok(Config {
        target_pids: args.target_pids,
        tree_pids: args.tree_pids,
        summary_period_ms: args.summary_period_ms,
        spike_threshold_ns,
        verbose: args.verbose,
        recording,
        max_duration,
    })
}

fn validate_pids(flag: &str, pids: &[u32]) -> anyhow::Result<()> {
    if pids.contains(&0) {
        anyhow::bail!("{flag} must be greater than zero");
    }
    Ok(())
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

fn format_optional_u32(value: Option<u32>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "-".to_owned())
}

fn print_event(event: &SchedulerEvent, comm: &str, label: &str) {
    let latency_us = event.latency_ns as f64 / 1_000.0;
    let latency_ms = event.latency_ns as f64 / 1_000_000.0;

    info!(
        "{label} runnable_latency={latency_us:.3}us ({latency_ms:.6}ms) task={} cpu={} prio={} comm={} wakeup_ns={} switch_ns={}",
        event.pid, event.cpu, event.prio, comm, event.wakeup_ns, event.switch_ns,
    );
}

fn print_latency_line(
    label: &str,
    task: u32,
    comm: &str,
    latency: LatencySnapshot,
    cpu: &CpuSnapshot,
) {
    info!(
        "{label} task={} comm={} samples={} stored_samples={} truncated_samples={} min={} avg={} p95={} p99={} max={} over_1ms={} over_2ms={} over_5ms={} busiest_cpu={} busiest_cpu_samples={} worst_cpu={} worst_cpu_max={} spikiest_cpu={} spikiest_cpu_spikes={}",
        task,
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

fn print_interval_summaries(
    stats_by_task: &mut BTreeMap<u32, ProcessStats>,
    elapsed_ms: u128,
    interval_records: &mut Vec<IntervalRecord>,
) {
    for (task, process) in stats_by_task.iter_mut() {
        let Some(latency) = process.interval_latency.snapshot_and_reset() else {
            continue;
        };

        let cpu = process.interval_cpu.snapshot_and_reset();

        print_latency_line("summary", *task, &process.comm, latency, &cpu);
        interval_records.push(interval_record_from_snapshot(
            elapsed_ms, *task, process, latency, &cpu,
        ));
    }
}

fn interval_record_from_snapshot(
    elapsed_ms: u128,
    task: u32,
    process: &ProcessStats,
    latency: LatencySnapshot,
    cpu: &CpuSnapshot,
) -> IntervalRecord {
    IntervalRecord {
        elapsed_ms,
        task,
        class: process.class,
        comm: process.comm.clone(),
        samples: latency.count,
        stored_samples: latency.count.min(MAX_EXACT_SAMPLES as u64),
        truncated_samples: latency.samples_truncated,
        min_ns: latency.min_ns,
        avg_ns: latency.avg_ns,
        p95_ns: latency.p95_ns,
        p99_ns: latency.p99_ns,
        max_ns: latency.max_ns,
        over_1ms: latency.over_1ms,
        over_2ms: latency.over_2ms,
        over_5ms: latency.over_5ms,
        busiest_cpu: cpu.busiest_cpu,
        busiest_cpu_samples: cpu.busiest_cpu_samples,
        worst_cpu: cpu.worst_cpu,
        worst_cpu_max_ns: cpu.worst_cpu_max_ns,
        spikiest_cpu: cpu.spikiest_cpu,
        spikiest_cpu_spikes: cpu.spikiest_cpu_spikes,
    }
}

fn print_session_summaries(stats_by_task: &mut BTreeMap<u32, ProcessStats>) {
    for (task, process) in stats_by_task.iter_mut() {
        let Some(latency) = process.session_latency.snapshot() else {
            continue;
        };

        let cpu = process.session_cpu.snapshot();

        print_latency_line("session", *task, &process.comm, latency, &cpu);

        for cpu_line in cpu.per_cpu {
            info!(
                "session_task_cpu task={} comm={} cpu={} samples={} max={} spikes={}",
                task,
                process.comm,
                cpu_line.cpu,
                cpu_line.samples,
                format_latency(cpu_line.max_ns),
                cpu_line.spikes,
            );
        }

        for spike in &process.top_spikes {
            info!(
                "session_spike task={} comm={} cpu={} prio={} latency={} wakeup_ns={} switch_ns={}",
                task,
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
    active_targets: &mut BTreeMap<u32, TaskInfo>,
    known_targets: &mut BTreeMap<u32, TaskInfo>,
    tree_events: &mut Vec<TreeEvent>,
    target_pid_map: &mut AyaHashMap<MapData, u32, u8>,
    recording_started: Option<Instant>,
) -> anyhow::Result<()> {
    let snapshot = process_tree::target_snapshot(&config.target_pids, &config.tree_pids);
    let desired_tasks = snapshot.tasks;

    if desired_tasks.len() > TARGET_PIDS_MAX {
        anyhow::bail!(
            "too many target tasks after tree/thread expansion: got {}, but TARGET_PIDS supports at most {}",
            desired_tasks.len(),
            TARGET_PIDS_MAX
        );
    }

    for diff in process_tree::diff_tasks(active_targets, &desired_tasks) {
        match diff.action {
            TargetDiffAction::Added => {
                let tid = diff.task.tid;
                target_pid_map.insert(tid, 1, 0).map_err(|e| {
                    anyhow::anyhow!(
                        "failed to insert TID {tid} into TARGET_PIDS during tree refresh \
                         (map full? eBPF TARGET_PIDS max_entries is {TARGET_PIDS_MAX}): {e}"
                    )
                })?;
                known_targets.insert(tid, diff.task.clone());
                if let Some(started) = recording_started {
                    tree_events.push(tree_event_from_task(started, "added", &diff.task));
                }
                info!(
                    "tree_target_added tid={} process_pid={} ppid={} comm={} class={}",
                    tid,
                    diff.task.process_pid,
                    diff.task.process_ppid,
                    diff.task.comm,
                    diff.task.class
                );
            }
            TargetDiffAction::Removed => {
                let tid = diff.task.tid;
                match target_pid_map.remove(&tid) {
                    Ok(()) => info!("tree_target_removed tid={tid} class={}", diff.task.class),
                    Err(e) => warn!("tree_target_remove_failed tid={tid} err={e}"),
                }
                if let Some(started) = recording_started {
                    tree_events.push(tree_event_from_task(started, "removed", &diff.task));
                }
            }
        }
    }

    *active_targets = desired_tasks;

    debug!(
        "tree_refresh_complete active_tasks={} manual_roots={} tree_roots={} process_count={}",
        active_targets.len(),
        config.target_pids.len(),
        config.tree_pids.len(),
        snapshot.process_roots.len(),
    );

    Ok(())
}

fn tree_event_from_task(started: Instant, action: &str, task: &TaskInfo) -> TreeEvent {
    TreeEvent {
        elapsed_ms: started.elapsed().as_millis(),
        action: action.to_owned(),
        tid: task.tid,
        process_pid: task.process_pid,
        process_ppid: task.process_ppid,
        comm: task.comm.clone(),
        process_comm: task.process_comm.clone(),
        class: task.class,
    }
}

fn prepare_recording(config: &Config) -> anyhow::Result<Option<RecordingRun>> {
    let Some(recording) = &config.recording else {
        return Ok(None);
    };

    let started_at = SystemTime::now();
    let run_dir = resolve_run_dir(recording, started_at, env::var_os("HOME"));
    ensure_empty_dir(&run_dir)?;

    Ok(Some(RecordingRun {
        run_name: recording.run_name.clone(),
        run_dir,
        started_at,
        started_instant: Instant::now(),
        monotonic_start_ns: monotonic_now_ns(),
    }))
}

fn resolve_run_dir(
    recording: &RecordingConfig,
    started_at: SystemTime,
    home: Option<OsString>,
) -> PathBuf {
    if let Some(out_dir) = &recording.out_dir {
        return out_dir.clone();
    }

    let run_name = recording.run_name.as_deref().unwrap_or("run");
    default_runs_root_from_home(home).join(format!(
        "{}_{}",
        format_path_timestamp(started_at),
        slugify_run_name(run_name)
    ))
}

fn default_runs_root_from_home(home: Option<OsString>) -> PathBuf {
    home.map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".local")
        .join("state")
        .join("stutter")
        .join("runs")
}

fn ensure_empty_dir(path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        if !path.is_dir() {
            anyhow::bail!(
                "recording output path is not a directory: {}",
                path.display()
            );
        }

        if fs::read_dir(path)?.next().is_some() {
            anyhow::bail!(
                "recording output directory is not empty: {}",
                path.display()
            );
        }
    } else {
        fs::create_dir_all(path)?;
    }

    Ok(())
}

fn slugify_run_name(name: &str) -> String {
    let mut slug = String::new();
    let mut last_was_separator = false;

    for ch in name.chars() {
        let next = if ch.is_ascii_alphanumeric() {
            last_was_separator = false;
            Some(ch.to_ascii_lowercase())
        } else if ch == '-' || ch == '_' || ch.is_ascii_whitespace() {
            if last_was_separator || slug.is_empty() {
                None
            } else {
                last_was_separator = true;
                Some('_')
            }
        } else if last_was_separator || slug.is_empty() {
            None
        } else {
            last_was_separator = true;
            Some('_')
        };

        if let Some(ch) = next {
            slug.push(ch);
        }
    }

    while slug.ends_with('_') || slug.ends_with('-') {
        slug.pop();
    }

    if slug.is_empty() {
        "run".to_owned()
    } else {
        slug
    }
}

fn write_session_recording(
    recording: &RecordingRun,
    config: &Config,
    stats_by_task: &mut BTreeMap<u32, ProcessStats>,
    artifacts: RunArtifacts<'_>,
    stop_reason: &str,
) -> anyhow::Result<()> {
    let session = build_session_file(
        recording,
        config,
        stats_by_task,
        artifacts.active_targets,
        artifacts.known_targets,
        stop_reason,
    );
    let session_path = recording.run_dir.join("session.json");
    let mut session_file = fs::File::create(&session_path)?;
    serde_json::to_writer_pretty(&mut session_file, &session)?;
    session_file.write_all(b"\n")?;

    write_config_toml(&recording.run_dir.join("config.toml"), recording, config)?;
    write_metadata_json(&recording.run_dir.join("metadata.json"), &session)?;
    write_interval_csv(
        &recording.run_dir.join("interval.csv"),
        artifacts.interval_records,
    )?;
    write_session_task_cpu_csv(&recording.run_dir.join("session_task_cpu.csv"), &session)?;
    write_session_spikes_csv(
        &recording.run_dir.join("session_spikes.csv"),
        &session.top_spikes,
    )?;
    write_tree_events_csv(
        &recording.run_dir.join("tree_events.csv"),
        artifacts.tree_events,
    )?;

    println!("Wrote recording: {}", recording.run_dir.display());

    Ok(())
}

fn build_session_file(
    recording: &RecordingRun,
    config: &Config,
    stats_by_task: &mut BTreeMap<u32, ProcessStats>,
    active_targets: &BTreeMap<u32, TaskInfo>,
    known_targets: &BTreeMap<u32, TaskInfo>,
    stop_reason: &str,
) -> SessionFile {
    let ended_at = SystemTime::now();
    let mut tasks = Vec::new();
    let mut top_spikes = Vec::new();

    for (task, process) in stats_by_task.iter_mut() {
        let Some(latency) = process.session_latency.snapshot() else {
            continue;
        };

        let cpu = process.session_cpu.snapshot();
        let class = known_targets
            .get(task)
            .map(|task_info| task_info.class)
            .unwrap_or(process.class);
        let process_pid = known_targets
            .get(task)
            .map(|task_info| task_info.process_pid)
            .or(process.process_pid);
        let process_comm = known_targets
            .get(task)
            .map(|task_info| task_info.process_comm.clone())
            .unwrap_or_else(|| process.process_comm.clone());
        let task_spikes = process
            .top_spikes
            .iter()
            .map(|spike| recorded_spike_from_spike_record(spike, class, process_pid, &process_comm))
            .collect::<Vec<_>>();

        for spike in &task_spikes {
            top_spikes.push(SessionSpike {
                task: *task,
                class,
                process_pid,
                process_comm: process_comm.clone(),
                comm: process.comm.clone(),
                cpu: spike.cpu,
                prio: spike.prio,
                latency_ns: spike.latency_ns,
                wakeup_ns: spike.wakeup_ns,
                switch_ns: spike.switch_ns,
            });
        }

        tasks.push(SessionTask {
            task: *task,
            class,
            process_pid,
            process_comm,
            comm: process.comm.clone(),
            latency: recorded_latency_from_snapshot(latency),
            cpu: recorded_cpu_from_snapshot(cpu),
            top_spikes: task_spikes,
        });
    }

    top_spikes.sort_unstable_by_key(|spike| std::cmp::Reverse(spike.latency_ns));

    SessionFile {
        schema_version: SESSION_SCHEMA_VERSION,
        run_name: recording.run_name.clone(),
        started_at: recorded_time(recording.started_at),
        ended_at: recorded_time(ended_at),
        monotonic_start_ns: recording.monotonic_start_ns,
        monotonic_end_ns: monotonic_now_ns(),
        duration_ms: recording.started_instant.elapsed().as_millis(),
        stop_reason: stop_reason.to_owned(),
        config: RecordedConfig {
            manual_pids: config.target_pids.clone(),
            tree_roots: config.tree_pids.clone(),
            summary_period_ms: config.summary_period_ms,
            spike_threshold_ns: config.spike_threshold_ns,
            verbose: config.verbose,
        },
        metadata: system_metadata(),
        target_pids_max: TARGET_PIDS_MAX,
        active_target_pids_count: active_targets.len(),
        active_expanded_tasks: active_targets.keys().copied().collect(),
        tasks,
        top_spikes,
    }
}

fn recorded_spike_from_spike_record(
    spike: &SpikeRecord,
    class: TaskClass,
    process_pid: Option<u32>,
    process_comm: &str,
) -> RecordedSpike {
    RecordedSpike {
        class,
        process_pid,
        process_comm: process_comm.to_owned(),
        cpu: spike.cpu,
        prio: spike.prio,
        latency_ns: spike.latency_ns,
        wakeup_ns: spike.wakeup_ns,
        switch_ns: spike.switch_ns,
    }
}

fn recorded_latency_from_snapshot(snapshot: LatencySnapshot) -> RecordedLatency {
    RecordedLatency {
        samples: snapshot.count,
        stored_samples: snapshot.count.min(MAX_EXACT_SAMPLES as u64),
        truncated_samples: snapshot.samples_truncated,
        min_ns: snapshot.min_ns,
        avg_ns: snapshot.avg_ns,
        p95_ns: snapshot.p95_ns,
        p99_ns: snapshot.p99_ns,
        max_ns: snapshot.max_ns,
        over_1ms: snapshot.over_1ms,
        over_2ms: snapshot.over_2ms,
        over_5ms: snapshot.over_5ms,
    }
}

fn recorded_cpu_from_snapshot(snapshot: CpuSnapshot) -> RecordedCpuSnapshot {
    RecordedCpuSnapshot {
        busiest_cpu: snapshot.busiest_cpu,
        busiest_cpu_samples: snapshot.busiest_cpu_samples,
        worst_cpu: snapshot.worst_cpu,
        worst_cpu_max_ns: snapshot.worst_cpu_max_ns,
        spikiest_cpu: snapshot.spikiest_cpu,
        spikiest_cpu_spikes: snapshot.spikiest_cpu_spikes,
        per_cpu: snapshot.per_cpu,
    }
}

fn write_config_toml(path: &Path, recording: &RecordingRun, config: &Config) -> anyhow::Result<()> {
    let mut file = fs::File::create(path)?;
    writeln!(file, "schema_version = {SESSION_SCHEMA_VERSION}")?;
    if let Some(run_name) = &recording.run_name {
        writeln!(file, "run_name = \"{}\"", toml_escape(run_name))?;
    }
    writeln!(
        file,
        "run_dir = \"{}\"",
        toml_escape(&recording.run_dir.display().to_string())
    )?;
    writeln!(file, "summary_period_ms = {}", config.summary_period_ms)?;
    writeln!(file, "spike_threshold_ns = {}", config.spike_threshold_ns)?;
    writeln!(file, "verbose = {}", config.verbose)?;
    match config.max_duration {
        Some(duration) => writeln!(file, "duration_seconds = {}", duration.as_secs())?,
        None => writeln!(file, "duration_seconds = \"until_stop\"")?,
    }
    writeln!(file)?;
    writeln!(file, "manual_pids = {:?}", config.target_pids)?;
    writeln!(file, "tree_roots = {:?}", config.tree_pids)?;
    Ok(())
}

fn write_metadata_json(path: &Path, session: &SessionFile) -> anyhow::Result<()> {
    let metadata = MetadataFile {
        schema_version: session.schema_version,
        run_name: session.run_name.clone(),
        started_at: recorded_time_from_recorded(&session.started_at),
        ended_at: recorded_time_from_recorded(&session.ended_at),
        monotonic_start_ns: session.monotonic_start_ns,
        monotonic_end_ns: session.monotonic_end_ns,
        duration_ms: session.duration_ms,
        metadata: SystemMetadata {
            kernel_osrelease: session.metadata.kernel_osrelease.clone(),
            kernel_version: session.metadata.kernel_version.clone(),
            cpu_online: session.metadata.cpu_online.clone(),
            cpu_possible: session.metadata.cpu_possible.clone(),
            scx_state: session.metadata.scx_state.clone(),
            scx_ops: session.metadata.scx_ops.clone(),
            scx_enable_seq: session.metadata.scx_enable_seq.clone(),
        },
        target_pids_max: session.target_pids_max,
        active_target_pids_count: session.active_target_pids_count,
        active_expanded_tasks: session.active_expanded_tasks.clone(),
    };

    let mut file = fs::File::create(path)?;
    serde_json::to_writer_pretty(&mut file, &metadata)?;
    file.write_all(b"\n")?;
    Ok(())
}

fn recorded_time_from_recorded(time: &RecordedTime) -> RecordedTime {
    RecordedTime {
        unix_seconds: time.unix_seconds,
        unix_nanos: time.unix_nanos,
        local: time.local.clone(),
    }
}

fn write_interval_csv(path: &Path, records: &[IntervalRecord]) -> anyhow::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(
        b"elapsed_ms,task,class,comm,samples,stored_samples,truncated_samples,min_ns,avg_ns,p95_ns,p99_ns,max_ns,over_1ms,over_2ms,over_5ms,busiest_cpu,busiest_cpu_samples,worst_cpu,worst_cpu_max_ns,spikiest_cpu,spikiest_cpu_spikes\n",
    )?;

    for record in records {
        writeln!(
            file,
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            record.elapsed_ms,
            record.task,
            record.class,
            csv_escape(&record.comm),
            record.samples,
            record.stored_samples,
            record.truncated_samples,
            record.min_ns,
            record.avg_ns,
            record.p95_ns,
            record.p99_ns,
            record.max_ns,
            record.over_1ms,
            record.over_2ms,
            record.over_5ms,
            csv_optional_u32(record.busiest_cpu),
            record.busiest_cpu_samples,
            csv_optional_u32(record.worst_cpu),
            record.worst_cpu_max_ns,
            csv_optional_u32(record.spikiest_cpu),
            record.spikiest_cpu_spikes,
        )?;
    }

    Ok(())
}

fn write_session_task_cpu_csv(path: &Path, session: &SessionFile) -> anyhow::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(b"task,class,comm,cpu,samples,max_ns,spikes\n")?;

    for task in &session.tasks {
        for cpu in &task.cpu.per_cpu {
            writeln!(
                file,
                "{},{},{},{},{},{},{}",
                task.task,
                task.class,
                csv_escape(&task.comm),
                cpu.cpu,
                cpu.samples,
                cpu.max_ns,
                cpu.spikes,
            )?;
        }
    }

    Ok(())
}

fn write_session_spikes_csv(path: &Path, spikes: &[SessionSpike]) -> anyhow::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(b"task,class,comm,cpu,prio,latency_ns,wakeup_ns,switch_ns\n")?;

    for spike in spikes {
        writeln!(
            file,
            "{},{},{},{},{},{},{},{}",
            spike.task,
            spike.class,
            csv_escape(&spike.comm),
            spike.cpu,
            spike.prio,
            spike.latency_ns,
            spike.wakeup_ns,
            spike.switch_ns
        )?;
    }

    Ok(())
}

fn write_tree_events_csv(path: &Path, events: &[TreeEvent]) -> anyhow::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(b"elapsed_ms,action,tid,process_pid,process_ppid,comm,process_comm,class\n")?;

    for event in events {
        writeln!(
            file,
            "{},{},{},{},{},{},{},{}",
            event.elapsed_ms,
            csv_escape(&event.action),
            event.tid,
            event.process_pid,
            event.process_ppid,
            csv_escape(&event.comm),
            csv_escape(&event.process_comm),
            event.class,
        )?;
    }

    Ok(())
}

fn csv_escape(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

fn csv_optional_u32(value: Option<u32>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn toml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn recorded_time(time: SystemTime) -> RecordedTime {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    RecordedTime {
        unix_seconds: duration.as_secs(),
        unix_nanos: duration.subsec_nanos(),
        local: format_local_timestamp(time, true),
    }
}

fn monotonic_now_ns() -> Option<u64> {
    let mut timespec = std::mem::MaybeUninit::<libc::timespec>::uninit();
    let ret = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, timespec.as_mut_ptr()) };
    if ret != 0 {
        return None;
    }

    let timespec = unsafe { timespec.assume_init() };
    if timespec.tv_sec < 0 || timespec.tv_nsec < 0 {
        return None;
    }

    (timespec.tv_sec as u64)
        .checked_mul(1_000_000_000)?
        .checked_add(timespec.tv_nsec as u64)
}

fn format_path_timestamp(time: SystemTime) -> String {
    format_local_timestamp(time, false)
}

fn format_local_timestamp(time: SystemTime, include_nanos: bool) -> String {
    let duration = time.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs() as libc::time_t;
    let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();

    let tm = unsafe {
        if libc::localtime_r(&secs, tm.as_mut_ptr()).is_null() {
            return duration.as_secs().to_string();
        }
        tm.assume_init()
    };

    if include_nanos {
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:09}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec,
            duration.subsec_nanos()
        )
    } else {
        format!(
            "{:04}-{:02}-{:02}_{:02}-{:02}-{:02}",
            tm.tm_year + 1900,
            tm.tm_mon + 1,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        )
    }
}

fn system_metadata() -> SystemMetadata {
    SystemMetadata {
        kernel_osrelease: read_trimmed("/proc/sys/kernel/osrelease"),
        kernel_version: read_trimmed("/proc/version"),
        cpu_online: read_trimmed("/sys/devices/system/cpu/online"),
        cpu_possible: read_trimmed("/sys/devices/system/cpu/possible"),
        scx_state: read_trimmed("/sys/kernel/sched_ext/state"),
        scx_ops: read_trimmed("/sys/kernel/sched_ext/root/ops"),
        scx_enable_seq: read_trimmed("/sys/kernel/sched_ext/enable_seq"),
    }
}

fn read_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

async fn run_monitor(config: Config) -> anyhow::Result<()> {
    let recording = prepare_recording(&config)?;

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

    let mut active_targets = BTreeMap::new();
    let mut known_targets = BTreeMap::new();
    let mut tree_events = Vec::new();
    refresh_target_pids(
        &config,
        &mut active_targets,
        &mut known_targets,
        &mut tree_events,
        &mut target_pid_map,
        recording
            .as_ref()
            .map(|recording| recording.started_instant),
    )?;

    let ring_buf = ebpf.take_map("EVENTS").expect("EVENTS map not found");
    let ring_buf = RingBuf::try_from(ring_buf)?;
    let mut async_fd = AsyncFd::new(ring_buf)?;

    let mut stats_by_task: BTreeMap<u32, ProcessStats> = BTreeMap::new();
    let mut interval_records = Vec::new();

    let mut summary_tick = interval(Duration::from_millis(config.summary_period_ms));
    summary_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    summary_tick.tick().await;

    let mut tree_tick = interval(Duration::from_secs(1));
    tree_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tree_tick.tick().await;

    println!("Attached to sched_wakeup + sched_switch.");
    println!("Tracking manual PID roots: {:?}", config.target_pids);
    println!("Tracking tree roots: {:?}", config.tree_pids);
    println!("Active expanded tasks: {}", active_targets.len());
    println!("Summary period: {}ms", config.summary_period_ms);
    println!(
        "Spike threshold: {}",
        format_latency(config.spike_threshold_ns)
    );
    println!("Verbose event logging: {}", config.verbose);
    if let Some(recording) = &recording {
        println!("Recording run directory: {}", recording.run_dir.display());
    }
    println!("Press Ctrl-C to stop.");

    let duration_sleep = config.max_duration.map(sleep);
    tokio::pin!(duration_sleep);

    loop {
        tokio::select! {
            _ = tree_tick.tick(), if !config.tree_pids.is_empty() => {
                if let Err(e) = refresh_target_pids(
                    &config,
                    &mut active_targets,
                    &mut known_targets,
                    &mut tree_events,
                    &mut target_pid_map,
                    recording.as_ref().map(|recording| recording.started_instant),
                ) {
                    warn!("tree_refresh_failed err={e}");
                }
            }

            _ = signal::ctrl_c() => {
                finalize_monitor(
                    &config,
                    recording.as_ref(),
                    &mut stats_by_task,
                    RunArtifacts {
                        active_targets: &active_targets,
                        known_targets: &known_targets,
                        interval_records: &interval_records,
                        tree_events: &tree_events,
                    },
                    "ctrl_c",
                )?;
                break;
            }

            _ = async {
                if let Some(duration_sleep) = duration_sleep.as_mut().as_pin_mut() {
                    duration_sleep.await;
                } else {
                    std::future::pending::<()>().await;
                }
            }, if config.max_duration.is_some() => {
                finalize_monitor(
                    &config,
                    recording.as_ref(),
                    &mut stats_by_task,
                    RunArtifacts {
                        active_targets: &active_targets,
                        known_targets: &known_targets,
                        interval_records: &interval_records,
                        tree_events: &tree_events,
                    },
                    "duration",
                )?;
                break;
            }

            _ = summary_tick.tick() => {
                print_interval_summaries(
                    &mut stats_by_task,
                    recording
                        .as_ref()
                        .map(|recording| recording.started_instant.elapsed().as_millis())
                        .unwrap_or(0),
                    &mut interval_records,
                );
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

                            let process = stats_by_task
                                .entry(event.pid)
                                .or_insert_with(|| ProcessStats::new(comm.clone()));

                            if process.comm != comm {
                                process.comm = comm.clone();
                            }
                            if let Some(task_info) = known_targets.get(&event.pid) {
                                process.apply_task_info(task_info);
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
                                "unknown_event kind={} task={} cpu={} comm={}",
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

fn finalize_monitor(
    config: &Config,
    recording: Option<&RecordingRun>,
    stats_by_task: &mut BTreeMap<u32, ProcessStats>,
    artifacts: RunArtifacts<'_>,
    stop_reason: &str,
) -> anyhow::Result<()> {
    let mut final_interval_records = artifacts.interval_records.to_vec();
    print_interval_summaries(
        stats_by_task,
        recording
            .map(|recording| recording.started_instant.elapsed().as_millis())
            .unwrap_or(0),
        &mut final_interval_records,
    );
    print_session_summaries(stats_by_task);

    if let Some(recording) = recording {
        write_session_recording(
            recording,
            config,
            stats_by_task,
            RunArtifacts {
                active_targets: artifacts.active_targets,
                known_targets: artifacts.known_targets,
                interval_records: &final_interval_records,
                tree_events: artifacts.tree_events,
            },
            stop_reason,
        )?;
    }

    Ok(())
}

fn run_inspect_tree(tree_pid: u32) -> anyhow::Result<()> {
    print!("{}", process_tree::render_tree(tree_pid)?);
    Ok(())
}

#[derive(Serialize)]
struct ReportAnalysis {
    schema_version: u32,
    run: ReportRun,
    data_quality: ReportDataQuality,
    worst: ReportWorst,
    top_spikes: Vec<ReportSpike>,
    class_summary: Vec<ReportClassSummary>,
    cpu_summary: Vec<ReportCpuSummary>,
    compositor: ReportCompositor,
    spike_clusters: Vec<ReportSpikeCluster>,
}

#[derive(Serialize)]
struct ReportRun {
    run_name: Option<String>,
    source_schema_version: u32,
    started_at: String,
    duration_ms: u128,
    stop_reason: String,
    tasks_tracked: usize,
    active_target_pids_count: usize,
    target_pids_max: usize,
    spike_threshold_ns: u64,
    monotonic_start_ns: Option<u64>,
    monotonic_end_ns: Option<u64>,
    top_limit: usize,
}

#[derive(Serialize)]
struct ReportDataQuality {
    truncated_tasks: usize,
    total_truncated_samples: u64,
    percentiles_exact: bool,
    percentiles_trustworthy: bool,
}

#[derive(Serialize)]
struct ReportWorst {
    max_latency: Option<ReportTaskLatency>,
    most_over_1ms: Option<ReportTaskCount>,
    most_over_2ms: Option<ReportTaskCount>,
    most_over_5ms: Option<ReportTaskCount>,
    worst_cpu_latency: Option<ReportTaskCpuLatency>,
}

#[derive(Clone, Serialize)]
struct ReportTaskRef {
    task: u32,
    class: TaskClass,
    comm: String,
    process_pid: Option<u32>,
    process_comm: String,
}

#[derive(Serialize)]
struct ReportTaskLatency {
    task: ReportTaskRef,
    latency_ns: u64,
}

#[derive(Serialize)]
struct ReportTaskCount {
    task: ReportTaskRef,
    count: u64,
}

#[derive(Serialize)]
struct ReportTaskCpuLatency {
    task: ReportTaskRef,
    cpu: u32,
    latency_ns: u64,
}

#[derive(Clone, Serialize)]
struct ReportSpike {
    rank: usize,
    elapsed_ms: Option<u128>,
    task: ReportTaskRef,
    cpu: u32,
    prio: i32,
    latency_ns: u64,
    wakeup_ns: u64,
    switch_ns: u64,
}

#[derive(Serialize)]
struct ReportClassSummary {
    class: TaskClass,
    task_count: u64,
    samples: u64,
    worst_max_ns: u64,
    over_1ms: u64,
    over_2ms: u64,
    over_5ms: u64,
}

#[derive(Serialize)]
struct ReportCpuSummary {
    cpu: u32,
    samples: u64,
    spikes: u64,
    worst_max_ns: u64,
    worst_task: Option<ReportTaskRef>,
}

#[derive(Serialize)]
struct ReportCompositor {
    status: String,
    spike_threshold_ns: u64,
    worst_task: Option<ReportTaskLatency>,
}

#[derive(Clone, Serialize)]
struct ReportSpikeCluster {
    task_count: usize,
    elapsed_ms: Option<u128>,
    window_ns: u64,
    start_switch_ns: u64,
    end_switch_ns: u64,
    members: Vec<ReportSpikeClusterMember>,
}

#[derive(Clone, Serialize)]
struct ReportSpikeClusterMember {
    task: ReportTaskRef,
    latency_ns: u64,
    switch_ns: u64,
}

#[derive(Default)]
struct ClassSummaryAccumulator {
    task_count: u64,
    samples: u64,
    worst_max_ns: u64,
    over_1ms: u64,
    over_2ms: u64,
    over_5ms: u64,
}

#[derive(Default)]
struct CpuSummaryAccumulator {
    samples: u64,
    spikes: u64,
    worst_max_ns: u64,
    worst_task: Option<ReportTaskRef>,
}

fn run_report(path: &Path, json: bool, top_limit: usize) -> anyhow::Result<()> {
    let session = load_session(path)?;
    let analysis = build_report_analysis(&session, top_limit);

    if json {
        let stdout = io::stdout();
        let mut stdout = stdout.lock();
        serde_json::to_writer_pretty(&mut stdout, &analysis)?;
        stdout.write_all(b"\n")?;
    } else {
        print!("{}", render_report_text(&analysis));
    }

    Ok(())
}

fn load_session(path: &Path) -> anyhow::Result<SessionFile> {
    let session_path = if path.is_dir() {
        path.join("session.json")
    } else {
        path.to_path_buf()
    };

    let session_file = fs::File::open(&session_path)?;
    Ok(serde_json::from_reader(session_file)?)
}

fn build_report_analysis(session: &SessionFile, top_limit: usize) -> ReportAnalysis {
    let truncated_tasks = session
        .tasks
        .iter()
        .filter(|task| task.latency.truncated_samples > 0)
        .count();
    let total_truncated_samples = session
        .tasks
        .iter()
        .map(|task| task.latency.truncated_samples)
        .sum();

    let worst = ReportWorst {
        max_latency: session
            .tasks
            .iter()
            .max_by_key(|task| task.latency.max_ns)
            .map(|task| ReportTaskLatency {
                task: report_task_ref_from_task(task),
                latency_ns: task.latency.max_ns,
            }),
        most_over_1ms: session
            .tasks
            .iter()
            .max_by_key(|task| task.latency.over_1ms)
            .map(|task| ReportTaskCount {
                task: report_task_ref_from_task(task),
                count: task.latency.over_1ms,
            }),
        most_over_2ms: session
            .tasks
            .iter()
            .max_by_key(|task| task.latency.over_2ms)
            .map(|task| ReportTaskCount {
                task: report_task_ref_from_task(task),
                count: task.latency.over_2ms,
            }),
        most_over_5ms: session
            .tasks
            .iter()
            .max_by_key(|task| task.latency.over_5ms)
            .map(|task| ReportTaskCount {
                task: report_task_ref_from_task(task),
                count: task.latency.over_5ms,
            }),
        worst_cpu_latency: session
            .tasks
            .iter()
            .filter_map(|task| {
                task.cpu.worst_cpu.map(|cpu| ReportTaskCpuLatency {
                    task: report_task_ref_from_task(task),
                    cpu,
                    latency_ns: task.cpu.worst_cpu_max_ns,
                })
            })
            .max_by_key(|leader| leader.latency_ns),
    };

    let top_spikes = build_top_spikes(session, top_limit);

    ReportAnalysis {
        schema_version: REPORT_SCHEMA_VERSION,
        run: ReportRun {
            run_name: session.run_name.clone(),
            source_schema_version: session.schema_version,
            started_at: session.started_at.local.clone(),
            duration_ms: session.duration_ms,
            stop_reason: session.stop_reason.clone(),
            tasks_tracked: session.tasks.len(),
            active_target_pids_count: session.active_target_pids_count,
            target_pids_max: session.target_pids_max,
            spike_threshold_ns: session.config.spike_threshold_ns,
            monotonic_start_ns: session.monotonic_start_ns,
            monotonic_end_ns: session.monotonic_end_ns,
            top_limit,
        },
        data_quality: ReportDataQuality {
            truncated_tasks,
            total_truncated_samples,
            percentiles_exact: truncated_tasks == 0,
            percentiles_trustworthy: truncated_tasks == 0,
        },
        worst,
        class_summary: build_class_summary(session),
        cpu_summary: build_cpu_summary(session, top_limit),
        compositor: build_compositor_report(session),
        spike_clusters: build_spike_clusters(&top_spikes),
        top_spikes,
    }
}

fn build_top_spikes(session: &SessionFile, top_limit: usize) -> Vec<ReportSpike> {
    let mut spikes = session.top_spikes.clone();
    spikes.sort_unstable_by(|a, b| {
        b.latency_ns
            .cmp(&a.latency_ns)
            .then_with(|| a.switch_ns.cmp(&b.switch_ns))
            .then_with(|| a.task.cmp(&b.task))
    });

    spikes
        .into_iter()
        .take(top_limit)
        .enumerate()
        .map(|(idx, spike)| ReportSpike {
            rank: idx + 1,
            elapsed_ms: spike_elapsed_ms(session.monotonic_start_ns, spike.switch_ns),
            task: report_task_ref_from_spike(&spike),
            cpu: spike.cpu,
            prio: spike.prio,
            latency_ns: spike.latency_ns,
            wakeup_ns: spike.wakeup_ns,
            switch_ns: spike.switch_ns,
        })
        .collect()
}

fn spike_elapsed_ms(monotonic_start_ns: Option<u64>, switch_ns: u64) -> Option<u128> {
    monotonic_start_ns
        .and_then(|start_ns| switch_ns.checked_sub(start_ns))
        .map(|elapsed_ns| elapsed_ns as u128 / 1_000_000)
}

fn build_class_summary(session: &SessionFile) -> Vec<ReportClassSummary> {
    let mut summaries: BTreeMap<TaskClass, ClassSummaryAccumulator> = BTreeMap::new();

    for task in &session.tasks {
        let summary = summaries.entry(task.class).or_default();
        summary.task_count += 1;
        summary.samples += task.latency.samples;
        summary.worst_max_ns = summary.worst_max_ns.max(task.latency.max_ns);
        summary.over_1ms += task.latency.over_1ms;
        summary.over_2ms += task.latency.over_2ms;
        summary.over_5ms += task.latency.over_5ms;
    }

    summaries
        .into_iter()
        .map(|(class, summary)| ReportClassSummary {
            class,
            task_count: summary.task_count,
            samples: summary.samples,
            worst_max_ns: summary.worst_max_ns,
            over_1ms: summary.over_1ms,
            over_2ms: summary.over_2ms,
            over_5ms: summary.over_5ms,
        })
        .collect()
}

fn build_cpu_summary(session: &SessionFile, top_limit: usize) -> Vec<ReportCpuSummary> {
    let mut summaries: BTreeMap<u32, CpuSummaryAccumulator> = BTreeMap::new();

    for task in &session.tasks {
        for cpu in &task.cpu.per_cpu {
            let summary = summaries.entry(cpu.cpu).or_default();
            summary.samples += cpu.samples;
            summary.spikes += cpu.spikes;
            if cpu.max_ns > summary.worst_max_ns {
                summary.worst_max_ns = cpu.max_ns;
                summary.worst_task = Some(report_task_ref_from_task(task));
            }
        }
    }

    let mut summaries = summaries
        .into_iter()
        .map(|(cpu, summary)| ReportCpuSummary {
            cpu,
            samples: summary.samples,
            spikes: summary.spikes,
            worst_max_ns: summary.worst_max_ns,
            worst_task: summary.worst_task,
        })
        .collect::<Vec<_>>();

    summaries.sort_unstable_by(|a, b| {
        b.worst_max_ns
            .cmp(&a.worst_max_ns)
            .then_with(|| b.spikes.cmp(&a.spikes))
            .then_with(|| a.cpu.cmp(&b.cpu))
    });
    summaries.truncate(top_limit);
    summaries
}

fn build_compositor_report(session: &SessionFile) -> ReportCompositor {
    let worst_task = session
        .tasks
        .iter()
        .filter(|task| matches!(task.class, TaskClass::Compositor | TaskClass::GameScope))
        .max_by_key(|task| task.latency.max_ns)
        .map(|task| ReportTaskLatency {
            task: report_task_ref_from_task(task),
            latency_ns: task.latency.max_ns,
        });

    let status = match &worst_task {
        Some(worst) if worst.latency_ns < session.config.spike_threshold_ns => "clean",
        Some(_) => "spiky",
        None => "missing",
    };

    ReportCompositor {
        status: status.to_owned(),
        spike_threshold_ns: session.config.spike_threshold_ns,
        worst_task,
    }
}

fn build_spike_clusters(spikes: &[ReportSpike]) -> Vec<ReportSpikeCluster> {
    let mut spikes = spikes.to_vec();
    spikes.sort_unstable_by_key(|spike| spike.switch_ns);

    let mut best_cluster: Option<ReportSpikeCluster> = None;

    for (idx, spike) in spikes.iter().enumerate() {
        let mut members: BTreeMap<u32, ReportSpikeClusterMember> = BTreeMap::new();
        let mut end_switch_ns = spike.switch_ns;

        for other in spikes.iter().skip(idx) {
            if other.switch_ns.saturating_sub(spike.switch_ns) > CLUSTER_WINDOW_NS {
                break;
            }

            end_switch_ns = end_switch_ns.max(other.switch_ns);
            let member = ReportSpikeClusterMember {
                task: other.task.clone(),
                latency_ns: other.latency_ns,
                switch_ns: other.switch_ns,
            };

            members
                .entry(other.task.task)
                .and_modify(|existing| {
                    if member.latency_ns > existing.latency_ns {
                        *existing = member.clone();
                    }
                })
                .or_insert(member);
        }

        if members.len() < 2 {
            continue;
        }

        let mut members = members.into_values().collect::<Vec<_>>();
        members.sort_unstable_by(|a, b| {
            b.latency_ns
                .cmp(&a.latency_ns)
                .then_with(|| a.task.task.cmp(&b.task.task))
        });

        let candidate = ReportSpikeCluster {
            task_count: members.len(),
            elapsed_ms: spike.elapsed_ms,
            window_ns: CLUSTER_WINDOW_NS,
            start_switch_ns: spike.switch_ns,
            end_switch_ns,
            members,
        };

        if best_cluster
            .as_ref()
            .is_none_or(|current| is_better_cluster(&candidate, current))
        {
            best_cluster = Some(candidate);
        }
    }

    best_cluster.into_iter().collect()
}

fn is_better_cluster(candidate: &ReportSpikeCluster, current: &ReportSpikeCluster) -> bool {
    candidate
        .task_count
        .cmp(&current.task_count)
        .then_with(|| cluster_max_latency(candidate).cmp(&cluster_max_latency(current)))
        .then_with(|| current.start_switch_ns.cmp(&candidate.start_switch_ns))
        .is_gt()
}

fn cluster_max_latency(cluster: &ReportSpikeCluster) -> u64 {
    cluster
        .members
        .iter()
        .map(|member| member.latency_ns)
        .max()
        .unwrap_or_default()
}

fn report_task_ref_from_task(task: &SessionTask) -> ReportTaskRef {
    ReportTaskRef {
        task: task.task,
        class: task.class,
        comm: task.comm.clone(),
        process_pid: task.process_pid,
        process_comm: task.process_comm.clone(),
    }
}

fn report_task_ref_from_spike(spike: &SessionSpike) -> ReportTaskRef {
    ReportTaskRef {
        task: spike.task,
        class: spike.class,
        comm: spike.comm.clone(),
        process_pid: spike.process_pid,
        process_comm: spike.process_comm.clone(),
    }
}

fn render_report_text(analysis: &ReportAnalysis) -> String {
    let mut output = String::new();

    let _ = writeln!(
        output,
        "Run: {}",
        analysis.run.run_name.as_deref().unwrap_or("(unnamed)")
    );
    let _ = writeln!(output, "Started: {}", analysis.run.started_at);
    let _ = writeln!(output, "Duration: {}ms", analysis.run.duration_ms);
    let _ = writeln!(output, "Tasks tracked: {}", analysis.run.tasks_tracked);
    let _ = writeln!(
        output,
        "Expanded target tasks at stop: {}/{}",
        analysis.run.active_target_pids_count, analysis.run.target_pids_max
    );

    if let Some(leader) = &analysis.worst.max_latency {
        let _ = writeln!(
            output,
            "Worst max latency: {} max={}",
            format_report_task_ref(&leader.task),
            format_latency(leader.latency_ns)
        );
    }
    if let Some(leader) = &analysis.worst.most_over_1ms {
        let _ = writeln!(
            output,
            "Most over_1ms: {} count={}",
            format_report_task_ref(&leader.task),
            leader.count
        );
    }
    if let Some(leader) = &analysis.worst.most_over_2ms {
        let _ = writeln!(
            output,
            "Most over_2ms: {} count={}",
            format_report_task_ref(&leader.task),
            leader.count
        );
    }
    if let Some(leader) = &analysis.worst.most_over_5ms {
        let _ = writeln!(
            output,
            "Most over_5ms: {} count={}",
            format_report_task_ref(&leader.task),
            leader.count
        );
    }
    if let Some(leader) = &analysis.worst.worst_cpu_latency {
        let _ = writeln!(
            output,
            "Worst CPU latency: cpu={} {} max={}",
            leader.cpu,
            format_report_task_ref(&leader.task),
            format_latency(leader.latency_ns)
        );
    }

    let _ = writeln!(
        output,
        "Data quality: percentiles_trustworthy={} truncated_tasks={} total_truncated_samples={}",
        analysis.data_quality.percentiles_trustworthy,
        analysis.data_quality.truncated_tasks,
        analysis.data_quality.total_truncated_samples
    );

    if analysis.top_spikes.is_empty() {
        let _ = writeln!(output, "Top spikes: none retained");
    } else {
        let _ = writeln!(output, "Top spikes:");
        for spike in &analysis.top_spikes {
            let _ = writeln!(
                output,
                "  #{} elapsed={} {} process_pid={} cpu={} prio={} latency={} switch_ns={}",
                spike.rank,
                format_elapsed_ms(spike.elapsed_ms),
                format_report_task_ref(&spike.task),
                format_optional_u32(spike.task.process_pid),
                spike.cpu,
                spike.prio,
                format_latency(spike.latency_ns),
                spike.switch_ns
            );
        }
    }

    if analysis.cpu_summary.is_empty() {
        let _ = writeln!(output, "CPU summary: no CPU samples");
    } else {
        let _ = writeln!(output, "CPU summary:");
        for cpu in &analysis.cpu_summary {
            let worst_task = cpu
                .worst_task
                .as_ref()
                .map(format_report_task_ref)
                .unwrap_or_else(|| "task=- class=Unknown comm=-".to_owned());
            let _ = writeln!(
                output,
                "  cpu={} samples={} spikes={} worst_max={} {}",
                cpu.cpu,
                cpu.samples,
                cpu.spikes,
                format_latency(cpu.worst_max_ns),
                worst_task
            );
        }
    }

    if !analysis.class_summary.is_empty() {
        let _ = writeln!(output, "Class summary:");
        for summary in &analysis.class_summary {
            let _ = writeln!(
                output,
                "  {} tasks={} samples={} worst_max={} over_1ms={} over_2ms={} over_5ms={}",
                summary.class,
                summary.task_count,
                summary.samples,
                format_latency(summary.worst_max_ns),
                summary.over_1ms,
                summary.over_2ms,
                summary.over_5ms,
            );
        }
    }

    match &analysis.compositor.worst_task {
        Some(worst) => {
            let _ = writeln!(
                output,
                "Compositor scheduling: {} worst_{} max={}",
                analysis.compositor.status,
                format_report_task_ref(&worst.task),
                format_latency(worst.latency_ns)
            );
        }
        None => {
            let _ = writeln!(
                output,
                "Compositor scheduling: no compositor/gamescope task recorded"
            );
        }
    }

    if analysis.spike_clusters.is_empty() {
        let _ = writeln!(
            output,
            "Spike clusters: none visible in retained top spikes"
        );
    } else {
        let _ = writeln!(output, "Spike clusters:");
        for cluster in &analysis.spike_clusters {
            let members = cluster
                .members
                .iter()
                .map(|member| {
                    format!(
                        "{}({},{},{})",
                        member.task.task,
                        member.task.class,
                        member.task.comm,
                        format_latency(member.latency_ns)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            let _ = writeln!(
                output,
                "  {} tasks within {}ms elapsed={} switch_ns={}..{} tasks={}",
                cluster.task_count,
                cluster.window_ns / 1_000_000,
                format_elapsed_ms(cluster.elapsed_ms),
                cluster.start_switch_ns,
                cluster.end_switch_ns,
                members
            );
        }
    }

    output
}

fn format_report_task_ref(task: &ReportTaskRef) -> String {
    format!("task={} class={} comm={}", task.task, task.class, task.comm)
}

fn format_elapsed_ms(elapsed_ms: Option<u128>) -> String {
    elapsed_ms
        .map(|elapsed_ms| format!("{elapsed_ms}ms"))
        .unwrap_or_else(|| "-".to_owned())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    match parse_app_command()? {
        AppCommand::Monitor(config) => run_monitor(config).await,
        AppCommand::InspectTree { tree_pid } => run_inspect_tree(tree_pid),
        AppCommand::Report { path, json, top } => run_report(&path, json, top),
    }
}

#[cfg(test)]
mod tests {
    use std::time::UNIX_EPOCH;

    use super::*;

    #[test]
    fn parses_legacy_monitor_args() {
        let command = parse_app_command_from([
            "stutter",
            "--pid",
            "10",
            "--tree-pid",
            "20",
            "--summary-ms",
            "250",
            "--spike-us",
            "1500",
            "--verbose",
        ])
        .unwrap();

        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };

        assert_eq!(config.target_pids, vec![10]);
        assert_eq!(config.tree_pids, vec![20]);
        assert_eq!(config.summary_period_ms, 250);
        assert_eq!(config.spike_threshold_ns, 1_500_000);
        assert!(config.verbose);
        assert!(config.recording.is_none());
    }

    #[test]
    fn parses_subcommand_monitor_recording_args() {
        let command = parse_app_command_from([
            "stutter",
            "monitor",
            "--pid",
            "10",
            "--run-name",
            "KCD Tree",
        ])
        .unwrap();

        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };

        assert_eq!(
            config.recording.as_ref().unwrap().run_name.as_deref(),
            Some("KCD Tree")
        );
    }

    #[test]
    fn record_forces_recording() {
        let command =
            parse_app_command_from(["stutter", "record", "--tree-pid", "44", "--duration", "300"])
                .unwrap();

        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };

        assert_eq!(config.tree_pids, vec![44]);
        assert_eq!(config.max_duration, Some(Duration::from_secs(300)));
        assert_eq!(
            config.recording.as_ref().unwrap().run_name.as_deref(),
            Some("record")
        );
    }

    #[test]
    fn parses_report_json_and_top_args() {
        let command =
            parse_app_command_from(["stutter", "report", "--json", "--top", "3", "/tmp/run"])
                .unwrap();

        let AppCommand::Report { path, json, top } = command else {
            panic!("expected report command");
        };

        assert_eq!(path, PathBuf::from("/tmp/run"));
        assert!(json);
        assert_eq!(top, 3);
    }

    #[test]
    fn parses_report_default_text_args() {
        let command = parse_app_command_from(["stutter", "report", "/tmp/run"]).unwrap();

        let AppCommand::Report { path, json, top } = command else {
            panic!("expected report command");
        };

        assert_eq!(path, PathBuf::from("/tmp/run"));
        assert!(!json);
        assert_eq!(top, 10);
    }

    #[test]
    fn slugifies_run_names() {
        assert_eq!(slugify_run_name("Kingdom Come Tree"), "kingdom_come_tree");
        assert_eq!(slugify_run_name("  !!!  "), "run");
        assert_eq!(slugify_run_name("KCD/Run #1"), "kcd_run_1");
    }

    #[test]
    fn resolves_run_name_under_state_dir() {
        let recording = RecordingConfig {
            run_name: Some("KCD Tree".to_owned()),
            out_dir: None,
        };
        let path = resolve_run_dir(
            &recording,
            UNIX_EPOCH + Duration::from_secs(1),
            Some(OsString::from("/tmp/home")),
        );

        assert!(path.starts_with("/tmp/home/.local/state/stutter/runs"));
        assert!(path.to_string_lossy().ends_with("_kcd_tree"));
    }

    #[test]
    fn refuses_non_empty_recording_dir() {
        let dir = temp_test_dir("non-empty");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("existing"), b"nope").unwrap();

        let err = ensure_empty_dir(&dir).unwrap_err();
        assert!(err.to_string().contains("not empty"));

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn latency_snapshot_tracks_thresholds() {
        let mut stats = LatencyStats::new();
        stats.record(500_000);
        stats.record(1_500_000);
        stats.record(3_000_000);

        let snapshot = stats.snapshot().unwrap();

        assert_eq!(snapshot.count, 3);
        assert_eq!(snapshot.min_ns, 500_000);
        assert_eq!(snapshot.max_ns, 3_000_000);
        assert_eq!(snapshot.over_1ms, 2);
        assert_eq!(snapshot.over_2ms, 1);
        assert_eq!(snapshot.over_5ms, 0);
    }

    #[test]
    fn keeps_top_spikes_sorted_and_capped() {
        let mut process = ProcessStats::new("task".to_owned());
        let mut event = SchedulerEvent {
            kind: EVENT_RUNNABLE_LATENCY,
            pid: 1,
            cpu: 0,
            prio: 120,
            wakeup_ns: 0,
            switch_ns: 0,
            latency_ns: 0,
            comm: [0; 16],
        };

        for latency in 1..=20 {
            event.latency_ns = latency;
            process.record(&event, 1);
        }

        assert_eq!(process.top_spikes.len(), 16);
        assert_eq!(process.top_spikes[0].latency_ns, 20);
        assert_eq!(process.top_spikes[15].latency_ns, 5);
    }

    #[test]
    fn serializes_session_json_and_spike_csv() {
        let mut stats = BTreeMap::new();
        let mut process = ProcessStats::new("task,one".to_owned());
        let event = SchedulerEvent {
            kind: EVENT_RUNNABLE_LATENCY,
            pid: 7,
            cpu: 3,
            prio: 120,
            wakeup_ns: 10,
            switch_ns: 20,
            latency_ns: 2_000_000,
            comm: [0; 16],
        };
        process.record(&event, 1_000_000);
        stats.insert(7, process);

        let recording = RecordingRun {
            run_name: Some("test".to_owned()),
            run_dir: PathBuf::from("/tmp/unused"),
            started_at: UNIX_EPOCH,
            started_instant: Instant::now(),
            monotonic_start_ns: Some(1_000_000_000),
        };
        let config = Config {
            target_pids: vec![7],
            tree_pids: vec![],
            summary_period_ms: 1_000,
            spike_threshold_ns: 1_000_000,
            verbose: false,
            recording: None,
            max_duration: None,
        };

        let session = build_session_file(
            &recording,
            &config,
            &mut stats,
            &BTreeMap::from([(
                7,
                TaskInfo {
                    tid: 7,
                    process_pid: 7,
                    process_ppid: 1,
                    comm: "task,one".to_owned(),
                    process_comm: "task,one".to_owned(),
                    class: TaskClass::Helper,
                },
            )]),
            &BTreeMap::from([(
                7,
                TaskInfo {
                    tid: 7,
                    process_pid: 7,
                    process_ppid: 1,
                    comm: "task,one".to_owned(),
                    process_comm: "task,one".to_owned(),
                    class: TaskClass::Helper,
                },
            )]),
            "test",
        );
        let json = serde_json::to_string(&session).unwrap();
        assert!(json.contains("\"schema_version\":3"));
        assert!(json.contains("\"monotonic_start_ns\":1000000000"));
        assert!(json.contains("\"task\":7"));
        assert!(json.contains("\"class\":\"Helper\""));
        let decoded: SessionFile = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.tasks[0].class, TaskClass::Helper);
        assert_eq!(decoded.top_spikes[0].class, TaskClass::Helper);
        assert_eq!(decoded.monotonic_start_ns, Some(1_000_000_000));

        let dir = temp_test_dir("csv");
        fs::create_dir_all(&dir).unwrap();

        write_config_toml(&dir.join("config.toml"), &recording, &config).unwrap();
        let config_toml = fs::read_to_string(dir.join("config.toml")).unwrap();
        assert!(config_toml.contains("schema_version = 3"));

        write_metadata_json(&dir.join("metadata.json"), &session).unwrap();
        let metadata_json = fs::read_to_string(dir.join("metadata.json")).unwrap();
        assert!(metadata_json.contains("\"schema_version\": 3"));
        assert!(metadata_json.contains("\"monotonic_start_ns\": 1000000000"));

        let interval = IntervalRecord {
            elapsed_ms: 12,
            task: 7,
            class: TaskClass::Helper,
            comm: "task,one".to_owned(),
            samples: 1,
            stored_samples: 1,
            truncated_samples: 0,
            min_ns: 2_000_000,
            avg_ns: 2_000_000,
            p95_ns: 2_000_000,
            p99_ns: 2_000_000,
            max_ns: 2_000_000,
            over_1ms: 1,
            over_2ms: 1,
            over_5ms: 0,
            busiest_cpu: Some(3),
            busiest_cpu_samples: 1,
            worst_cpu: Some(3),
            worst_cpu_max_ns: 2_000_000,
            spikiest_cpu: Some(3),
            spikiest_cpu_spikes: 1,
        };
        write_interval_csv(&dir.join("interval.csv"), &[interval]).unwrap();
        let interval_csv = fs::read_to_string(dir.join("interval.csv")).unwrap();
        assert!(interval_csv.contains("12,7,Helper,\"task,one\""));

        write_session_task_cpu_csv(&dir.join("session_task_cpu.csv"), &session).unwrap();
        let cpu_csv = fs::read_to_string(dir.join("session_task_cpu.csv")).unwrap();
        assert!(cpu_csv.contains("7,Helper,\"task,one\",3,1,2000000,1"));

        let csv_path = dir.join("session_spikes.csv");
        write_session_spikes_csv(&csv_path, &session.top_spikes).unwrap();
        let csv = fs::read_to_string(&csv_path).unwrap();
        assert!(csv.starts_with("task,class,comm,cpu,prio,latency_ns,wakeup_ns,switch_ns"));
        assert!(csv.contains("7,Helper,\"task,one\",3,120,2000000,10,20"));

        let tree_event = TreeEvent {
            elapsed_ms: 3,
            action: "added".to_owned(),
            tid: 7,
            process_pid: 7,
            process_ppid: 1,
            comm: "task,one".to_owned(),
            process_comm: "task,one".to_owned(),
            class: TaskClass::Helper,
        };
        write_tree_events_csv(&dir.join("tree_events.csv"), &[tree_event]).unwrap();
        let tree_csv = fs::read_to_string(dir.join("tree_events.csv")).unwrap();
        assert!(tree_csv.contains("3,added,7,7,1,\"task,one\",\"task,one\",Helper"));

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn loads_v1_session_without_class_fields_as_unknown() {
        let json = r#"{
            "schema_version": 1,
            "run_name": "legacy",
            "started_at": {"unix_seconds": 0, "unix_nanos": 0, "local": "1970-01-01T00:00:00.000000000"},
            "ended_at": {"unix_seconds": 1, "unix_nanos": 0, "local": "1970-01-01T00:00:01.000000000"},
            "duration_ms": 1000,
            "stop_reason": "test",
            "config": {
                "manual_pids": [7],
                "tree_roots": [],
                "summary_period_ms": 1000,
                "spike_threshold_ns": 1000000,
                "verbose": false
            },
            "metadata": {
                "kernel_osrelease": null,
                "kernel_version": null,
                "cpu_online": null,
                "cpu_possible": null,
                "scx_state": null,
                "scx_ops": null,
                "scx_enable_seq": null
            },
            "target_pids_max": 1024,
            "active_target_pids_count": 1,
            "active_expanded_tasks": [7],
            "tasks": [{
                "task": 7,
                "comm": "legacy",
                "latency": {
                    "samples": 1,
                    "stored_samples": 1,
                    "truncated_samples": 0,
                    "min_ns": 1,
                    "avg_ns": 1,
                    "p95_ns": 1,
                    "p99_ns": 1,
                    "max_ns": 1,
                    "over_1ms": 0,
                    "over_2ms": 0,
                    "over_5ms": 0
                },
                "cpu": {
                    "busiest_cpu": null,
                    "busiest_cpu_samples": 0,
                    "worst_cpu": null,
                    "worst_cpu_max_ns": 0,
                    "spikiest_cpu": null,
                    "spikiest_cpu_spikes": 0,
                    "per_cpu": []
                },
                "top_spikes": []
            }],
            "top_spikes": [{
                "task": 7,
                "comm": "legacy",
                "cpu": 0,
                "prio": 120,
                "latency_ns": 1,
                "wakeup_ns": 1,
                "switch_ns": 2
            }]
        }"#;

        let session: SessionFile = serde_json::from_str(json).unwrap();

        assert_eq!(session.schema_version, 1);
        assert_eq!(session.tasks[0].class, TaskClass::Unknown);
        assert_eq!(session.top_spikes[0].class, TaskClass::Unknown);
        assert_eq!(session.monotonic_start_ns, None);
        assert_eq!(session.monotonic_end_ns, None);
    }

    #[test]
    fn report_analysis_computes_top_spikes_cpu_quality_and_json() {
        let session = synthetic_report_session(Some(1_000_000_000));

        let analysis = build_report_analysis(&session, 2);

        assert_eq!(analysis.top_spikes.len(), 2);
        assert_eq!(analysis.top_spikes[0].task.task, 8);
        assert_eq!(analysis.top_spikes[0].elapsed_ms, Some(7));
        assert_eq!(analysis.top_spikes[1].task.task, 7);
        assert_eq!(analysis.top_spikes[1].elapsed_ms, Some(5));
        assert_eq!(analysis.cpu_summary[0].cpu, 3);
        assert_eq!(analysis.cpu_summary[0].samples, 25);
        assert_eq!(analysis.cpu_summary[0].spikes, 5);
        assert_eq!(analysis.data_quality.truncated_tasks, 1);
        assert_eq!(analysis.data_quality.total_truncated_samples, 2);
        assert!(!analysis.data_quality.percentiles_trustworthy);
        assert_eq!(analysis.spike_clusters[0].task_count, 2);

        let json = serde_json::to_value(&analysis).unwrap();
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["top_spikes"][0]["elapsed_ms"], 7);
        assert_eq!(json["cpu_summary"][0]["cpu"], 3);
    }

    #[test]
    fn report_analysis_omits_elapsed_for_old_sessions() {
        let session = synthetic_report_session(None);

        let analysis = build_report_analysis(&session, 2);

        assert_eq!(analysis.top_spikes[0].elapsed_ms, None);
        assert_eq!(analysis.spike_clusters[0].elapsed_ms, None);
    }

    #[test]
    fn report_text_contains_triage_sections() {
        let session = synthetic_report_session(Some(1_000_000_000));
        let analysis = build_report_analysis(&session, 2);

        let rendered = render_report_text(&analysis);

        assert!(rendered.contains("Data quality: percentiles_trustworthy=false"));
        assert!(rendered.contains("Top spikes:"));
        assert!(rendered.contains("#1 elapsed=7ms task=8 class=WineServer comm=wineserver"));
        assert!(rendered.contains("CPU summary:"));
        assert!(rendered.contains("Spike clusters:"));
    }

    fn synthetic_report_session(monotonic_start_ns: Option<u64>) -> SessionFile {
        SessionFile {
            schema_version: SESSION_SCHEMA_VERSION,
            run_name: Some("synthetic".to_owned()),
            started_at: recorded_time(UNIX_EPOCH),
            ended_at: recorded_time(UNIX_EPOCH + Duration::from_secs(1)),
            monotonic_start_ns,
            monotonic_end_ns: monotonic_start_ns.map(|start| start + 10_000_000),
            duration_ms: 1_000,
            stop_reason: "test".to_owned(),
            config: RecordedConfig {
                manual_pids: vec![7],
                tree_roots: vec![],
                summary_period_ms: 1_000,
                spike_threshold_ns: 1_000_000,
                verbose: false,
            },
            metadata: SystemMetadata {
                kernel_osrelease: None,
                kernel_version: None,
                cpu_online: None,
                cpu_possible: None,
                scx_state: None,
                scx_ops: None,
                scx_enable_seq: None,
            },
            target_pids_max: TARGET_PIDS_MAX,
            active_target_pids_count: 3,
            active_expanded_tasks: vec![7, 8, 9],
            tasks: vec![
                session_task(
                    7,
                    TaskClass::Game,
                    "RenderThread",
                    recorded_latency(10, 5_000_000, 3, 2, 1, 2),
                    vec![
                        CpuLine {
                            cpu: 0,
                            samples: 5,
                            max_ns: 5_000_000,
                            spikes: 2,
                        },
                        CpuLine {
                            cpu: 3,
                            samples: 5,
                            max_ns: 1_000_000,
                            spikes: 1,
                        },
                    ],
                ),
                session_task(
                    8,
                    TaskClass::WineServer,
                    "wineserver",
                    recorded_latency(20, 6_000_000, 4, 1, 1, 0),
                    vec![CpuLine {
                        cpu: 3,
                        samples: 20,
                        max_ns: 6_000_000,
                        spikes: 4,
                    }],
                ),
                session_task(
                    9,
                    TaskClass::GameScope,
                    "gamescope",
                    recorded_latency(4, 900_000, 0, 0, 0, 0),
                    vec![CpuLine {
                        cpu: 2,
                        samples: 4,
                        max_ns: 900_000,
                        spikes: 0,
                    }],
                ),
            ],
            top_spikes: vec![
                session_spike(
                    7,
                    TaskClass::Game,
                    "RenderThread",
                    0,
                    5_000_000,
                    1_005_000_000,
                ),
                session_spike(
                    8,
                    TaskClass::WineServer,
                    "wineserver",
                    3,
                    6_000_000,
                    1_007_000_000,
                ),
                session_spike(
                    7,
                    TaskClass::Game,
                    "RenderThread",
                    3,
                    3_000_000,
                    1_007_500_000,
                ),
            ],
        }
    }

    fn session_task(
        task: u32,
        class: TaskClass,
        comm: &str,
        latency: RecordedLatency,
        per_cpu: Vec<CpuLine>,
    ) -> SessionTask {
        let process_pid = Some(task);
        let process_comm = comm.to_owned();
        SessionTask {
            task,
            class,
            process_pid,
            process_comm: process_comm.clone(),
            comm: comm.to_owned(),
            latency,
            cpu: RecordedCpuSnapshot {
                busiest_cpu: per_cpu
                    .iter()
                    .max_by_key(|cpu| cpu.samples)
                    .map(|cpu| cpu.cpu),
                busiest_cpu_samples: per_cpu
                    .iter()
                    .map(|cpu| cpu.samples)
                    .max()
                    .unwrap_or_default(),
                worst_cpu: per_cpu
                    .iter()
                    .max_by_key(|cpu| cpu.max_ns)
                    .map(|cpu| cpu.cpu),
                worst_cpu_max_ns: per_cpu
                    .iter()
                    .map(|cpu| cpu.max_ns)
                    .max()
                    .unwrap_or_default(),
                spikiest_cpu: per_cpu
                    .iter()
                    .max_by_key(|cpu| cpu.spikes)
                    .map(|cpu| cpu.cpu),
                spikiest_cpu_spikes: per_cpu
                    .iter()
                    .map(|cpu| cpu.spikes)
                    .max()
                    .unwrap_or_default(),
                per_cpu,
            },
            top_spikes: Vec::new(),
        }
    }

    fn recorded_latency(
        samples: u64,
        max_ns: u64,
        over_1ms: u64,
        over_2ms: u64,
        over_5ms: u64,
        truncated_samples: u64,
    ) -> RecordedLatency {
        RecordedLatency {
            samples,
            stored_samples: samples.saturating_sub(truncated_samples),
            truncated_samples,
            min_ns: 1_000,
            avg_ns: max_ns / samples.max(1),
            p95_ns: max_ns,
            p99_ns: max_ns,
            max_ns,
            over_1ms,
            over_2ms,
            over_5ms,
        }
    }

    fn session_spike(
        task: u32,
        class: TaskClass,
        comm: &str,
        cpu: u32,
        latency_ns: u64,
        switch_ns: u64,
    ) -> SessionSpike {
        SessionSpike {
            task,
            class,
            process_pid: Some(task),
            process_comm: comm.to_owned(),
            comm: comm.to_owned(),
            cpu,
            prio: 120,
            latency_ns,
            wakeup_ns: switch_ns.saturating_sub(latency_ns),
            switch_ns,
        }
    }

    fn temp_test_dir(name: &str) -> PathBuf {
        let mut path = env::temp_dir();
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        path.push(format!("stutter-{name}-{unique}"));
        path
    }
}
