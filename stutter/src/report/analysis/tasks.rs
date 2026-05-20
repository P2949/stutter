//! Task row selection and task-format helpers for report analysis.
//!
//! Owns latency task filtering, top task row selection, and compact task-related formatting.
//! Does not own artifact loading, spike density, pressure timelines, clustering, or rendering.

use super::*;

pub(crate) fn top_task_rows_by_max_latency(
    session: &SessionFile,
    top: usize,
    filter_class: Option<TaskClass>,
) -> Vec<TaskHtmlRow> {
    let mut tasks = filtered_latency_tasks(session, filter_class);
    tasks.sort_by_key(|task| std::cmp::Reverse(task.latency.max_ns));
    tasks.into_iter().take(top).map(task_html_row).collect()
}

pub(crate) fn top_task_rows_by_p99_latency(
    session: &SessionFile,
    top: usize,
    filter_class: Option<TaskClass>,
) -> Vec<TaskHtmlRow> {
    let mut tasks = filtered_latency_tasks(session, filter_class);
    tasks.sort_by_key(|task| {
        (
            std::cmp::Reverse(task.latency.p99_ns),
            std::cmp::Reverse(task.latency.max_ns),
        )
    });
    tasks.into_iter().take(top).map(task_html_row).collect()
}

pub(crate) fn filtered_latency_tasks(
    session: &SessionFile,
    filter_class: Option<TaskClass>,
) -> Vec<&SessionTask> {
    session
        .tasks
        .iter()
        .filter(|task| task.latency.samples > 0)
        .filter(|task| filter_class.is_none_or(|class| task.class == class))
        .collect()
}
