use std::{collections::BTreeMap, fmt};

use log::info;
use serde::{Deserialize, Serialize};
use stutter_common::SchedulerEvent;

use crate::process_tree::{TaskClass, TaskInfo};

pub const MAX_EXACT_SAMPLES: usize = 1_024;
pub const LATENCY_HISTOGRAM_BUCKETS_NS: [u64; 15] = [
    1_000, 2_000, 5_000, 10_000, 20_000, 50_000, 100_000, 200_000, 500_000, 1_000_000, 2_000_000,
    5_000_000, 10_000_000, 20_000_000, 50_000_000,
];
pub const LATENCY_HISTOGRAM_BUCKET_COUNT: usize = LATENCY_HISTOGRAM_BUCKETS_NS.len() + 1;

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct SpikeRecord {
    pub latency_ns: u64,
    pub cpu: u32,
    #[serde(default)]
    pub wakeup_target_cpu: u32,
    pub prio: i32,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
    // Diagnostic-only: counter of monitored wakeups that were still
    // pending on the CPU that actually ran the task after dequeue/migration.
    // This is NOT the kernel runqueue depth and MUST NOT be used in
    // scoring or tuning decisions.
    #[serde(alias = "target_runnable_depth")]
    pub target_pending_wakeups: u32,
    #[serde(default)]
    pub major_faults: u32,
    #[serde(default)]
    pub minor_faults: u32,
}

#[derive(Clone)]
pub struct TaskStats {
    pub task: u32,
    pub comm: String,
    pub class: TaskClass,
    pub process_pid: Option<u32>,
    pub process_comm: std::sync::Arc<str>,
    pub process_starttime_ticks: Option<u64>,
    pub task_starttime_ticks: Option<u64>,
    pub exe_dev: Option<u64>,
    pub exe_ino: Option<u64>,
    pub active: bool,
    pub first_seen_ms: u128,
    pub last_seen_ms: u128,
    pub removed_ms: Option<u128>,

    pub interval_latency: LatencyStats,
    pub interval_cpu: CpuStatsSet,

    pub session_latency: LatencyStats,
    pub session_cpu: CpuStatsSet,
    pub top_spikes: Vec<SpikeRecord>,
    pub migration_count: u64,
    pub cross_numa_migrations: u64,
    pub waker_counts: BTreeMap<u32, u64>,
    pub sched_policy: Option<u32>,
    pub stat_wait_sum_ns: u128,
    pub stat_wait_count: u64,
    pub major_faults: u64,
    pub minor_faults: u64,
    pub from_cgroup: bool,
    histogram_truncation_warned: bool,
}

