use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::Serialize;

use crate::{
    metrics::format_latency,
    process_tree::TaskClass,
    recorder::{SESSION_SCHEMA_VERSION, SessionFile, SessionTask},
    report::{DataQualityLevel, data_quality_summary},
    scorer,
    session_io::{self, RunValidationReport},
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

pub fn summary_command(
    path: &Path,
    json: bool,
    top: usize,
    filter_class: Option<TaskClass>,
) -> anyhow::Result<()> {
    let summary = build_compact_run_summary(path, top, filter_class)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        print!("{}", render_compact_run_summary(&summary));
    }
    Ok(())
}

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
        schema_version: session.core.schema_version,
        expected_schema_version: SESSION_SCHEMA_VERSION,
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

pub fn render_compact_run_summary(summary: &CompactRunSummary) -> String {
    let mut output = String::new();
    pushln(&mut output, "stutter summary");
    pushln(&mut output, "===============");
    pushln(&mut output, format!("file: {}", summary.path.display()));
    pushln(
        &mut output,
        format!("run: {}", summary.run_name.as_deref().unwrap_or("-")),
    );
    pushln(&mut output, format!("duration_ms: {}", summary.duration_ms));
    pushln(&mut output, format!("stop_reason: {}", summary.stop_reason));

    if let Some(warning) = crate::report::event_stream_warning(
        summary.event_stream_write_errors,
        summary.first_event_stream_write_error.as_deref(),
    ) {
        pushln(&mut output, warning);
    }

    pushln(
        &mut output,
        format!("data_quality: {:?}", summary.data_quality_level),
    );
    pushln(
        &mut output,
        format!(
            "targets: manual_pids={} tree_roots={} active_target_pids={} active_expanded_tasks={}",
            summary.target_counts.manual_pids,
            summary.target_counts.tree_roots,
            summary.target_counts.active_target_pids,
            summary.target_counts.active_expanded_tasks
        ),
    );
    pushln(
        &mut output,
        format!(
            "artifacts (reported): intervals={} spikes={} irq={} gpu={} frames={} migrations={} cpu_freq={} block_io={} scx={}",
            summary.artifact_counts.reported_interval_records,
            summary.artifact_counts.reported_spike_events,
            summary.artifact_counts.reported_irq_events,
            summary.artifact_counts.reported_gpu_samples,
            summary.artifact_counts.reported_frame_events,
            summary.artifact_counts.reported_migration_events,
            summary.artifact_counts.reported_cpu_freq_samples,
            summary.artifact_counts.reported_block_io_events,
            summary.artifact_counts.reported_scx_events
        ),
    );
    pushln(
        &mut output,
        format!(
            "threshold_totals: over_1ms={} over_2ms={} over_5ms={}",
            summary.threshold_totals.over_1ms,
            summary.threshold_totals.over_2ms,
            summary.threshold_totals.over_5ms
        ),
    );
    pushln(
        &mut output,
        format!(
            "spike_events: retained={} dropped={} truncated={}",
            summary.spike_events_retained_count,
            summary.spike_events_dropped_count,
            summary.spike_events_truncated
        ),
    );
    pushln(
        &mut output,
        format!("intervals_dropped: {}", summary.intervals_dropped),
    );

    if let Some(task) = &summary.worst_task_by_max_latency {
        pushln(
            &mut output,
            format!(
                "worst_max: task={} class={:?} comm={} max={}",
                task.task,
                task.class,
                task.comm,
                format_latency(task.max_ns)
            ),
        );
    }
    if let Some(task) = &summary.worst_p99_task {
        pushln(
            &mut output,
            format!(
                "worst_p99: task={} class={:?} comm={} p99={}",
                task.task,
                task.class,
                task.comm,
                format_latency(task.p99_ns)
            ),
        );
    }

    if !summary.top_tasks_by_max_latency.is_empty() {
        pushln(&mut output, "");
        pushln(&mut output, "top tasks by max latency");
        pushln(&mut output, "------------------------");
        for task in &summary.top_tasks_by_max_latency {
            pushln(
                &mut output,
                format!(
                    "task={} active={} class={:?} comm={} process={} samples={} max={} p99={} over_1ms={} over_2ms={} over_5ms={}",
                    task.task,
                    task.active,
                    task.class,
                    task.comm,
                    task.process_comm,
                    task.samples,
                    format_latency(task.max_ns),
                    format_latency(task.p99_ns),
                    task.over_1ms,
                    task.over_2ms,
                    task.over_5ms
                ),
            );
        }
    }

    output
}

