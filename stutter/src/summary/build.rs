use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;

use super::model::*;
use crate::{
    artifacts::{ArtifactKind, artifact_path},
    process_tree::TaskClass,
    recorder::{SESSION_SCHEMA_VERSION, SessionFile, SessionTask},
    report::{build_run_diff_summary, data_quality_summary},
    session_io::{self, RunValidationReport},
};

pub fn build_compact_run_summary(
    path: &Path,
    top: usize,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<CompactRunSummary> {
    let validation = session_io::validate_run_dir_shallow(path)?;
    let session = session_io::load_session(path)?;
    Ok(compact_run_summary_from_session(
        path,
        &session,
        &validation,
        top,
        filter_class,
    ))
}

pub fn compact_run_summary_from_session(
    path: &Path,
    session: &SessionFile,
    validation: &RunValidationReport,
    top: usize,
    filter_class: Option<TaskClass>,
) -> CompactRunSummary {
    let quality = data_quality_summary(session, validation);
    let mut tasks = session
        .tasks
        .iter()
        .filter(|task| task.latency.samples > 0)
        .filter(|task| filter_class.is_none_or(|class| task.class == class))
        .map(task_summary)
        .collect::<Vec<_>>();

    let mut by_max = tasks.clone();
    by_max.sort_by_key(|task| std::cmp::Reverse(task.max_ns));

    let mut by_p99 = tasks.clone();
    by_p99.sort_by_key(|task| std::cmp::Reverse(task.p99_ns));

    let threshold_totals = tasks
        .iter()
        .fold(ThresholdTotals::default(), |mut totals, task| {
            totals.over_1ms = totals.over_1ms.saturating_add(task.over_1ms);
            totals.over_2ms = totals.over_2ms.saturating_add(task.over_2ms);
            totals.over_5ms = totals.over_5ms.saturating_add(task.over_5ms);
            totals
        });

    tasks.clear();

    CompactRunSummary {
        path: path.to_path_buf(),
        schema_version: session.core.schema_version.get(),
        expected_schema_version: SESSION_SCHEMA_VERSION.get(),
        run_name: session.core.run_name.clone(),
        duration_ms: session.core.duration_ms,
        stop_reason: session.stop_reason.clone(),
        target_counts: TargetCountsSummary {
            manual_pids: session.config.manual_pids.len(),
            tree_roots: session.config.tree_roots.len(),
            active_target_pids: session.core.active_target_pids_count,
            active_expanded_tasks: session.core.active_expanded_tasks.len(),
        },
        artifact_counts: ArtifactCountsSummary {
            reported_interval_records: session.core.interval_record_count,
            reported_spike_events: session.core.spike_events_retained_count,
            reported_irq_events: session.core.irq_event_count,
            reported_gpu_samples: session.core.gpu_sample_count,
            reported_frame_events: session.core.frame_event_count,
            reported_migration_events: session.core.migration_event_count.unwrap_or(0),
            reported_cpu_freq_samples: session.core.cpu_freq_sample_count.unwrap_or(0),
            reported_block_io_events: session.core.block_io_event_count,
            reported_scx_events: session.core.scx_event_count,
            reported_cpu_perf_samples: session.core.cpu_perf_sample_count,
        },
        data_quality_level: quality.level,
        data_quality_reasons: quality.reasons,
        worst_task_by_max_latency: by_max.first().cloned(),
        worst_p99_task: by_p99.first().cloned(),
        top_tasks_by_max_latency: by_max.into_iter().take(top).collect(),
        top_tasks_by_p99: by_p99.into_iter().take(top).collect(),
        threshold_totals,
        spike_events_retained_count: session.core.spike_events_retained_count,
        spike_events_dropped_count: session.core.spike_events_dropped_count,
        spike_events_truncated: session.core.spike_events_truncated,
        intervals_dropped: session.core.intervals_dropped,
        event_stream_write_errors: session.core.event_stream_write_errors,
        first_event_stream_write_error: session.core.first_event_stream_write_error.clone(),
    }
}

pub fn build_batch_run_summary(
    batch_dir: &Path,
    baseline_path: Option<&Path>,
    top: usize,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<BatchRunSummary> {
    let run_dirs = discover_run_dirs(batch_dir)?;
    let mut runs = Vec::new();
    let mut errors = Vec::new();

    for run_dir in run_dirs {
        match build_compact_run_summary(&run_dir, top, filter_class) {
            Ok(summary) => runs.push(summary),
            Err(err) => errors.push(BatchRunError {
                path: run_dir,
                error: format!("{err:#}"),
            }),
        }
    }

    let comparison_baseline = baseline_path
        .map(Path::to_path_buf)
        .or_else(|| runs.first().map(|summary| summary.path.clone()));
    let mut comparisons = Vec::new();
    if let Some(baseline) = &comparison_baseline {
        for run in runs.iter().filter(|run| run.path != *baseline) {
            match build_run_diff_summary(baseline, &run.path, filter_class) {
                Ok(diff) => comparisons.push(diff.limited(top)),
                Err(err) => errors.push(BatchRunError {
                    path: run.path.clone(),
                    error: format!("diff against {} failed: {err:#}", baseline.display()),
                }),
            }
        }
    }

    let best_p99 = select_run_metric(&runs, MetricKind::P99, true);
    let worst_p99 = select_run_metric(&runs, MetricKind::P99, false);
    let best_max = select_run_metric(&runs, MetricKind::Max, true);
    let worst_max = select_run_metric(&runs, MetricKind::Max, false);

    Ok(BatchRunSummary {
        batch_dir: batch_dir.to_path_buf(),
        baseline_path: comparison_baseline,
        run_count: runs.len(),
        runs,
        best_p99,
        worst_p99,
        best_max,
        worst_max,
        comparisons,
        errors,
    })
}

fn discover_run_dirs(batch_dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(batch_dir)
        .with_context(|| format!("failed to read batch directory {}", batch_dir.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", batch_dir.display()))?;
        let path = entry.path();
        if path.is_dir() && artifact_path(&path, ArtifactKind::Session).exists() {
            dirs.push(path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

fn task_summary(task: &SessionTask) -> TaskSummary {
    TaskSummary {
        task: task.task,
        active: task.active,
        class: task.class,
        process_pid: task.process_pid,
        process_comm: task.process_comm.clone(),
        comm: task.comm.clone(),
        samples: task.latency.samples,
        p95_ns: task.latency.p95_ns,
        p99_ns: task.latency.p99_ns,
        max_ns: task.latency.max_ns,
        over_1ms: task.latency.over_1ms,
        over_2ms: task.latency.over_2ms,
        over_5ms: task.latency.over_5ms,
    }
}

#[derive(Clone, Copy)]
enum MetricKind {
    P99,
    Max,
}

fn select_run_metric(
    runs: &[CompactRunSummary],
    kind: MetricKind,
    best: bool,
) -> Option<RunMetricSummary> {
    let iter = runs.iter().filter_map(|run| {
        let task = match kind {
            MetricKind::P99 => run.worst_p99_task.clone(),
            MetricKind::Max => run.worst_task_by_max_latency.clone(),
        };
        let value_ns = match (&kind, &task) {
            (MetricKind::P99, Some(task)) => task.p99_ns,
            (MetricKind::Max, Some(task)) => task.max_ns,
            _ => return None,
        };
        Some(RunMetricSummary {
            path: run.path.clone(),
            run_name: run.run_name.clone(),
            task,
            value_ns,
        })
    });

    if best {
        iter.min_by_key(|metric| metric.value_ns)
    } else {
        iter.max_by_key(|metric| metric.value_ns)
    }
}