#[derive(Clone, Default)]
pub struct LatencyStats {
    pub count: u64,
    pub min_ns: u64,
    pub max_ns: u64,
    pub sum_ns: u128,
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
    pub samples_ns: Vec<u64>,
    pub samples_truncated: u64,
    pub histogram: LatencyHistogram,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
pub struct CpuStats {
    pub samples: u64,
    pub max_ns: u64,
    pub spikes: u64,
}

#[derive(Clone, Default)]
pub struct CpuStatsSet {
    pub by_cpu: BTreeMap<u32, CpuStats>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LatencyHistogramBucket {
    pub upper_bound_ns: Option<u64>,
    pub count: u64,
}

#[derive(Clone)]
pub struct LatencyHistogram {
    buckets: [u64; LATENCY_HISTOGRAM_BUCKET_COUNT],
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LatencySnapshot {
    pub count: u64,
    pub stored_samples: u64,
    pub min_ns: u64,
    pub avg_ns: u64,
    pub max_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
    pub samples_truncated: u64,
    pub percentile_scope: String,
    pub histogram: Vec<LatencyHistogramBucket>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct CpuLine {
    pub cpu: u32,
    pub samples: u64,
    pub max_ns: u64,
    pub spikes: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct CpuSnapshot {
    pub busiest_cpu: Option<u32>,
    pub busiest_cpu_samples: u64,
    pub worst_cpu: Option<u32>,
    pub worst_cpu_max_ns: u64,
    pub spikiest_cpu: Option<u32>,
    pub spikiest_cpu_spikes: u64,
    pub per_cpu: Vec<CpuLine>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct IntervalRecord {
    pub elapsed_ms: u128,
    pub task: u32,
    pub active: bool,
    pub class: TaskClass,
    pub comm: String,
    pub process_pid: Option<u32>,
    pub process_comm: std::sync::Arc<str>,
    pub samples: u64,
    pub stored_samples: u64,
    pub truncated_samples: u64,
    pub min_ns: u64,
    pub avg_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
    pub busiest_cpu: Option<u32>,
    pub busiest_cpu_samples: u64,
    pub worst_cpu: Option<u32>,
    pub worst_cpu_max_ns: u64,
    pub spikiest_cpu: Option<u32>,
    pub spikiest_cpu_spikes: u64,
    #[serde(default)]
    pub major_faults: u64,
    #[serde(default)]
    pub minor_faults: u64,
    #[serde(default)]
    pub cpu_psi_some: f64,
    #[serde(default)]
    pub mem_psi_some: f64,
    #[serde(default)]
    pub mem_psi_full: f64,
    #[serde(default)]
    pub io_psi_some: f64,
    #[serde(default)]
    pub io_psi_full: f64,
    #[serde(default)]
    pub percentile_scope: String,
    #[serde(default)]
    pub histogram: Vec<LatencyHistogramBucket>,
    #[serde(default)]
    pub drop_counters: crate::ebpf_loader::DropCountersSnapshot,
}

impl LatencyHistogram {
    pub fn new() -> Self {
        Self {
            buckets: [0; LATENCY_HISTOGRAM_BUCKET_COUNT],
        }
    }

    pub fn record(&mut self, latency_ns: u64) {
        let bucket_idx = LATENCY_HISTOGRAM_BUCKETS_NS
            .iter()
            .position(|upper_bound_ns| latency_ns <= *upper_bound_ns)
            .unwrap_or(LATENCY_HISTOGRAM_BUCKET_COUNT - 1);

        self.buckets[bucket_idx] += 1;
    }

    pub fn percentile_upper_bound(&self, total_count: u64, percentile: f64) -> Option<u64> {
        if total_count == 0 {
            return Some(0);
        }

        let rank = ((total_count as f64 * percentile).ceil() as u64).max(1);
        let mut cumulative = 0;

        for (idx, count) in self.buckets.iter().copied().enumerate() {
            cumulative += count;
            if cumulative >= rank {
                return LATENCY_HISTOGRAM_BUCKETS_NS.get(idx).copied();
            }
        }

        None
    }

    pub fn snapshot(&self) -> Vec<LatencyHistogramBucket> {
        let mut buckets = Vec::with_capacity(LATENCY_HISTOGRAM_BUCKET_COUNT);

        for (idx, count) in self.buckets.iter().copied().enumerate() {
            buckets.push(LatencyHistogramBucket {
                upper_bound_ns: LATENCY_HISTOGRAM_BUCKETS_NS.get(idx).copied(),
                count,
            });
        }

        buckets
    }
}

impl Default for LatencyHistogram {
    fn default() -> Self {
        Self::new()
    }
}

impl LatencyStats {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, latency_ns: u64) {
        self.histogram.record(latency_ns);

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

    pub fn snapshot(&mut self) -> Option<LatencySnapshot> {
        if self.count == 0 {
            return None;
        }

        self.samples_ns.sort_unstable();
        let percentile_scope = if self.samples_truncated > 0 {
            "histogram"
        } else {
            "exact"
        };

        let (p95_ns, p99_ns) = if self.samples_truncated > 0 {
            (
                self.histogram
                    .percentile_upper_bound(self.count, 0.95)
                    // The final histogram bucket is an overflow bucket with no finite
                    // upper bound. If a percentile lands there, use the exact observed
                    // max as the conservative fallback.
                    .unwrap_or(self.max_ns),
                self.histogram
                    .percentile_upper_bound(self.count, 0.99)
                    // The final histogram bucket is an overflow bucket with no finite
                    // upper bound. If a percentile lands there, use the exact observed
                    // max as the conservative fallback.
                    .unwrap_or(self.max_ns),
            )
        } else {
            (
                percentile_from_sorted(&self.samples_ns, 0.95),
                percentile_from_sorted(&self.samples_ns, 0.99),
            )
        };

        Some(LatencySnapshot {
            count: self.count,
            stored_samples: self.stored_samples(),
            min_ns: self.min_ns,
            avg_ns: (self.sum_ns / self.count as u128) as u64,
            max_ns: self.max_ns,
            p95_ns,
            p99_ns,
            over_1ms: self.over_1ms,
            over_2ms: self.over_2ms,
            over_5ms: self.over_5ms,
            samples_truncated: self.samples_truncated,
            percentile_scope: percentile_scope.to_owned(),
            histogram: self.histogram.snapshot(),
        })
    }

    pub fn snapshot_and_reset(&mut self) -> Option<LatencySnapshot> {
        let snapshot = self.snapshot()?;
        *self = Self::new();
        Some(snapshot)
    }

    pub fn stored_samples(&self) -> u64 {
        self.count.min(MAX_EXACT_SAMPLES as u64)
    }
}

impl CpuStats {
    pub fn record(&mut self, latency_ns: u64, spike_threshold_ns: u64) {
        self.samples += 1;
        self.max_ns = self.max_ns.max(latency_ns);

        if latency_ns >= spike_threshold_ns {
            self.spikes += 1;
        }
    }
}

impl CpuStatsSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, cpu: u32, latency_ns: u64, spike_threshold_ns: u64) {
        self.by_cpu
            .entry(cpu)
            .or_default()
            .record(latency_ns, spike_threshold_ns);
    }

    pub fn snapshot(&self) -> CpuSnapshot {
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

    pub fn snapshot_and_reset(&mut self) -> CpuSnapshot {
        let snapshot = self.snapshot();
        self.by_cpu.clear();
        snapshot
    }
}

impl TaskStats {
    pub fn new(task: u32, comm: String, elapsed_ms: u128) -> Self {
        let class = crate::process_tree::classify_task(&comm, &comm, "");

        Self {
            task,
            comm,
            class,
            process_pid: None,
            process_comm: std::sync::Arc::from(""),
            process_starttime_ticks: None,
            task_starttime_ticks: None,
            exe_dev: None,
            exe_ino: None,
            active: true,
            first_seen_ms: elapsed_ms,
            last_seen_ms: elapsed_ms,
            removed_ms: None,
            interval_latency: LatencyStats::new(),
            interval_cpu: CpuStatsSet::new(),
            session_latency: LatencyStats::new(),
            session_cpu: CpuStatsSet::new(),
            top_spikes: Vec::with_capacity(16),
            migration_count: 0,
            cross_numa_migrations: 0,
            waker_counts: BTreeMap::new(),
            sched_policy: None,
            stat_wait_sum_ns: 0,
            stat_wait_count: 0,
            major_faults: 0,
            minor_faults: 0,
            from_cgroup: false,
            histogram_truncation_warned: false,
        }
    }

    pub fn apply_task_info(&mut self, task_info: &TaskInfo) {
        self.class = task_info.class;
        self.process_pid = Some(task_info.process_pid);
        self.process_comm = task_info.process_comm.clone();
        self.process_starttime_ticks = task_info.process_starttime_ticks;
        self.task_starttime_ticks = task_info.task_starttime_ticks;
        self.exe_dev = task_info.exe_dev;
        self.exe_ino = task_info.exe_ino;
        self.sched_policy = task_info.sched_policy;
        self.from_cgroup = task_info.from_cgroup;

        if should_replace_comm_from_task_info(&self.comm, task_info) {
            self.comm = task_info.comm.clone();
        }
    }

    pub fn record(&mut self, event: &SchedulerEvent, spike_threshold_ns: u64, elapsed_ms: u128) {
        self.last_seen_ms = elapsed_ms;

        self.interval_latency.record(event.latency_ns);
        self.interval_cpu
            .record(event.cpu, event.latency_ns, spike_threshold_ns);

        self.session_latency.record(event.latency_ns);
        self.session_cpu
            .record(event.cpu, event.latency_ns, spike_threshold_ns);

        if event.waker_tid != 0 {
            *self.waker_counts.entry(event.waker_tid).or_insert(0) += 1;
        }

        if self.session_latency.samples_truncated > 0 && !self.histogram_truncation_warned {
            self.histogram_truncation_warned = true;
            log::warn!(
                "latency_samples_truncated task={} comm={} stored_samples={} percentile_scope=histogram",
                self.task,
                self.comm,
                MAX_EXACT_SAMPLES
            );
        }

        if event.latency_ns >= spike_threshold_ns {
            self.top_spikes.push(SpikeRecord {
                latency_ns: event.latency_ns,
                cpu: event.cpu,
                wakeup_target_cpu: event.wakeup_target_cpu,
                prio: event.prio,
                wakeup_ns: event.wakeup_ns,
                switch_ns: event.switch_ns,
                target_pending_wakeups: event.target_pending_wakeups,
                major_faults: event.maj_flt.saturating_sub(self.major_faults as u32),
                minor_faults: event.min_flt.saturating_sub(self.minor_faults as u32),
            });

            self.top_spikes
                .sort_unstable_by_key(|spike| std::cmp::Reverse(spike.latency_ns));

            self.top_spikes.truncate(16);
        }

        self.major_faults = event.maj_flt as u64;
        self.minor_faults = event.min_flt as u64;
    }
}

fn should_replace_comm_from_task_info(current: &str, task_info: &TaskInfo) -> bool {
    (current == "?" || current.is_empty())
        || current == task_info.process_comm.as_ref()
        || current == "wine64-preloader"
}

fn percentile_from_sorted(samples: &[u64], percentile: f64) -> u64 {
    if samples.is_empty() {
        return 0;
    }

    let rank = ((samples.len() as f64 * percentile).ceil() as usize).saturating_sub(1);
    let idx = rank.min(samples.len() - 1);

    samples[idx]
}

pub fn comm_to_string(comm: &[u8; 16]) -> String {
    std::str::from_utf8(comm)
        .unwrap_or("?")
        .trim_end_matches('\0')
        .to_owned()
}

pub fn format_latency(ns: u64) -> String {
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

pub fn print_event(event: &SchedulerEvent, comm: &str, label: &str) {
    let latency_us = event.latency_ns as f64 / 1_000.0;
    let latency_ms = event.latency_ns as f64 / 1_000_000.0;

    info!(
        "{label} runnable_latency={latency_us:.3}us ({latency_ms:.6}ms) task={} cpu={} wakeup_target_cpu={} prio={} comm={} wakeup_ns={} switch_ns={}",
        event.pid, event.cpu, event.wakeup_target_cpu, event.prio, comm, event.wakeup_ns, event.switch_ns,
    );
}

pub fn print_latency_line(
    label: &str,
    task: u32,
    comm: &str,
    latency: &LatencySnapshot,
    cpu: &CpuSnapshot,
) {
    let percentile_note = if latency.samples_truncated > 0 {
        format!(" percentile_scope={}", latency.percentile_scope)
    } else {
        String::new()
    };

    info!(
        "{label} task={} comm={} samples={} stored_samples={} truncated_samples={} min={} avg={} p95={} p99={} max={} over_1ms={} over_2ms={} over_5ms={} busiest_cpu={} busiest_cpu_samples={} worst_cpu={} worst_cpu_max={} spikiest_cpu={} spikiest_cpu_spikes={}{}",
        task,
        comm,
        latency.count,
        latency.stored_samples,
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
        percentile_note,
    );
}

pub fn collect_interval_summaries_labeled(
    label: &str,
    stats_by_task: &mut BTreeMap<u32, TaskStats>,
    elapsed_ms: u128,
    drop_counters: &crate::ebpf_loader::DropCountersSnapshot,
    prev_faults_map: Option<&aya::maps::HashMap<aya::maps::MapData, u32, [u64; 2]>>,
    psi_snapshot: Option<&crate::psi::PsiSnapshot>,
    prev_faults_snapshot: &mut BTreeMap<u32, (u64, u64)>,
) -> Vec<IntervalRecord> {
    let mut interval_records = Vec::new();

    for (task, stats) in stats_by_task.iter_mut() {
        // Read cumulative fault counters from the eBPF map (if present),
        // compute the interval delta relative to the previous snapshot, and
        // update both the stats and the snapshot for next interval.
        let mut maj_delta = 0u64;
        let mut min_delta = 0u64;
        if let Some(map) = prev_faults_map
            && let Ok(counters) = map.get(task, 0)
        {
            let current_maj = counters[0];
            let current_min = counters[1];
            let prev = prev_faults_snapshot
                .get(task)
                .copied()
                .unwrap_or((current_maj, current_min));
            maj_delta = current_maj.saturating_sub(prev.0);
            min_delta = current_min.saturating_sub(prev.1);
            prev_faults_snapshot.insert(*task, (current_maj, current_min));

            // Preserve the TaskStats fields for use by spike delta calculations
            // elsewhere (they store the last seen cumulative counts).
            stats.major_faults = current_maj;
            stats.minor_faults = current_min;
        }

        let Some(latency) = stats.interval_latency.snapshot_and_reset() else {
            continue;
        };

        let cpu = stats.interval_cpu.snapshot_and_reset();

        print_latency_line(label, *task, &stats.comm, &latency, &cpu);
        interval_records.push(interval_record_from_snapshot(
            *task,
            stats,
            &latency,
            &cpu,
            elapsed_ms,
            drop_counters,
            psi_snapshot,
            (maj_delta, min_delta),
        ));
    }

    interval_records
}

pub fn print_session_summaries(stats_by_task: &mut BTreeMap<u32, TaskStats>) {
    for (task, stats) in stats_by_task.iter_mut() {
        let Some(latency) = stats.session_latency.snapshot() else {
            continue;
        };

        let cpu = stats.session_cpu.snapshot();

        print_latency_line("session", *task, &stats.comm, &latency, &cpu);

        if latency.samples_truncated > 0 {
            info!(
                "session_percentile_warning task={} comm={} truncated_samples={} percentile_scope={} note=\"p95/p99 are histogram estimates across the full session; prefer max and over_1ms/over_2ms/over_5ms for exact spike counts\"",
                task, stats.comm, latency.samples_truncated, latency.percentile_scope
            );
        }

        for cpu_line in cpu.per_cpu {
            info!(
                "session_task_cpu task={} comm={} cpu={} samples={} max={} spikes={}",
                task,
                stats.comm,
                cpu_line.cpu,
                cpu_line.samples,
                format_latency(cpu_line.max_ns),
                cpu_line.spikes,
            );
        }

        for spike in &stats.top_spikes {
            info!(
                "session_spike task={} comm={} cpu={} prio={} latency={} wakeup_ns={} switch_ns={}",
                task,
                stats.comm,
                spike.cpu,
                spike.prio,
                format_latency(spike.latency_ns),
                spike.wakeup_ns,
                spike.switch_ns,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn interval_record_from_snapshot(
    task: u32,
    stats: &TaskStats,
    latency: &LatencySnapshot,
    cpu: &CpuSnapshot,
    elapsed_ms: u128,
    drop_counters: &crate::ebpf_loader::DropCountersSnapshot,
    psi: Option<&crate::psi::PsiSnapshot>,
    faults_delta: (u64, u64),
) -> IntervalRecord {
    IntervalRecord {
        elapsed_ms,
        task,
        active: stats.active,
        class: stats.class,
        comm: stats.comm.clone(),
        process_pid: stats.process_pid,
        process_comm: stats.process_comm.clone(),
        samples: latency.count,
        stored_samples: latency.stored_samples,
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
        major_faults: faults_delta.0,
        minor_faults: faults_delta.1,
        worst_cpu_max_ns: cpu.worst_cpu_max_ns,
        spikiest_cpu: cpu.spikiest_cpu,
        spikiest_cpu_spikes: cpu.spikiest_cpu_spikes,
        cpu_psi_some: psi.map(|p| p.cpu_some_avg10).unwrap_or(0.0),
        mem_psi_some: psi.map(|p| p.mem_some_avg10).unwrap_or(0.0),
        mem_psi_full: psi.map(|p| p.mem_full_avg10).unwrap_or(0.0),
        io_psi_some: psi.map(|p| p.io_some_avg10).unwrap_or(0.0),
        io_psi_full: psi.map(|p| p.io_full_avg10).unwrap_or(0.0),
        percentile_scope: latency.percentile_scope.clone(),
        histogram: latency.histogram.clone(),
        drop_counters: drop_counters.clone(),
    }
}

impl fmt::Debug for TaskStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TaskStats")
            .field("task", &self.task)
            .field("comm", &self.comm)
            .field("class", &self.class)
            .field("process_pid", &self.process_pid)
            .field("active", &self.active)
            .finish()
    }
}
