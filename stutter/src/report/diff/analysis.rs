use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

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
    let mut compared_tasks = 0;
    let mut regressions = Vec::new();
    let mut improvements = Vec::new();

    for (identity, baseline_task) in &baseline_tasks {
        if let Some(current_task) = current_tasks.get(identity) {
            compared_tasks += 1;
            let delta = task_delta(identity.clone(), baseline_task, current_task);
            if delta.delta_max_ns > 0 || delta.delta_p99_ns > 0 || delta.delta_over_1ms > 0 {
                regressions.push(delta);
            } else if delta.delta_max_ns < 0 || delta.delta_p99_ns < 0 || delta.delta_over_1ms < 0 {
                improvements.push(delta);
            }
        }
    }

    regressions.sort_by(|left, right| compare_regression(right, left));
    improvements.sort_by(compare_improvement);

    let worst_p99_regression = regressions
        .iter()
        .filter(|delta| delta.delta_p99_ns > 0)
        .max_by_key(|delta| delta.delta_p99_ns)
        .cloned();
    let worst_max_regression = regressions
        .iter()
        .filter(|delta| delta.delta_max_ns > 0)
        .max_by_key(|delta| delta.delta_max_ns)
        .cloned();

    let current_keys = current_tasks.keys().cloned().collect::<BTreeSet<_>>();
    let baseline_keys = baseline_tasks.keys().cloned().collect::<BTreeSet<_>>();

    let mut new_tasks = current_keys
        .difference(&baseline_keys)
        .filter_map(|identity| current_tasks.get(identity).cloned())
        .collect::<Vec<_>>();
    new_tasks.sort_by_key(|task| std::cmp::Reverse(task.max_ns));

    let new_scored_tasks = new_tasks
        .iter()
        .filter(|task| task.scored)
        .cloned()
        .collect::<Vec<_>>();

    let mut removed_tasks = baseline_keys
        .difference(&current_keys)
        .filter_map(|identity| baseline_tasks.get(identity).cloned())
        .collect::<Vec<_>>();
    removed_tasks.sort_by_key(|task| std::cmp::Reverse(task.max_ns));

    RunDiffSummary {
        baseline_path: baseline_path.to_path_buf(),
        current_path: current_path.to_path_buf(),
        baseline_run_name: baseline.core.run_name.clone(),
        current_run_name: current.core.run_name.clone(),
        baseline_duration_ms: baseline.core.duration_ms,
        current_duration_ms: current.core.duration_ms,
        filter_class,
        compared_tasks,
        worst_p99_regression,
        worst_max_regression,
        regressions,
        improvements,
        new_scored_tasks,
        new_tasks,
        removed_tasks,
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

fn task_delta(
    identity: TaskIdentitySummary,
    baseline: &AggregatedTaskSummary,
    current: &AggregatedTaskSummary,
) -> TaskDeltaSummary {
    TaskDeltaSummary {
        identity,
        baseline_samples: baseline.samples,
        current_samples: current.samples,
        baseline_p99_ns: baseline.p99_ns,
        current_p99_ns: current.p99_ns,
        delta_p99_ns: current.p99_ns as i64 - baseline.p99_ns as i64,
        baseline_max_ns: baseline.max_ns,
        current_max_ns: current.max_ns,
        delta_max_ns: current.max_ns as i64 - baseline.max_ns as i64,
        baseline_over_1ms: baseline.over_1ms,
        current_over_1ms: current.over_1ms,
        delta_over_1ms: current.over_1ms as i64 - baseline.over_1ms as i64,
    }
}

fn compare_regression(left: &TaskDeltaSummary, right: &TaskDeltaSummary) -> std::cmp::Ordering {
    (
        left.delta_max_ns.max(0),
        left.delta_p99_ns.max(0),
        left.delta_over_1ms.max(0),
        &left.identity,
    )
        .cmp(&(
            right.delta_max_ns.max(0),
            right.delta_p99_ns.max(0),
            right.delta_over_1ms.max(0),
            &right.identity,
        ))
}

fn compare_improvement(left: &TaskDeltaSummary, right: &TaskDeltaSummary) -> std::cmp::Ordering {
    (
        left.delta_max_ns.min(0),
        left.delta_p99_ns.min(0),
        left.delta_over_1ms.min(0),
        &left.identity,
    )
        .cmp(&(
            right.delta_max_ns.min(0),
            right.delta_p99_ns.min(0),
            right.delta_over_1ms.min(0),
            &right.identity,
        ))
}
