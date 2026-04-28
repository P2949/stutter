use std::{collections::BTreeMap, fmt};

use log::info;
use serde::{Deserialize, Serialize};
use stutter_common::SchedulerEvent;

use crate::process_tree::{TaskClass, TaskInfo};

pub const MAX_EXACT_SAMPLES: usize = 65_536;
pub const LATENCY_HISTOGRAM_BUCKETS_NS: [u64; 15] = [
    1_000, 2_000, 5_000, 10_000, 20_000, 50_000, 100_000, 200_000, 500_000, 1_000_000, 2_000_000,
    5_000_000, 10_000_000, 20_000_000, 50_000_000,
];
pub const LATENCY_HISTOGRAM_BUCKET_COUNT: usize = LATENCY_HISTOGRAM_BUCKETS_NS.len() + 1;

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

#[derive(Clone)]
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

#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct CpuStats {
    pub samples: u64,
    pub max_ns: u64,
    pub spikes: u64,
}

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
    #[serde(default)]
    pub percentile_scope: String,
    #[serde(default)]
    pub histogram: Vec<LatencyHistogramBucket>,
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
            histogram: LatencyHistogram::new(),
        }
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
                    .unwrap_or(self.max_ns),
                self.histogram
                    .percentile_upper_bound(self.count, 0.99)
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

        print_latency_line("summary", *task, &stats.comm, &latency, &cpu);
        interval_records.push(interval_record_from_snapshot(
            elapsed_ms, *task, stats, &latency, &cpu,
        ));
    }
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

pub fn interval_record_from_snapshot(
    elapsed_ms: u128,
    task: u32,
    stats: &TaskStats,
    latency: &LatencySnapshot,
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
        worst_cpu_max_ns: cpu.worst_cpu_max_ns,
        spikiest_cpu: cpu.spikiest_cpu,
        spikiest_cpu_spikes: cpu.spikiest_cpu_spikes,
        percentile_scope: latency.percentile_scope.clone(),
        histogram: latency.histogram.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histogram_records_boundaries_and_overflow() {
        let mut histogram = LatencyHistogram::new();

        histogram.record(1_000);
        histogram.record(1_001);
        histogram.record(60_000_000);

        let buckets = histogram.snapshot();

        assert_eq!(buckets[0].upper_bound_ns, Some(1_000));
        assert_eq!(buckets[0].count, 1);
        assert_eq!(buckets[1].upper_bound_ns, Some(2_000));
        assert_eq!(buckets[1].count, 1);
        assert_eq!(buckets.last().unwrap().upper_bound_ns, None);
        assert_eq!(buckets.last().unwrap().count, 1);
    }

    #[test]
    fn histogram_percentile_uses_conservative_bucket_upper_bound() {
        let mut histogram = LatencyHistogram::new();

        for _ in 0..95 {
            histogram.record(1_000);
        }
        for _ in 0..5 {
            histogram.record(1_500_000);
        }

        assert_eq!(histogram.percentile_upper_bound(100, 0.95), Some(1_000));
        assert_eq!(histogram.percentile_upper_bound(100, 0.99), Some(2_000_000));
    }

    #[test]
    fn untruncated_snapshot_uses_exact_percentiles() {
        let mut stats = LatencyStats::new();

        stats.record(1_234);
        stats.record(9_876);

        let snapshot = stats.snapshot().unwrap();

        assert_eq!(snapshot.percentile_scope, "exact");
        assert_eq!(snapshot.stored_samples, 2);
        assert_eq!(snapshot.samples_truncated, 0);
        assert_eq!(snapshot.p95_ns, 9_876);
        assert_eq!(snapshot.p99_ns, 9_876);
    }

    #[test]
    fn truncated_snapshot_uses_histogram_percentiles() {
        let mut stats = LatencyStats::new();

        for _ in 0..MAX_EXACT_SAMPLES {
            stats.record(1_000);
        }
        for _ in 0..4_000 {
            stats.record(2_000_000);
        }

        let snapshot = stats.snapshot().unwrap();

        assert_eq!(snapshot.percentile_scope, "histogram");
        assert_eq!(snapshot.samples_truncated, 4_000);
        assert_eq!(snapshot.p95_ns, 2_000_000);
        assert_eq!(snapshot.p99_ns, 2_000_000);
    }

    #[test]
    fn snapshot_and_reset_clears_histogram_state() {
        let mut stats = LatencyStats::new();

        stats.record(1_000);
        assert!(stats.snapshot_and_reset().is_some());
        stats.record(60_000_000);

        let snapshot = stats.snapshot().unwrap();

        assert_eq!(snapshot.count, 1);
        assert_eq!(snapshot.histogram[0].count, 0);
        assert_eq!(snapshot.histogram.last().unwrap().count, 1);
    }
}
