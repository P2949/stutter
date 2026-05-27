use std::path::PathBuf;

use serde::Serialize;

use crate::{
    process_tree::TaskClass,
    report::{DataQualityLevel, RunDiffSummary},
};

#[derive(Clone, Debug, Serialize)]
pub struct CompactRunSummary {
    pub path: PathBuf,
    pub schema_version: u32,
    pub expected_schema_version: u32,
    pub run_name: Option<String>,
    pub duration_ms: u64,
    pub stop_reason: String,
    pub target_counts: TargetCountsSummary,
    pub artifact_counts: ArtifactCountsSummary,
    pub data_quality_level: DataQualityLevel,
    pub data_quality_reasons: Vec<String>,
    pub worst_task_by_max_latency: Option<TaskSummary>,
    pub worst_p99_task: Option<TaskSummary>,
    pub top_tasks_by_max_latency: Vec<TaskSummary>,
    pub top_tasks_by_p99: Vec<TaskSummary>,
    pub threshold_totals: ThresholdTotals,
    pub spike_events_retained_count: u64,
    pub spike_events_dropped_count: u64,
    pub spike_events_truncated: bool,
    pub intervals_dropped: u64,
    pub event_stream_write_errors: u64,
    pub first_event_stream_write_error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TargetCountsSummary {
    pub manual_pids: usize,
    pub tree_roots: usize,
    pub active_target_pids: u64,
    pub active_expanded_tasks: usize,
}

/// Artifact counts as reported in the session file.
/// Note: these are NOT verified against the actual artifacts on disk unless deep validation is requested.
#[derive(Clone, Debug, Serialize)]
pub struct ArtifactCountsSummary {
    pub reported_interval_records: u64,
    pub reported_spike_events: u64,
    pub reported_irq_events: u64,
    pub reported_gpu_samples: u64,
    pub reported_frame_events: u64,
    pub reported_migration_events: u64,
    pub reported_cpu_freq_samples: u64,
    pub reported_block_io_events: u64,
    pub reported_scx_events: u64,
    pub reported_cpu_perf_samples: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ThresholdTotals {
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct TaskSummary {
    pub task: u32,
    pub active: bool,
    pub class: TaskClass,
    pub process_pid: Option<u32>,
    pub process_comm: String,
    pub comm: String,
    pub samples: u64,
    pub p95_ns: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct TaskIdentitySummary {
    pub class: TaskClass,
    pub process_comm: String,
    pub comm: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct AggregatedTaskSummary {
    pub identity: TaskIdentitySummary,
    pub samples: u64,
    pub p99_ns: u64,
    pub max_ns: u64,
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
    pub scored: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct BatchRunSummary {
    pub batch_dir: PathBuf,
    pub baseline_path: Option<PathBuf>,
    pub run_count: usize,
    pub runs: Vec<CompactRunSummary>,
    pub best_p99: Option<RunMetricSummary>,
    pub worst_p99: Option<RunMetricSummary>,
    pub best_max: Option<RunMetricSummary>,
    pub worst_max: Option<RunMetricSummary>,
    pub comparisons: Vec<RunDiffSummary>,
    pub errors: Vec<BatchRunError>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RunMetricSummary {
    pub path: PathBuf,
    pub run_name: Option<String>,
    pub task: Option<TaskSummary>,
    pub value_ns: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct BatchRunError {
    pub path: PathBuf,
    pub error: String,
}
