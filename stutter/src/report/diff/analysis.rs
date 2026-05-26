use std::{collections::BTreeMap, path::Path};

use stutter_report::diff::{ReportDiffTask, ReportTaskDelta, diff_report_task_sets};

use super::model::{RunDiffSummary, TaskDeltaSummary};
use crate::{
    process_tree::TaskClass,
    recorder::SessionFile,
    scorer, session_io,
    summary::{AggregatedTaskSummary, TaskIdentitySummary},
};

pub fn build_run_diff_summary(
    baseline_path: &Path,
    current_path: &Path,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<RunDiffSummary> {
    let baseline = session_io::load_session(baseline_path)?;
    let current = session_io::load_session(current_path)?;
    Ok(run_diff_summary_from_sessions(
        baseline_path,
        current_path,
        &baseline,
        &current,
        filter_class,
    ))
}

pub fn run_diff_summary_from_sessions(
    baseline_path: &Path,
    current_path: &Path,
    baseline: &SessionFile,
    current: &SessionFile,
    filter_class: Option<TaskClass>,
) -> RunDiffSummary {
    let baseline_tasks = aggregate_tasks(baseline, filter_class);
    let current_tasks = aggregate_tasks(current, filter_class);
    let diff = diff_report_task_sets(
        baseline_tasks
            .values()
            .map(report_task_from_aggregate)
            .collect(),
        current_tasks
            .values()
            .map(report_task_from_aggregate)
            .collect(),
    );

    RunDiffSummary {
        baseline_path: baseline_path.to_path_buf(),
        current_path: current_path.to_path_buf(),
        baseline_run_name: baseline.core.run_name.clone(),
        current_run_name: current.core.run_name.clone(),
        baseline_duration_ms: baseline.core.duration_ms,
        current_duration_ms: current.core.duration_ms,
        filter_class,
        compared_tasks: diff.compared_tasks,
        worst_p99_regression: diff.worst_p99_regression.map(task_delta_from_report),
        worst_max_regression: diff.worst_max_regression.map(task_delta_from_report),
        regressions: diff
            .regressions
            .into_iter()
            .map(task_delta_from_report)
            .collect(),
        improvements: diff
            .improvements
            .into_iter()
            .map(task_delta_from_report)
            .collect(),
        new_scored_tasks: diff
            .new_scored_tasks
            .into_iter()
            .map(aggregate_from_report_task)
            .collect(),
        new_tasks: diff
            .new_tasks
            .into_iter()
            .map(aggregate_from_report_task)
            .collect(),
        removed_tasks: diff
            .removed_tasks
            .into_iter()
            .map(aggregate_from_report_task)
            .collect(),
    }
}

fn aggregate_tasks(
    session: &SessionFile,
    filter_class: Option<TaskClass>,
) -> BTreeMap<TaskIdentitySummary, AggregatedTaskSummary> {
    let mut tasks = BTreeMap::<TaskIdentitySummary, AggregatedTaskSummary>::new();
    for task in session
        .tasks
        .iter()
        .filter(|task| task.latency.samples > 0)
        .filter(|task| filter_class.is_none_or(|class| task.class == class))
    {
        let identity = TaskIdentitySummary {
            class: task.class,
            process_comm: task.process_comm.clone(),
            comm: task.comm.clone(),
        };
        let entry = tasks
            .entry(identity.clone())
            .or_insert_with(|| AggregatedTaskSummary {
                identity,
                samples: 0,
                p99_ns: 0,
                max_ns: 0,
                over_1ms: 0,
                over_2ms: 0,
                over_5ms: 0,
                scored: scorer::class_contributes_to_score(task.class),
            });
        entry.samples = entry.samples.saturating_add(task.latency.samples);
        entry.p99_ns = entry.p99_ns.max(task.latency.p99_ns);
        entry.max_ns = entry.max_ns.max(task.latency.max_ns);
        entry.over_1ms = entry.over_1ms.saturating_add(task.latency.over_1ms);
        entry.over_2ms = entry.over_2ms.saturating_add(task.latency.over_2ms);
        entry.over_5ms = entry.over_5ms.saturating_add(task.latency.over_5ms);
    }
    tasks
}

fn report_task_from_aggregate(task: &AggregatedTaskSummary) -> ReportDiffTask<TaskIdentitySummary> {
    ReportDiffTask {
        identity: task.identity.clone(),
        samples: task.samples,
        p95_ns: 0,
        p99_ns: task.p99_ns,
        max_ns: task.max_ns,
        over_1ms: task.over_1ms,
        over_2ms: task.over_2ms,
        over_5ms: task.over_5ms,
        scored: task.scored,
    }
}

fn aggregate_from_report_task(task: ReportDiffTask<TaskIdentitySummary>) -> AggregatedTaskSummary {
    AggregatedTaskSummary {
        identity: task.identity,
        samples: task.samples,
        p99_ns: task.p99_ns,
        max_ns: task.max_ns,
        over_1ms: task.over_1ms,
        over_2ms: task.over_2ms,
        over_5ms: task.over_5ms,
        scored: task.scored,
    }
}

fn task_delta_from_report(delta: ReportTaskDelta<TaskIdentitySummary>) -> TaskDeltaSummary {
    TaskDeltaSummary {
        identity: delta.identity,
        baseline_samples: delta.baseline_samples,
        current_samples: delta.current_samples,
        baseline_p99_ns: delta.baseline_p99_ns,
        current_p99_ns: delta.current_p99_ns,
        delta_p99_ns: delta.delta_p99_ns,
        baseline_max_ns: delta.baseline_max_ns,
        current_max_ns: delta.current_max_ns,
        delta_max_ns: delta.delta_max_ns,
        baseline_over_1ms: delta.baseline_over_1ms,
        current_over_1ms: delta.current_over_1ms,
        delta_over_1ms: delta.delta_over_1ms,
    }
}