pub fn render_batch_run_summary(summary: &BatchRunSummary, top: usize) -> String {
    let mut output = String::new();
    pushln(&mut output, "stutter batch summary");
    pushln(&mut output, "=====================");
    pushln(&mut output, format!("dir: {}", summary.batch_dir.display()));
    pushln(&mut output, format!("runs: {}", summary.run_count));
    if let Some(baseline) = &summary.baseline_path {
        pushln(&mut output, format!("baseline: {}", baseline.display()));
    }

    for (label, metric) in [
        ("best_p99", &summary.best_p99),
        ("worst_p99", &summary.worst_p99),
        ("best_max", &summary.best_max),
        ("worst_max", &summary.worst_max),
    ] {
        if let Some(metric) = metric {
            let task = metric
                .task
                .as_ref()
                .map(|task| format!(" task={} comm={}", task.task, task.comm))
                .unwrap_or_default();
            pushln(
                &mut output,
                format!(
                    "{label}: run={} value={}{}",
                    metric.path.display(),
                    format_latency(metric.value_ns),
                    task
                ),
            );
        }
    }

    if !summary.comparisons.is_empty() {
        pushln(&mut output, "");
        pushln(&mut output, "regressions relative to baseline");
        pushln(&mut output, "--------------------------------");
        for diff in &summary.comparisons {
            if let Some(worst) = &diff.worst_p99_regression {
                pushln(
                    &mut output,
                    format!(
                        "run={} worst_p99_delta={} comm={} process={}",
                        diff.current_path.display(),
                        format_latency_signed(worst.delta_p99_ns),
                        worst.identity.comm,
                        worst.identity.process_comm
                    ),
                );
            } else {
                pushln(
                    &mut output,
                    format!("run={} worst_p99_delta=none", diff.current_path.display()),
                );
            }
        }
    }

    if !summary.errors.is_empty() {
        pushln(&mut output, "");
        pushln(&mut output, "warnings");
        pushln(&mut output, "--------");
        for error in summary.errors.iter().take(top) {
            pushln(
                &mut output,
                format!("{}: {}", error.path.display(), error.error),
            );
        }
    }

    output
}

pub fn format_latency_signed(ns: i64) -> String {
    let abs_ns = ns.unsigned_abs();
    let sign = if ns >= 0 { "+" } else { "-" };
    format!("{sign}{}", format_latency(abs_ns))
}

