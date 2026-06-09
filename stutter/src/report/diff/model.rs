use std::path::PathBuf;

use serde::Serialize;

use crate::{
    process_tree::TaskClass,
    summary::{AggregatedTaskSummary, TaskIdentitySummary},
};

#[derive(Clone, Debug, Serialize)]
pub struct TaskDeltaSummary {
    pub identity: TaskIdentitySummary,
    pub baseline_samples: u64,
    pub current_samples: u64,
    pub baseline_p99_ns: u64,
    pub current_p99_ns: u64,
    pub delta_p99_ns: i64,
    pub baseline_max_ns: u64,
    pub current_max_ns: u64,
    pub delta_max_ns: i64,
    pub baseline_over_1ms: u64,
    pub current_over_1ms: u64,
    pub delta_over_1ms: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct RunDiffSummary {
    pub baseline_path: PathBuf,
    pub current_path: PathBuf,
    pub baseline_run_name: Option<String>,
    pub current_run_name: Option<String>,
    pub baseline_duration_ms: u64,
    pub current_duration_ms: u64,
    pub filter_class: Option<TaskClass>,
    pub compared_tasks: usize,
    pub worst_p99_regression: Option<TaskDeltaSummary>,
    pub worst_max_regression: Option<TaskDeltaSummary>,
    pub regressions: Vec<TaskDeltaSummary>,
    pub improvements: Vec<TaskDeltaSummary>,
    pub new_scored_tasks: Vec<AggregatedTaskSummary>,
    pub new_tasks: Vec<AggregatedTaskSummary>,
    pub removed_tasks: Vec<AggregatedTaskSummary>,
}

impl RunDiffSummary {
    pub fn limited(&self, top: usize) -> Self {
        let mut limited = self.clone();
        limited.regressions.truncate(top);
        limited.improvements.truncate(top);
        limited.new_scored_tasks.truncate(top);
        limited.new_tasks.truncate(top);
        limited.removed_tasks.truncate(top);
        limited
    }
}
