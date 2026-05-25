use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};
use stutter_common::SchedulerEvent;
use stutter_core::ids::{CpuId, Pid, Tid};

use crate::process_tree::{TaskClass, TaskInfo};

pub const MAX_EXACT_SAMPLES: usize = 1_024;
pub const LATENCY_HISTOGRAM_BUCKETS_NS: [u64; 15] = [
    1_000, 2_000, 5_000, 10_000, 20_000, 50_000, 100_000, 200_000, 500_000, 1_000_000, 2_000_000,
    5_000_000, 10_000_000, 20_000_000, 50_000_000,
];
pub const LATENCY_HISTOGRAM_BUCKET_COUNT: usize = LATENCY_HISTOGRAM_BUCKETS_NS.len() + 1;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SpikeRecord {
    pub latency_ns: u64,
    pub cpu: u32,
    #[serde(default)]
    pub wakeup_target_cpu: u32,
    pub prio: i32,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
    #[serde(default)]
    pub switch_prev_pid: u32,
    #[serde(default)]
    pub switch_prev_state: i64,
    #[serde(default)]
    pub switch_prev_state_label: String,
    /// Diagnostic-only count of monitored pending wakeups for this target/task.
    /// This is not CPU runqueue depth and must not be used as true CPU contention.
    #[serde(alias = "target_runnable_depth")]
    pub target_pending_wakeups: u32,
    /// Approximate per-CPU runnable depth for monitored target tasks only,
    /// reconstructed from sched wakeup/switch/migrate tracepoints.
    /// This is not literal rq->nr_running and does not include unrelated system tasks.
    #[serde(default)]
    pub observed_runnable_depth: u32,
    #[serde(default)]
    pub major_faults: u64,
    #[serde(default)]
    pub minor_faults: u64,
    #[serde(default)]
    pub scx_ops: Option<String>,
    #[serde(default)]
    pub scx_state: Option<String>,
    #[serde(default)]
    pub scx_enable_seq: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cause_tags: Vec<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_cause: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct SpikeRecordDiagnostics {
    pub scx_ops: Option<String>,
    pub scx_state: Option<String>,
    pub scx_enable_seq: Option<String>,
    pub cause_tags: Vec<String>,
    pub primary_cause: Option<String>,
}

#[derive(Clone)]
pub struct TaskStats {
    pub task: Tid,
    pub comm: String,
    pub class: TaskClass,
    pub process_pid: Option<Pid>,
    pub process_comm: String,
    pub process_starttime_ticks: Option<u64>,
    pub task_starttime_ticks: Option<u64>,
    pub exe_dev: Option<u64>,
    pub exe_ino: Option<u64>,
    pub active: bool,
    pub first_seen_ms: u64,
    pub last_seen_ms: u64,
    pub removed_ms: Option<u64>,

    pub interval_latency: LatencyStats,
    pub interval_cpu: CpuStatsSet,
    pub interval_cpu_perf: Option<CpuPerfAccumulator>,

    pub session_latency: LatencyStats,
    pub session_cpu: CpuStatsSet,
    pub session_cpu_perf: Option<CpuPerfAccumulator>,
    pub top_spikes: Vec<SpikeRecord>,
    pub migration_count: u64,
    pub cross_numa_migrations: u64,
    pub waker_counts: BTreeMap<u32, u64>,
    pub sched_policy: Option<u32>,
    pub stat_wait_sum_ns: u128,
    pub stat_wait_count: u64,
    pub last_spike_major_faults: u64,
    pub last_spike_minor_faults: u64,
    pub from_cgroup: bool,
    histogram_truncation_warned: bool,
}

#[derive(Clone, Debug, Default)]
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

#[derive(Clone, Debug, Copy, Default, Serialize, Deserialize)]
pub struct CpuStats {
    pub samples: u64,
    pub max_ns: u64,
    pub spikes: u64,
}

#[derive(Clone, Debug, Default)]
pub struct CpuStatsSet {
    pub by_cpu: BTreeMap<u32, CpuStats>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CpuPerfRecord {
    pub cycles: Option<u64>,
    pub instructions: Option<u64>,
    pub cache_references: Option<u64>,
    pub cache_misses: Option<u64>,
    pub ipc: Option<f64>,
    pub cache_miss_rate: Option<f64>,
    pub cache_mpki: Option<f64>,
    pub time_enabled_ns: Option<u64>,
    pub time_running_ns: Option<u64>,
    pub multiplexed: bool,
    pub scaled: bool,
    pub unavailable_reason: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct CpuPerfAccumulator {
    pub cycles: u128,
    pub instructions: u128,
    pub cache_references: u128,
    pub cache_misses: u128,
    pub time_enabled_ns: u128,
    pub time_running_ns: u128,
    pub samples: u64,
    pub multiplexed_samples: u64,
    pub scaled_samples: u64,
    pub unavailable_samples: u64,
    pub last_unavailable_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LatencyHistogramBucket {
    pub upper_bound_ns: Option<u64>,
    pub count: u64,
}

#[derive(Clone, Debug)]
pub struct LatencyHistogram {
    buckets: [u64; LATENCY_HISTOGRAM_BUCKET_COUNT],
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
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

#[derive(Clone, Debug, Copy, Serialize, Deserialize, Default)]
pub struct CpuLine {
    pub cpu: u32,
    pub samples: u64,
    pub max_ns: u64,
    pub spikes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CpuSnapshot {
    pub busiest_cpu: Option<u32>,
    pub busiest_cpu_samples: u64,
    pub worst_cpu: Option<u32>,
    pub worst_cpu_max_ns: u64,
    pub spikiest_cpu: Option<u32>,
    pub spikiest_cpu_spikes: u64,
    pub per_cpu: Vec<CpuLine>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct IntervalRecord {
    pub elapsed_ms: u64,
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
    pub mem_psi_delta_us: u64,
    #[serde(default)]
    pub mem_psi_spike: bool,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_perf: Option<CpuPerfRecord>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSliceSource {
    #[default]
    ProcSchedstat,
    ProcStatFallback,
}

impl RuntimeSliceSource {
    pub fn as_str(self) -> &'static str {
        match self {
            RuntimeSliceSource::ProcSchedstat => "proc_schedstat",
            RuntimeSliceSource::ProcStatFallback => "proc_stat_fallback",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct RuntimeSliceRecord {
    #[serde(default)]
    pub elapsed_ms: u64,

    #[serde(default)]
    pub task: u32,
    #[serde(default)]
    pub process_pid: Option<u32>,
    #[serde(default)]
    pub class: TaskClass,
    #[serde(default)]
    pub comm: String,
    #[serde(default)]
    pub process_comm: String,

    #[serde(default)]
    pub source: RuntimeSliceSource,

    #[serde(default)]
    pub interval_ms: u64,

    #[serde(default)]
    pub runtime_delta_ns: u64,
    #[serde(default)]
    pub runqueue_wait_delta_ns: Option<u64>,
    #[serde(default)]
    pub timeslices_delta: Option<u64>,

    #[serde(default)]
    pub user_runtime_delta_ns: Option<u64>,
    #[serde(default)]
    pub system_runtime_delta_ns: Option<u64>,

    #[serde(default)]
    pub runtime_ratio: Option<f64>,
    #[serde(default)]
    pub wait_ratio: Option<f64>,
    #[serde(default)]
    pub avg_runtime_per_slice_ns: Option<u64>,
    #[serde(default)]
    pub avg_wait_per_slice_ns: Option<u64>,

    #[serde(default)]
    pub valid: bool,
    #[serde(default)]
    pub unavailable_reason: Option<String>,
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

impl CpuPerfAccumulator {
    pub fn record(&mut self, delta: &crate::perf_counters::CpuPerfDelta) {
        self.samples = self.samples.saturating_add(1);

        if let Some(cycles) = delta.cycles {
            self.cycles = self.cycles.saturating_add(cycles as u128);
        }
        if let Some(instructions) = delta.instructions {
            self.instructions = self.instructions.saturating_add(instructions as u128);
        }
        if let Some(cache_references) = delta.cache_references {
            self.cache_references = self
                .cache_references
                .saturating_add(cache_references as u128);
        }
        if let Some(cache_misses) = delta.cache_misses {
            self.cache_misses = self.cache_misses.saturating_add(cache_misses as u128);
        }
        if let Some(time_enabled_ns) = delta.time_enabled_ns {
            self.time_enabled_ns = self.time_enabled_ns.saturating_add(time_enabled_ns as u128);
        }
        if let Some(time_running_ns) = delta.time_running_ns {
            self.time_running_ns = self.time_running_ns.saturating_add(time_running_ns as u128);
        }

        if delta.multiplexed {
            self.multiplexed_samples = self.multiplexed_samples.saturating_add(1);
        }
        if delta.scaled {
            self.scaled_samples = self.scaled_samples.saturating_add(1);
        }
        if let Some(reason) = &delta.unavailable_reason {
            self.unavailable_samples = self.unavailable_samples.saturating_add(1);
            self.last_unavailable_reason = Some(reason.clone());
        }
    }

    pub fn snapshot(&self) -> Option<CpuPerfRecord> {
        if self.samples == 0 {
            return None;
        }

        let cycles = optional_u128_to_u64(self.cycles);
        let instructions = optional_u128_to_u64(self.instructions);
        let cache_references = optional_u128_to_u64(self.cache_references);
        let cache_misses = optional_u128_to_u64(self.cache_misses);

        Some(CpuPerfRecord {
            cycles,
            instructions,
            cache_references,
            cache_misses,
            ipc: ratio_u128(self.instructions, self.cycles),
            cache_miss_rate: ratio_u128(self.cache_misses, self.cache_references),
            cache_mpki: if self.instructions > 0 {
                Some(self.cache_misses as f64 * 1000.0 / self.instructions as f64)
                    .filter(|v| v.is_finite())
            } else {
                None
            },
            time_enabled_ns: optional_u128_to_u64(self.time_enabled_ns),
            time_running_ns: optional_u128_to_u64(self.time_running_ns),
            multiplexed: self.multiplexed_samples > 0,
            scaled: self.scaled_samples > 0,
            unavailable_reason: self.last_unavailable_reason.clone(),
        })
    }

    pub fn snapshot_and_reset(&mut self) -> Option<CpuPerfRecord> {
        let snapshot = self.snapshot()?;
        *self = Self::default();
        Some(snapshot)
    }
}

impl CpuLine {
    pub fn cpu_id(&self) -> CpuId {
        CpuId::new(self.cpu)
    }
}

impl TaskStats {
    pub fn task_id(&self) -> Tid {
        self.task
    }

    pub fn process_id(&self) -> Option<Pid> {
        self.process_pid
    }

    pub fn new(task: u32, comm: String, elapsed_ms: u64) -> Self {
        let class = crate::process_tree::classify_task(&comm, &comm, "");

        Self {
            task: Tid::new(task),
            comm,
            class,
            process_pid: None,
            process_comm: String::new(),
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
            interval_cpu_perf: None,
            session_latency: LatencyStats::new(),
            session_cpu: CpuStatsSet::new(),
            session_cpu_perf: None,
            top_spikes: Vec::with_capacity(16),
            migration_count: 0,
            cross_numa_migrations: 0,
            waker_counts: BTreeMap::new(),
            sched_policy: None,
            stat_wait_sum_ns: 0,
            stat_wait_count: 0,
            last_spike_major_faults: 0,
            last_spike_minor_faults: 0,
            from_cgroup: false,
            histogram_truncation_warned: false,
        }
    }

    pub fn apply_task_info(&mut self, task_info: &TaskInfo) {
        self.process_pid = Some(task_info.process_id());
        self.process_comm = task_info.process_comm.clone();
        self.process_starttime_ticks = task_info.process_starttime_ticks;
        self.task_starttime_ticks = task_info.task_starttime_ticks;
        self.exe_dev = task_info.exe_dev;
        self.exe_ino = task_info.exe_ino;
        self.class = task_info.class;
        self.from_cgroup = task_info.from_cgroup;
        self.sched_policy = task_info.sched_policy;

        if should_replace_comm_from_task_info(&self.comm, task_info) {
            self.comm = task_info.comm.clone();
        }
    }

    pub fn recording_started(&mut self, elapsed_ms: u64) {
        if self.first_seen_ms == 0 {
            self.first_seen_ms = elapsed_ms;
        }
    }

    pub fn task_info(&self) -> TaskInfo {
        TaskInfo {
            tid: self.task_id(),
            process_pid: self.process_id().unwrap_or_else(|| Pid::new(0)),
            process_ppid: Pid::new(0), // ppid is not tracked in TaskStats
            comm: self.comm.clone(),
            process_comm: self.process_comm.clone(),
            process_starttime_ticks: self.process_starttime_ticks,
            task_starttime_ticks: self.task_starttime_ticks,
            exe_dev: self.exe_dev,
            exe_ino: self.exe_ino,
            class: self.class,
            sched_policy: self.sched_policy,
            from_cgroup: self.from_cgroup,
        }
    }

    pub fn record(
        &mut self,
        event: &SchedulerEvent,
        spike_threshold_ns: u64,
        elapsed_ms: u64,
        diagnostics: Option<SpikeRecordDiagnostics>,
    ) -> (u64, u64) {
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

        let major_faults = event.maj_flt.saturating_sub(self.last_spike_major_faults);
        let minor_faults = event.min_flt.saturating_sub(self.last_spike_minor_faults);

        if event.latency_ns >= spike_threshold_ns {
            let diagnostics = diagnostics.unwrap_or_default();

            self.top_spikes.push(SpikeRecord {
                latency_ns: event.latency_ns,
                cpu: event.cpu,
                wakeup_target_cpu: event.wakeup_target_cpu,
                prio: event.prio,
                wakeup_ns: event.wakeup_ns,
                switch_ns: event.switch_ns,
                switch_prev_pid: event.switch_prev_pid,
                switch_prev_state: event.switch_prev_state,
                switch_prev_state_label: crate::sched_state::classify_switch_prev_state(
                    event.switch_prev_state,
                )
                .to_owned(),
                target_pending_wakeups: event.target_pending_wakeups,
                observed_runnable_depth: event.observed_runnable_depth,
                major_faults,
                minor_faults,
                scx_ops: diagnostics.scx_ops,
                scx_state: diagnostics.scx_state,
                scx_enable_seq: diagnostics.scx_enable_seq,
                cause_tags: diagnostics.cause_tags,
                primary_cause: diagnostics.primary_cause,
            });

            self.top_spikes
                .sort_unstable_by_key(|spike| std::cmp::Reverse(spike.latency_ns));

            self.top_spikes.truncate(16);
        }

        self.last_spike_major_faults = event.maj_flt;
        self.last_spike_minor_faults = event.min_flt;

        (major_faults, minor_faults)
    }

    pub fn record_cpu_perf(&mut self, delta: &crate::perf_counters::CpuPerfDelta) {
        self.interval_cpu_perf
            .get_or_insert_with(CpuPerfAccumulator::default)
            .record(delta);
        self.session_cpu_perf
            .get_or_insert_with(CpuPerfAccumulator::default)
            .record(delta);
    }
}

fn optional_u128_to_u64(value: u128) -> Option<u64> {
    if value == 0 {
        None
    } else {
        Some(value.min(u64::MAX as u128) as u64)
    }
}

fn ratio_u128(numerator: u128, denominator: u128) -> Option<f64> {
    if denominator == 0 {
        return None;
    }
    Some(numerator as f64 / denominator as f64).filter(|v| v.is_finite())
}

fn should_replace_comm_from_task_info(current: &str, task_info: &TaskInfo) -> bool {
    (current == "?" || current.is_empty())
        || current == task_info.process_comm.as_str()
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

mod output;
pub use output::{collect_interval_summaries_labeled, print_event, print_session_summaries};

pub struct IntervalRecordFromSnapshotInput<'a> {
    pub task: u32,
    pub stats: &'a mut TaskStats,
    pub latency: &'a LatencySnapshot,
    pub cpu: &'a CpuSnapshot,
    pub elapsed_ms: u64,
    pub drop_counters: &'a crate::ebpf_loader::DropCountersSnapshot,
    pub psi: Option<&'a crate::psi::PsiDelta>,
    pub faults_delta: (u64, u64),
}

pub fn interval_record_from_snapshot(input: IntervalRecordFromSnapshotInput) -> IntervalRecord {
    let IntervalRecordFromSnapshotInput {
        task,
        stats,
        latency,
        cpu,
        elapsed_ms,
        drop_counters,
        psi,
        faults_delta,
    } = input;
    IntervalRecord {
        elapsed_ms,
        task,
        active: stats.active,
        class: stats.class,
        comm: stats.comm.clone(),
        process_pid: stats.process_pid.map(Pid::as_u32),
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
        cpu_psi_some: psi.map(|p| p.snapshot.cpu_some_avg10).unwrap_or(0.0),
        mem_psi_some: psi.map(|p| p.snapshot.mem_some_avg10).unwrap_or(0.0),
        mem_psi_full: psi.map(|p| p.snapshot.mem_full_avg10).unwrap_or(0.0),
        mem_psi_delta_us: psi.and_then(|p| p.mem_stall_delta_us).unwrap_or(0),
        mem_psi_spike: psi.map(|p| p.mem_stall_spike).unwrap_or(false),
        io_psi_some: psi.map(|p| p.snapshot.io_some_avg10).unwrap_or(0.0),
        io_psi_full: psi.map(|p| p.snapshot.io_full_avg10).unwrap_or(0.0),
        percentile_scope: latency.percentile_scope.clone(),
        histogram: latency.histogram.clone(),
        drop_counters: drop_counters.clone(),
        cpu_perf: stats
            .interval_cpu_perf
            .as_mut()
            .and_then(|perf| perf.snapshot_and_reset()),
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
pub fn log_drop_counters(drop_counters: &crate::ebpf_loader::DropCountersSnapshot) {
    if drop_counters.total() == 0 {
        log::debug!("ebpf_drop_counters total=0");
        return;
    }

    log::warn!(
        "ebpf_drop_counters cumulative_total={} wakeup_data_insert_failed={} ringbuf_reserve_failed={} irq_start_times_insert_failed={} block_start_insert_failed={} block_fallback_key_collisions={} block_zero_keys={} drm_fence_missing_start={}",
        drop_counters.total(),
        drop_counters.wakeup_data_insert_failed,
        drop_counters.ringbuf_reserve_failed,
        drop_counters.irq_start_times_insert_failed,
        drop_counters.block_start_insert_failed,
        drop_counters.block_fallback_key_collisions,
        drop_counters.block_zero_keys,
        drop_counters.drm_fence_missing_start,
    );
}

#[cfg(test)]
mod tests;
