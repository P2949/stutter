use std::{collections::BTreeMap, fmt};

use log::info;
use serde::{Deserialize, Serialize};
use stutter_common::SchedulerEvent;

use crate::process_tree::{TaskClass, TaskInfo};

pub const MAX_EXACT_SAMPLES: usize = 65_536;

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct SpikeRecord {
    pub latency_ns: u64,
    pub cpu: u32,
    pub prio: i32,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
}

pub struct TaskStats {
    pub task: u32,
    pub comm: String,
    pub class: TaskClass,
    pub process_pid: Option<u32>,
    pub process_comm: String,
    pub active: bool,
    pub first_seen_ms: u128,
    pub last_seen_ms: u128,
    pub removed_ms: Option<u128>,

    pub interval_latency: LatencyStats,
    pub interval_cpu: CpuStatsSet,

    pub session_latency: LatencyStats,
    pub session_cpu: CpuStatsSet,
    pub top_spikes: Vec<SpikeRecord>,
}

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
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct CpuStats {
    pub samples: u64,
    pub max_ns: u64,
    pub spikes: u64,
}

pub struct CpuStatsSet {
    pub by_cpu: BTreeMap<u32, CpuStats>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct LatencySnapshot {
    pub count: u64,
    pub min_ns: u64,
    pub avg_ns: u64,
    pub max_ns: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
    pub samples_truncated: u64,
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
    pub process_comm: String,
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
}

impl LatencyStats {
    pub fn new() -> Self {
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

    pub fn record(&mut self, latency_ns: u64) {
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

    pub fn snapshot_and_reset(&mut self) -> Option<LatencySnapshot> {
        let snapshot = self.snapshot()?;
        *self = Self::new();
        Some(snapshot)
    }

    #[allow(dead_code)]
    pub fn stored_samples(&self) -> u64 {
        self.count.min(MAX_EXACT_SAMPLES as u64)
    }
}

impl CpuStats {
    pub fn new() -> Self {
        Self {
            samples: 0,
            max_ns: 0,
            spikes: 0,
        }
    }

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
        Self {
            by_cpu: BTreeMap::new(),
        }
    }

    pub fn record(&mut self, cpu: u32, latency_ns: u64, spike_threshold_ns: u64) {
        self.by_cpu
            .entry(cpu)
            .or_insert_with(CpuStats::new)
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
            process_comm: String::new(),
            active: true,
            first_seen_ms: elapsed_ms,
            last_seen_ms: elapsed_ms,
            removed_ms: None,
            interval_latency: LatencyStats::new(),
            interval_cpu: CpuStatsSet::new(),
            session_latency: LatencyStats::new(),
            session_cpu: CpuStatsSet::new(),
            top_spikes: Vec::with_capacity(16),
        }
    }

    pub fn apply_task_info(&mut self, task_info: &TaskInfo) {
        self.class = task_info.class;
        self.process_pid = Some(task_info.process_pid);
        self.process_comm = task_info.process_comm.clone();

        if self.comm == "?" || self.comm.is_empty() {
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
        "{label} runnable_latency={latency_us:.3}us ({latency_ms:.6}ms) task={} cpu={} prio={} comm={} wakeup_ns={} switch_ns={}",
        event.pid, event.cpu, event.prio, comm, event.wakeup_ns, event.switch_ns,
    );
}

pub fn print_latency_line(
    label: &str,
    task: u32,
    comm: &str,
    latency: LatencySnapshot,
    cpu: &CpuSnapshot,
) {
    let percentile_note = if latency.samples_truncated > 0 {
        " percentile_scope=capped"
    } else {
        ""
    };

    info!(
        "{label} task={} comm={} samples={} stored_samples={} truncated_samples={} min={} avg={} p95={} p99={} max={} over_1ms={} over_2ms={} over_5ms={} busiest_cpu={} busiest_cpu_samples={} worst_cpu={} worst_cpu_max={} spikiest_cpu={} spikiest_cpu_spikes={}{}",
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
        percentile_note,
    );
}

pub fn print_interval_summaries(
    stats_by_task: &mut BTreeMap<u32, TaskStats>,
    elapsed_ms: u128,
    interval_records: &mut Vec<IntervalRecord>,
) {
    for (task, stats) in stats_by_task.iter_mut() {
        let Some(latency) = stats.interval_latency.snapshot_and_reset() else {
            continue;
        };

        let cpu = stats.interval_cpu.snapshot_and_reset();

        print_latency_line("summary", *task, &stats.comm, latency, &cpu);
        interval_records.push(interval_record_from_snapshot(
            elapsed_ms, *task, stats, latency, &cpu,
        ));
    }
}

pub fn print_session_summaries(stats_by_task: &mut BTreeMap<u32, TaskStats>) {
    for (task, stats) in stats_by_task.iter_mut() {
        let Some(latency) = stats.session_latency.snapshot() else {
            continue;
        };

        let cpu = stats.session_cpu.snapshot();

        print_latency_line("session", *task, &stats.comm, latency, &cpu);

        if latency.samples_truncated > 0 {
            info!(
                "session_percentile_warning task={} comm={} truncated_samples={} note=\"p95/p99 are based only on stored_samples; prefer max and over_1ms/over_2ms/over_5ms for this task\"",
                task, stats.comm, latency.samples_truncated
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

pub fn interval_record_from_snapshot(
    elapsed_ms: u128,
    task: u32,
    stats: &TaskStats,
    latency: LatencySnapshot,
    cpu: &CpuSnapshot,
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