fn discover_run_dirs(batch_dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for entry in fs::read_dir(batch_dir)
        .with_context(|| format!("failed to read batch directory {}", batch_dir.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", batch_dir.display()))?;
        let path = entry.path();
        if path.is_dir() && path.join("session.json").exists() {
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
        process_comm: task.process_comm.to_string(),
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
            process_comm: task.process_comm.to_string(),
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

fn pushln(output: &mut String, line: impl AsRef<str>) {
    output.push_str(line.as_ref());
    output.push('\n');
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::recorder::{RecordedLatency, SessionTask};

    fn test_session(tasks: Vec<SessionTask>) -> SessionFile {
        SessionFile {
            core: crate::recorder::SessionMetadataCore {
                schema_version: SESSION_SCHEMA_VERSION,
                run_name: Some("test-run".to_owned()),
                started_at: crate::recorder::RecordedTime::default(),
                ended_at: crate::recorder::RecordedTime::default(),
                duration_ms: 1000,
                interval_record_count: 7,
                ..Default::default()
            },
            stop_reason: "test".to_owned(),
            config: crate::recorder::RecordedConfig::default(),
            tasks,
            top_spikes: Vec::new(),
        }
    }

    fn test_task(task: u32, class: TaskClass, comm: &str, p99_ns: u64, max_ns: u64) -> SessionTask {
        SessionTask {
            task,
            active: true,
            class,
            process_pid: Some(task),
            process_comm: "game".into(),
            comm: comm.to_owned(),
            latency: RecordedLatency {
                samples: 10,
                p99_ns,
                max_ns,
                over_1ms: 1,
                over_2ms: 2,
                over_5ms: 3,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-summary-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_session(dir: &Path, session: &SessionFile) {
        fs::create_dir_all(dir).unwrap();
        fs::write(
            dir.join("session.json"),
            serde_json::to_string(session).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn compact_summary_filters_top_tasks_by_class() {
        let session = test_session(vec![
            test_task(1, TaskClass::Game, "game", 2_000_000, 5_000_000),
            test_task(2, TaskClass::Helper, "helper", 9_000_000, 9_000_000),
        ]);

        let summary = compact_run_summary_from_session(
            Path::new("/tmp/run"),
            &session,
            &RunValidationReport::default(),
            10,
            Some(TaskClass::Game),
        );

        assert_eq!(summary.top_tasks_by_max_latency.len(), 1);
        assert_eq!(summary.top_tasks_by_max_latency[0].comm, "game");
        assert_eq!(summary.threshold_totals.over_1ms, 1);
    }

    #[test]
    fn compact_summary_json_does_not_include_histogram() {
        let session = test_session(vec![test_task(
            1,
            TaskClass::Game,
            "game",
            2_000_000,
            5_000_000,
        )]);

        let summary = compact_run_summary_from_session(
            Path::new("/tmp/run"),
            &session,
            &RunValidationReport::default(),
            10,
            None,
        );

        let json = serde_json::to_string(&summary).unwrap();
        assert!(!json.contains("histogram"));
        assert!(json.contains("worst_task_by_max_latency"));
    }

    #[test]
    fn run_diff_summary_tracks_regressions_and_new_scored_tasks() {
        let baseline = test_session(vec![test_task(
            1,
            TaskClass::Game,
            "game",
            2_000_000,
            5_000_000,
        )]);
        let current = test_session(vec![
            test_task(1, TaskClass::Game, "game", 3_000_000, 8_000_000),
            test_task(2, TaskClass::Game, "new-game", 1_000_000, 2_000_000),
        ]);

        let diff = run_diff_summary_from_sessions(
            Path::new("/tmp/base"),
            Path::new("/tmp/current"),
            &baseline,
            &current,
            None,
        );

        assert_eq!(diff.compared_tasks, 1);
        assert_eq!(diff.regressions.len(), 1);
        assert_eq!(diff.regressions[0].delta_max_ns, 3_000_000);
        assert_eq!(diff.new_scored_tasks.len(), 1);
    }

    #[test]
    fn batch_summary_collects_runs_and_structured_errors() {
        let root = temp_dir("batch");
        let run_a = root.join("a");
        let run_b = root.join("b");
        let bad = root.join("bad");

        write_session(
            &run_a,
            &test_session(vec![test_task(
                1,
                TaskClass::Game,
                "game",
                2_000_000,
                5_000_000,
            )]),
        );
        write_session(
            &run_b,
            &test_session(vec![test_task(
                1,
                TaskClass::Game,
                "game",
                3_000_000,
                6_000_000,
            )]),
        );
        fs::create_dir_all(&bad).unwrap();
        fs::write(bad.join("session.json"), "not json").unwrap();

        let summary = build_batch_run_summary(&root, None, 10, None).unwrap();

        assert_eq!(summary.run_count, 2);
        assert!(summary.worst_p99.is_some());
        assert_eq!(summary.errors.len(), 1);
        assert!(summary.errors[0].error.contains("failed to parse"));

        fs::remove_dir_all(root).ok();
    }
}
