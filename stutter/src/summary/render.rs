use super::model::*;
use crate::metrics::format_latency;

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

fn pushln(output: &mut String, line: impl AsRef<str>) {
    output.push_str(line.as_ref());
    output.push('\n');
}
