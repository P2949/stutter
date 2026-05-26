use serde::{Deserialize, Serialize};
use stutter_core::ids::Pid;

use super::{CpuPerfRecord, CpuSnapshot, LatencyHistogramBucket, LatencySnapshot, TaskStats};
use crate::process_tree::TaskClass;

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
        busiest_cpu: cpu.busiest_cpu.map(|cpu| cpu.as_u32()),
        busiest_cpu_samples: cpu.busiest_cpu_samples,
        worst_cpu: cpu.worst_cpu.map(|cpu| cpu.as_u32()),
        major_faults: faults_delta.0,
        minor_faults: faults_delta.1,
        worst_cpu_max_ns: cpu.worst_cpu_max_ns,
        spikiest_cpu: cpu.spikiest_cpu.map(|cpu| cpu.as_u32()),
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
