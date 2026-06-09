use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};
use stutter_common::SchedulerEvent;
use stutter_core::ids::{CpuId, Pid, Tid};

use super::{CpuPerfAccumulator, CpuStatsSet, LatencyStats, MAX_EXACT_SAMPLES};
use crate::process_tree::{TaskClass, TaskInfo};

pub type TaskStatsMap = BTreeMap<Tid, TaskStats>;

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SpikeRecord {
    pub latency_ns: u64,
    pub cpu: CpuId,
    #[serde(default)]
    pub wakeup_target_cpu: CpuId,
    pub prio: i32,
    pub wakeup_ns: u64,
    pub switch_ns: u64,
    #[serde(default)]
    pub switch_prev_pid: Pid,
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
    pub waker_counts: BTreeMap<Tid, u64>,
    pub sched_policy: Option<u32>,
    pub stat_wait_sum_ns: u128,
    pub stat_wait_count: u64,
    pub last_spike_major_faults: u64,
    pub last_spike_minor_faults: u64,
    pub from_cgroup: bool,
    histogram_truncation_warned: bool,
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
            .record(CpuId::new(event.cpu), event.latency_ns, spike_threshold_ns);

        self.session_latency.record(event.latency_ns);
        self.session_cpu
            .record(CpuId::new(event.cpu), event.latency_ns, spike_threshold_ns);

        if event.waker_tid != 0 {
            *self
                .waker_counts
                .entry(Tid::new(event.waker_tid))
                .or_insert(0) += 1;
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
                cpu: CpuId::new(event.cpu),
                wakeup_target_cpu: CpuId::new(event.wakeup_target_cpu),
                prio: event.prio,
                wakeup_ns: event.wakeup_ns,
                switch_ns: event.switch_ns,
                switch_prev_pid: Pid::new(event.switch_prev_pid),
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

fn should_replace_comm_from_task_info(current: &str, task_info: &TaskInfo) -> bool {
    (current == "?" || current.is_empty())
        || current == task_info.process_comm.as_str()
        || current == "wine64-preloader"
}
