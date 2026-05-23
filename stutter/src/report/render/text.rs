use std::{collections::BTreeSet, path::Path};

use super::super::{
    MAX_INLINE_CLUSTER_POINTS, MIN_CLUSTER_TASKS,
    analysis::{
        block_io_correlation_basis, block_io_correlation_confidence, block_io_correlation_warning,
        cluster_elapsed, cluster_labels, event_stream_warning, format_elapsed,
        format_optional_ratio, format_pressure_option, format_process_pid, format_task_cpu_perf,
        percentile_warning_note,
    },
    *,
};
use crate::{
    metrics::format_latency, recorder::SESSION_SCHEMA_VERSION,
    sched_state::classify_switch_prev_state, summary::format_latency_signed,
};

pub(crate) fn render_focus_summary_text(focus: &FocusReportSummary) -> String {
    let mut output = String::new();

    if !focus.is_visible() {
        return output;
    }

    pushln(&mut output, "Auto focus:");
    pushln(
        &mut output,
        format!("  mode: {}", focus.mode.as_deref().unwrap_or("unknown")),
    );
    pushln(
        &mut output,
        format!(
            "  final focus: {}",
            focus.final_focus.as_deref().unwrap_or("none")
        ),
    );
    pushln(
        &mut output,
        format!(
            "  situation: {}",
            focus.situation.as_deref().unwrap_or("unknown")
        ),
    );

    if let Some(confidence) = focus.confidence {
        pushln(&mut output, format!("  confidence: {:.2}", confidence));
    } else {
        pushln(&mut output, "  confidence: unknown");
    }

    pushln(&mut output, format!("  roots: {:?}", focus.roots));
    pushln(
        &mut output,
        format!("  focus switches: {}", focus.focus_switches),
    );

    if !focus.reasons.is_empty() {
        pushln(&mut output, "  reasons:");
        for reason in &focus.reasons {
            pushln(&mut output, format!("    - {reason}"));
        }
    }

    pushln(&mut output, "");
    output
}

pub(crate) fn render_foreground_summary_text(foreground: &ForegroundReportSummary) -> String {
    let mut output = String::new();

    if !foreground.is_visible() {
        return output;
    }

    pushln(&mut output, "Foreground window:");
    pushln(
        &mut output,
        format!(
            "  source: {}",
            foreground.source.as_deref().unwrap_or("unknown")
        ),
    );

    if let Some(pid) = foreground.final_pid {
        pushln(&mut output, format!("  final pid: {pid}"));
    } else {
        pushln(&mut output, "  final pid: none");
    }

    let app_or_class = foreground
        .final_app_id
        .as_deref()
        .or(foreground.final_class.as_deref())
        .unwrap_or("unknown");
    pushln(&mut output, format!("  app_id/class: {app_or_class}"));

    if let Some(window_id) = foreground.final_window_id.as_deref() {
        pushln(&mut output, format!("  window_id: {window_id}"));
    } else {
        pushln(&mut output, "  window_id: unknown");
    }

    if let Some(workspace) = foreground.final_workspace.as_deref() {
        pushln(&mut output, format!("  workspace: {workspace}"));
    } else {
        pushln(&mut output, "  workspace: unknown");
    }

    if let Some(title) = foreground.final_title.as_deref() {
        pushln(&mut output, format!("  title: {title}"));
    } else if foreground.enabled || foreground.event_count > 0 {
        pushln(
            &mut output,
            "  title: redacted (pass --foreground-include-title to record it)",
        );
    }

    if let Some(confidence) = foreground.confidence {
        pushln(&mut output, format!("  confidence: {:.2}", confidence));
    } else {
        pushln(&mut output, "  confidence: unknown");
    }

    if let Some(status) = foreground.provider_status.as_deref() {
        pushln(&mut output, format!("  provider status: {status}"));
    }

    if let Some(stale_ms) = foreground.stale_ms {
        pushln(&mut output, format!("  stale: yes, {stale_ms} ms"));
    } else {
        pushln(&mut output, "  stale: no");
    }

    pushln(&mut output, format!("  events: {}", foreground.event_count));

    if !foreground.reasons.is_empty() {
        pushln(&mut output, "  reasons:");
        for reason in &foreground.reasons {
            pushln(&mut output, format!("    - {reason}"));
        }
    }

    pushln(&mut output, "");
    output
}

pub(crate) fn render_display_path_diagnosis_text(
    diagnosis: &DisplayPathDiagnosisSummary,
) -> String {
    let mut output = String::new();

    if diagnosis.verdict.is_empty() {
        return output;
    }

    pushln(&mut output, "Display path diagnosis:");
    pushln(
        &mut output,
        format!(
            "  suspicion: {} score={:.2} confidence={}",
            diagnosis.verdict, diagnosis.suspicion_score, diagnosis.confidence
        ),
    );
    if let Some(is_cross_gpu) = diagnosis.is_cross_gpu {
        pushln(&mut output, format!("  cross_gpu: {is_cross_gpu}"));
    }
    if let Some(render) = diagnosis.render_gpu.as_deref() {
        pushln(&mut output, format!("  render_gpu: {render}"));
    }
    if let Some(scanout) = diagnosis.scanout_gpu.as_deref() {
        pushln(&mut output, format!("  scanout_gpu: {scanout}"));
    }
    pushln(
        &mut output,
        format!("  direct_scanout: {}", diagnosis.direct_scanout.status),
    );
    pushln(
        &mut output,
        format!(
            "  components: render={} fence={} kms={} wayland={} compositor={}",
            diagnosis.render_component.status,
            diagnosis.fence_component.status,
            diagnosis.kms_component.status,
            diagnosis.wayland_component.status,
            diagnosis.compositor_component.status
        ),
    );
    if !diagnosis.evidence.is_empty() {
        pushln(&mut output, "  evidence:");
        for evidence in diagnosis.evidence.iter().take(8) {
            pushln(&mut output, format!("    - {evidence}"));
        }
    }
    if !diagnosis.missing_evidence.is_empty() {
        pushln(&mut output, "  missing evidence:");
        for missing in diagnosis.missing_evidence.iter().take(8) {
            pushln(&mut output, format!("    - {missing}"));
        }
    }
    pushln(&mut output, "");
    output
}

pub(crate) fn render_check_summary(summary: &RegressionCheckSummary, top: usize) -> String {
    let mut output = String::new();
    pushln(&mut output, "stutter check");
    pushln(&mut output, "=============");
    pushln(
        &mut output,
        format!("baseline: {}", summary.baseline_path.display()),
    );
    pushln(
        &mut output,
        format!("current: {}", summary.current_path.display()),
    );
    pushln(
        &mut output,
        format!(
            "result: {}",
            if summary.passed { "passed" } else { "failed" }
        ),
    );
    if let Some(threshold) = summary.max_regression_p99_ms {
        pushln(&mut output, format!("max_regression_p99_ms: {threshold}"));
    }
    if let Some(threshold) = summary.max_max_regression_ms {
        pushln(&mut output, format!("max_max_regression_ms: {threshold}"));
    }

    if let Some(worst) = &summary.diff.worst_p99_regression {
        pushln(
            &mut output,
            format!(
                "worst_p99_regression: {} on comm={} process={}",
                format_latency_signed(worst.delta_p99_ns),
                worst.identity.comm,
                worst.identity.process_comm
            ),
        );
    } else {
        pushln(&mut output, "worst_p99_regression: none");
    }

    if let Some(worst) = &summary.diff.worst_max_regression {
        pushln(
            &mut output,
            format!(
                "worst_max_regression: {} on comm={} process={}",
                format_latency_signed(worst.delta_max_ns),
                worst.identity.comm,
                worst.identity.process_comm
            ),
        );
    } else {
        pushln(&mut output, "worst_max_regression: none");
    }

    if !summary.violations.is_empty() {
        pushln(&mut output, "");
        pushln(&mut output, "violations");
        pushln(&mut output, "----------");
        for violation in summary.violations.iter().take(top) {
            pushln(
                &mut output,
                format!(
                    "metric={:?} class={:?} comm={} process={} delta={} threshold={} new_task={}",
                    violation.metric,
                    violation.class,
                    violation.comm,
                    violation.process_comm,
                    format_latency_signed(violation.delta_ns),
                    format_latency(violation.threshold_ns as u64),
                    violation.new_task
                ),
            );
        }
    }

    if !summary.diff.regressions.is_empty() {
        pushln(&mut output, "");
        pushln(&mut output, "top regressions");
        pushln(&mut output, "---------------");
        for delta in summary.diff.regressions.iter().take(top) {
            pushln(
                &mut output,
                format!(
                    "class={:?} comm={} process={} p99_delta={} max_delta={} over_1ms_delta={}",
                    delta.identity.class,
                    delta.identity.comm,
                    delta.identity.process_comm,
                    format_latency_signed(delta.delta_p99_ns),
                    format_latency_signed(delta.delta_max_ns),
                    delta.delta_over_1ms
                ),
            );
        }
    }

    if !summary.diff.improvements.is_empty() {
        pushln(&mut output, "");
        pushln(&mut output, "top improvements");
        pushln(&mut output, "----------------");
        for delta in summary.diff.improvements.iter().take(top) {
            pushln(
                &mut output,
                format!(
                    "class={:?} comm={} process={} p99_delta={} max_delta={} over_1ms_delta={}",
                    delta.identity.class,
                    delta.identity.comm,
                    delta.identity.process_comm,
                    format_latency_signed(delta.delta_p99_ns),
                    format_latency_signed(delta.delta_max_ns),
                    delta.delta_over_1ms
                ),
            );
        }
    }

    output
}

pub(crate) struct TextReportRenderInput<'a> {
    pub path: &'a Path,
    pub session: &'a SessionFile,
    pub cluster_analysis: &'a SpikeClusterAnalysis,
    pub frame_diagnoses: &'a [FrameDiagnosis],
    pub data_quality: &'a DataQualitySummary,
    pub pressure_timeline: &'a PressureTimelineSummary,
    pub runtime_slice_summary: &'a RuntimeSliceAnalysisSummary,
    pub correlation_sections: &'a TextReportCorrelationSections,
    pub focus_summary: &'a FocusReportSummary,
    pub foreground_summary: &'a ForegroundReportSummary,
    pub display_path_diagnosis: Option<&'a DisplayPathDiagnosisSummary>,
    pub top: usize,
    pub cluster_window_ms: u64,
    pub filter_class: Option<TaskClass>,
}

pub(crate) fn render_report(input: TextReportRenderInput<'_>) -> String {
    let path = input.path;
    let session = input.session;
    let cluster_analysis = input.cluster_analysis;
    let frame_diagnoses = input.frame_diagnoses;
    let data_quality = input.data_quality;
    let pressure_timeline = input.pressure_timeline;
    let runtime_slice_summary = input.runtime_slice_summary;
    let correlation_sections = input.correlation_sections;
    let focus_summary = input.focus_summary;
    let foreground_summary = input.foreground_summary;
    let display_path_diagnosis = input.display_path_diagnosis;
    let top = input.top;
    let cluster_window_ms = input.cluster_window_ms;
    let filter_class = input.filter_class;
    let mut output = String::new();

    pushln(&mut output, "stutter report");
    pushln(&mut output, "==============");

    output.push_str(&render_focus_summary_text(focus_summary));
    output.push_str(&render_foreground_summary_text(foreground_summary));
    if let Some(display_path_diagnosis) = display_path_diagnosis {
        output.push_str(&render_display_path_diagnosis_text(display_path_diagnosis));
    }

    pushln(&mut output, format!("file: {}", path.display()));
    pushln(
        &mut output,
        format!("schema: {}", session.core.schema_version),
    );
    pushln(
        &mut output,
        format!("expected_schema: {}", SESSION_SCHEMA_VERSION),
    );
    pushln(
        &mut output,
        format!("run: {}", session.core.run_name.as_deref().unwrap_or("-")),
    );
    pushln(
        &mut output,
        format!("duration_ms: {}", session.core.duration_ms),
    );
    pushln(&mut output, format!("stop_reason: {}", session.stop_reason));
    pushln(
        &mut output,
        format!("manual_pids: {:?}", session.config.manual_pids),
    );
    pushln(
        &mut output,
        format!("tree_roots: {:?}", session.config.tree_roots),
    );
    pushln(
        &mut output,
        format!("include_comm: {:?}", session.config.include_comm),
    );
    pushln(
        &mut output,
        format!("exclude_comm: {:?}", session.config.exclude_comm),
    );

    if let Some(warning) = event_stream_warning(
        session.core.event_stream_write_errors,
        session.core.first_event_stream_write_error.as_deref(),
    ) {
        pushln(&mut output, warning);
        pushln(&mut output, "");
    }
    pushln(
        &mut output,
        format!(
            "watch_process: {}",
            session.config.watch_process.as_deref().unwrap_or("-")
        ),
    );
    pushln(
        &mut output,
        format!("persistent: {}", session.config.persistent),
    );
    pushln(
        &mut output,
        format!(
            "csv_stream: {}",
            match &session.config.csv_stream {
                Some(crate::config::CsvStreamTarget::File(path)) => path.display().to_string(),
                Some(crate::config::CsvStreamTarget::Stdout) => "stdout".to_owned(),
                None => "-".to_owned(),
            }
        ),
    );
    pushln(
        &mut output,
        format!(
            "active_tasks_at_end: {}",
            session.core.active_target_pids_count
        ),
    );
    pushln(&mut output, "");

    pushln(&mut output, "data quality");
    pushln(&mut output, "------------");
    pushln(&mut output, format!("level: {:?}", data_quality.level));
    pushln(
        &mut output,
        format!(
            "schema: {} expected={}",
            data_quality.schema_version, data_quality.expected_schema_version
        ),
    );
    pushln(
        &mut output,
        format!(
            "event_stream_write_errors: {}",
            data_quality.event_stream_write_errors
        ),
    );
    pushln(
        &mut output,
        format!(
            "spike_events: retained={} dropped={} truncated={}",
            data_quality.spike_events_retained_count,
            data_quality.spike_events_dropped_count,
            data_quality.spike_events_truncated
        ),
    );
    pushln(
        &mut output,
        format!("interval_records: {}", data_quality.interval_record_count),
    );
    pushln(
        &mut output,
        format!(
            "active_target_pids: {}",
            data_quality.active_target_pids_count
        ),
    );
    pushln(
        &mut output,
        format!(
            "drop_counters_nonzero: {}",
            data_quality.drop_counters_nonzero
        ),
    );
    pushln(
        &mut output,
        format!(
            "percentile_scope_counts: {:?}",
            data_quality.percentile_scope_counts
        ),
    );
    pushln(
        &mut output,
        format!(
            "block_io_correlation_basis: {} (confidence: {})",
            data_quality.block_io_correlation_basis, data_quality.block_io_correlation_confidence
        ),
    );
    pushln(
        &mut output,
        format!(
            "frame_timestamp_alignment: {}",
            data_quality.frame_timestamp_alignment
        ),
    );
    pushln(
        &mut output,
        format!(
            "cpu_perf: requested={} open_errors={} read_errors={} skipped_tasks={}",
            data_quality.cpu_perf_requested,
            data_quality.cpu_perf_open_errors,
            data_quality.cpu_perf_read_errors,
            data_quality.cpu_perf_skipped_tasks
        ),
    );

    for reason in &data_quality.reasons {
        pushln(&mut output, format!("reason: {reason}"));
    }

    if !data_quality.missing_optional_files.is_empty() {
        pushln(
            &mut output,
            format!(
                "missing_optional_files: {:?}",
                data_quality.missing_optional_files
            ),
        );
    }

    if !data_quality.validation_warnings.is_empty() {
        for warning in &data_quality.validation_warnings {
            pushln(&mut output, format!("validation_warning: {warning}"));
        }
    }

    if !data_quality.validation_errors.is_empty() {
        for error in &data_quality.validation_errors {
            pushln(&mut output, format!("validation_error: {error}"));
        }
    }

    pushln(&mut output, "");

    if pressure_timeline_has_pressure(pressure_timeline) {
        output.push_str(&render_pressure_timeline_summary(pressure_timeline));
        pushln(&mut output, "");
    }

    output.push_str(&render_runtime_slice_summary(runtime_slice_summary, top));

    if session.core.spike_events_truncated {
        pushln(&mut output, "spike event warning");
        pushln(&mut output, "-------------------");
        pushln(
            &mut output,
            format!(
                "spike_events_truncated=true retained_spike_events={} note=spike_events.json is capped; top_spikes and threshold counters remain available",
                session.core.spike_events_retained_count
            ),
        );
        pushln(&mut output, "");
    }

    if session.core.scx_event_count > 0 {
        pushln(
            &mut output,
            format!("scx_events: {}", session.core.scx_event_count),
        );
        pushln(&mut output, "");
    }
    if session.core.irq_event_count > 0
        || session.core.gpu_sample_count > 0
        || session.core.frame_event_count > 0
        || session.core.block_io_event_count > 0
        || session.core.runtime_slice_count > 0
        || session.core.migration_event_count.unwrap_or(0) > 0
        || session.core.cpu_freq_sample_count.unwrap_or(0) > 0
    {
        pushln(&mut output, "correlation artifacts");
        pushln(&mut output, "---------------------");
        pushln(
            &mut output,
            format!("irq_events: {}", session.core.irq_event_count),
        );
        pushln(
            &mut output,
            format!("gpu_samples: {}", session.core.gpu_sample_count),
        );
        pushln(
            &mut output,
            format!("frame_events: {}", session.core.frame_event_count),
        );
        if session.core.frame_event_count > 0 {
            let alignment = if session.core.mangohud_first_frame_monotonic_ns.is_some() {
                "monotonic_observed"
            } else {
                "approximate_first_row"
            };
            pushln(
                &mut output,
                format!("frame_timestamp_alignment={}", alignment),
            );
        }
        pushln(
            &mut output,
            format!(
                "migration_events: {}",
                session.core.migration_event_count.unwrap_or(0)
            ),
        );
        pushln(
            &mut output,
            format!(
                "cpu_freq_samples: {}",
                session.core.cpu_freq_sample_count.unwrap_or(0)
            ),
        );
        pushln(
            &mut output,
            format!(
                "io_events: {} ({}{})",
                session.core.block_io_event_count,
                block_io_correlation_basis(session),
                if block_io_correlation_basis(session) == "dev+sector" {
                    format!(
                        " correlated (advisory, approximate, confidence: {})",
                        block_io_correlation_confidence(session)
                    )
                } else {
                    format!(
                        " correlated (confidence: {})",
                        block_io_correlation_confidence(session)
                    )
                },
            ),
        );
        pushln(
            &mut output,
            format!("runtime_slices: {}", session.core.runtime_slice_count),
        );
        pushln(&mut output, "");

        let block_fallback_key_collisions =
            session.core.drop_counters.block_fallback_key_collisions;
        let basis = block_io_correlation_basis(session);
        let has_block_fallback_warning = basis == "dev+sector"
            && (session.core.block_io_event_count > 0 || block_fallback_key_collisions > 0);
        let should_show_block_io_warning =
            has_block_fallback_warning || (basis == "unavailable" && session.config.block_io);
        if should_show_block_io_warning {
            pushln(&mut output, "block i/o correlation warning");
            pushln(&mut output, "----------------------------");
            if let Some(warning) = block_io_correlation_warning(session) {
                pushln(&mut output, format!("note: {warning}"));
            }
            if block_fallback_key_collisions > 0 {
                pushln(
                    &mut output,
                    format!(
                        "note: block_fallback_key_collisions={block_fallback_key_collisions}; ambiguous fallback samples were dropped, so block I/O latency coverage may be incomplete."
                    ),
                );
            }
            pushln(&mut output, "");
        }
    }

    let truncated = session
        .tasks
        .iter()
        .filter(|task| task.latency.truncated_samples > 0)
        .collect::<Vec<_>>();

    if !truncated.is_empty() {
        pushln(&mut output, "percentile warnings");
        pushln(&mut output, "-------------------");
        for task in truncated.iter().take(top) {
            pushln(
                &mut output,
                format!(
                    "task={} comm={} truncated_samples={} percentile_scope={} note={}",
                    task.task,
                    task.comm,
                    task.latency.truncated_samples,
                    task.latency.percentile_scope,
                    percentile_warning_note(&task.latency.percentile_scope)
                ),
            );
        }
        pushln(&mut output, "");
    }

    let mut tasks = session
        .tasks
        .iter()
        .filter(|task| task.latency.samples > 0)
        .filter(|task| filter_class.is_none_or(|c| task.class == c))
        .collect::<Vec<_>>();

    tasks.sort_by_key(|task| std::cmp::Reverse(task.latency.max_ns));

    pushln(&mut output, "top tasks by max latency");
    pushln(&mut output, "------------------------");
    let duration_secs = session.core.duration_ms as f64 / 1000.0;
    for task in tasks.iter().take(top) {
        let spike_rate = if duration_secs > 0.0 {
            task.latency.over_1ms as f64 / duration_secs
        } else {
            0.0
        };
        pushln(
            &mut output,
            format!(
                "task={} active={} class={:?} comm={} process_pid={:?} samples={} max={} over_1ms={} over_2ms={} over_5ms={} spike_rate_per_s={:.1} percentile_scope={}{}",
                task.task,
                task.active,
                task.class,
                task.comm,
                task.process_pid,
                task.latency.samples,
                format_latency(task.latency.max_ns),
                task.latency.over_1ms,
                task.latency.over_2ms,
                task.latency.over_5ms,
                spike_rate,
                task.latency.percentile_scope,
                format_task_cpu_perf(task),
            ),
        );
    }
    pushln(&mut output, "");

    tasks.sort_by_key(|task| {
        (
            std::cmp::Reverse(task.latency.over_5ms),
            std::cmp::Reverse(task.latency.over_2ms),
            std::cmp::Reverse(task.latency.over_1ms),
            std::cmp::Reverse(task.latency.max_ns),
        )
    });

    pushln(&mut output, "top tasks by threshold counters");
    pushln(&mut output, "-------------------------------");
    for task in tasks.iter().take(top) {
        let spike_rate = if duration_secs > 0.0 {
            task.latency.over_1ms as f64 / duration_secs
        } else {
            0.0
        };
        pushln(
            &mut output,
            format!(
                "task={} active={} class={:?} comm={} over_5ms={} over_2ms={} over_1ms={} spike_rate_per_s={:.1} max={}",
                task.task,
                task.active,
                task.class,
                task.comm,
                task.latency.over_5ms,
                task.latency.over_2ms,
                task.latency.over_1ms,
                spike_rate,
                format_latency(task.latency.max_ns),
            ),
        );
    }
    pushln(&mut output, "");

    pushln(&mut output, "top spikes");
    pushln(&mut output, "----------");
    for spike in session.top_spikes.iter().take(top) {
        pushln(
            &mut output,
            format!(
                "task={} active={} class={:?} comm={} cpu={} wakeup_target_cpu={} latency={} wakeup_ns={} switch_ns={} observed_runnable_depth={} target_pending_wakeups={}(diagnostic) switch_prev_pid={} switch_prev_state={} switch_prev_state_label={}",
                spike.task,
                spike.active,
                spike.class,
                spike.comm,
                spike.cpu,
                spike.wakeup_target_cpu,
                format_latency(spike.latency_ns),
                spike.wakeup_ns,
                spike.switch_ns,
                spike.observed_runnable_depth,
                spike.target_pending_wakeups,
                spike.switch_prev_pid,
                spike.switch_prev_state,
                classify_switch_prev_state(spike.switch_prev_state),
            ),
        );
    }
    pushln(&mut output, "");

    pushln(&mut output, "spike clusters");
    pushln(&mut output, "--------------");
    pushln(
        &mut output,
        render_cluster_source(cluster_analysis, cluster_window_ms),
    );
    pushln(
        &mut output,
        "observed_runnable_depth is an approximation of runnable pressure on the CPU reconstructed",
    );
    pushln(
        &mut output,
        "from sched tracepoints. target_pending_wakeups is diagnostic-only monitored-target backlog.",
    );
    pushln(
        &mut output,
        "It is not kernel runqueue depth and must not be used for scoring or tuning decisions.",
    );
    pushln(&mut output, "");
    if cluster_analysis.clusters.is_empty() {
        pushln(
            &mut output,
            format!(
                "none min_tasks={} window_ms={}",
                MIN_CLUSTER_TASKS, cluster_window_ms
            ),
        );
    } else {
        for (rank, cluster) in cluster_analysis.clusters.iter().take(top).enumerate() {
            pushln(&mut output, render_cluster(rank + 1, cluster));
        }
    }
    pushln(&mut output, "");

    if !frame_diagnoses.is_empty() {
        pushln(&mut output, "frame spike diagnoses");
        pushln(&mut output, "---------------------");
        for (rank, diag) in frame_diagnoses.iter().take(top).enumerate() {
            pushln(&mut output, render_frame_diagnosis(rank + 1, diag));
        }
        pushln(&mut output, "");
    }

    render_correlation_sections(&mut output, correlation_sections);

    output
}

pub(crate) fn pushln(output: &mut String, line: impl AsRef<str>) {
    output.push_str(line.as_ref());
    output.push('\n');
}

pub(crate) fn render_pressure_timeline_summary(summary: &PressureTimelineSummary) -> String {
    let mut output = String::new();
    let windows_near_spikes = summary
        .windows
        .iter()
        .filter(|window| window.near_spike)
        .count();

    pushln(&mut output, "pressure timeline");
    pushln(&mut output, "-----------------");
    pushln(
        &mut output,
        format!(
            "samples={} windows_near_spikes={} max_cpu_some={:.2}",
            summary.sample_count, windows_near_spikes, summary.max_cpu_some
        ),
    );
    pushln(
        &mut output,
        format!(
            "max_mem_some={} max_mem_full={} max_io_some={} max_io_full={}",
            format_pressure_option(summary.max_mem_some),
            format_pressure_option(summary.max_mem_full),
            format_pressure_option(summary.max_io_some),
            format_pressure_option(summary.max_io_full),
        ),
    );

    output
}

pub(crate) fn render_runtime_slice_summary(
    summary: &RuntimeSliceAnalysisSummary,
    top: usize,
) -> String {
    let mut output = String::new();

    if !summary.available && summary.missing_reason.is_none() {
        return output;
    }

    pushln(&mut output, "Runtime slices:");
    pushln(&mut output, format!("  samples: {}", summary.sample_count));
    if !summary.source_counts.is_empty() {
        let sources = summary
            .source_counts
            .iter()
            .map(|(source, count)| format!("{source}={count}"))
            .collect::<Vec<_>>()
            .join(" ");
        pushln(&mut output, format!("  source: {sources}"));
    }
    if let Some(reason) = &summary.missing_reason {
        pushln(&mut output, format!("  missing: {reason}"));
    }
    for note in &summary.data_quality_notes {
        pushln(&mut output, format!("  note: {note}"));
    }
    if summary.available {
        pushln(
            &mut output,
            "  context: supporting evidence only; not a primary diagnosis by itself",
        );
    }

    if !summary.high_runtime_threads.is_empty() {
        pushln(&mut output, "  top CPU-consuming threads near spikes:");
        for thread in summary.high_runtime_threads.iter().take(top) {
            pushln(
                &mut output,
                format!(
                    "    task={} comm={} process={} runtime={:.1}% wait={}",
                    thread.task,
                    thread.comm,
                    thread.process_comm,
                    thread.max_runtime_ratio * 100.0,
                    format_optional_ratio(thread.max_wait_ratio),
                ),
            );
        }
    }

    if !summary.high_wait_threads.is_empty() {
        pushln(&mut output, "  top runqueue-waiting threads near spikes:");
        for thread in summary.high_wait_threads.iter().take(top) {
            pushln(
                &mut output,
                format!(
                    "    task={} comm={} process={} runtime={:.1}% wait={}",
                    thread.task,
                    thread.comm,
                    thread.process_comm,
                    thread.max_runtime_ratio * 100.0,
                    format_optional_ratio(thread.max_wait_ratio),
                ),
            );
        }
    }

    pushln(&mut output, "");
    output
}

pub(crate) fn render_cluster_source(
    analysis: &SpikeClusterAnalysis,
    cluster_window_ms: u64,
) -> String {
    let source = match analysis.source {
        SpikeClusterSource::SpikeEvents => "source=spike_events",
        SpikeClusterSource::TopSpikesFallback => "source=top_spikes fallback",
    };
    format!(
        "{source} count={} window_ms={} min_tasks={}",
        analysis.source_count, cluster_window_ms, MIN_CLUSTER_TASKS
    )
}

pub(crate) fn render_correlation_sections(
    output: &mut String,
    correlations: &TextReportCorrelationSections,
) {
    for section in &correlations.sections {
        pushln(output, &section.title);
        pushln(output, "-".repeat(section.title.len()));
        for line in &section.lines {
            pushln(output, line);
        }
        pushln(output, "");
    }
}

pub(crate) fn render_cluster(rank: usize, cluster: &SpikeCluster) -> String {
    let labels = cluster_labels(cluster);
    let labels = if labels.is_empty() {
        "-".to_owned()
    } else {
        labels.join(",")
    };
    let span_ns = cluster.max_switch_ns.saturating_sub(cluster.min_switch_ns);
    let elapsed = cluster_elapsed(cluster);
    let cpu_list = cluster
        .points
        .iter()
        .map(|point| point.cpu)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|cpu| cpu.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let shown_points = cluster.points.len().min(MAX_INLINE_CLUSTER_POINTS);
    let omitted_points = cluster.points.len().saturating_sub(shown_points);
    let points = cluster
        .points
        .iter()
        .take(MAX_INLINE_CLUSTER_POINTS)
        .map(render_cluster_point)
        .collect::<Vec<_>>()
        .join(" ");

    let diagnosis_line = if let Some(d) = &cluster.diagnosis {
        format!("\n{}", render_diagnosis_lines(d, "  "))
    } else {
        String::new()
    };

    let wake_block = if cluster.wake_graph.is_empty() {
        String::new()
    } else {
        let mut wake_lines = Vec::new();
        wake_lines.push("\n  wake relationships:".to_owned());
        for edge in &cluster.wake_graph {
            wake_lines.push(format!(
                "    {} [{}] woke {} [{}] (count={}, max_lat={})",
                edge.waker_comm,
                edge.waker_tid,
                edge.wakee_comm,
                edge.wakee_tid,
                edge.count,
                format_latency(edge.max_latency_ns)
            ));
        }
        wake_lines.join("\n")
    };

    format!(
        "#{rank} elapsed={} span={} tasks={} total_spikes={} shown_points={} omitted_points={} cpus={} labels={} max={} switch_ns={}..{} points={}{}{}",
        format_elapsed(elapsed),
        format_latency(span_ns),
        cluster.distinct_tasks,
        cluster.points.len(),
        shown_points,
        omitted_points,
        cpu_list,
        labels,
        format_latency(cluster.max_latency_ns),
        cluster.min_switch_ns,
        cluster.max_switch_ns,
        points,
        diagnosis_line,
        wake_block
    )
}

pub(crate) fn render_diagnosis_lines(diagnosis: &Diagnosis, indent: &str) -> String {
    let mut output = String::new();
    pushln(
        &mut output,
        format!("{indent}diagnosis: {}", diagnosis.report_summary()),
    );
    output.push_str(&render_diagnosis_detail_lines(diagnosis, indent));
    output.trim_end().to_owned()
}

pub(crate) fn render_diagnosis_detail_lines(diagnosis: &Diagnosis, indent: &str) -> String {
    let mut output = String::new();
    if !diagnosis.secondary_causes.is_empty() {
        pushln(
            &mut output,
            format!(
                "{indent}diagnosis_secondary causes={:?}",
                diagnosis.secondary_causes
            ),
        );
    }

    pushln(
        &mut output,
        format!("{indent}why this diagnosis was chosen:"),
    );
    if let Some(primary) = &diagnosis.primary {
        pushln(
            &mut output,
            format!(
                "{indent}  - primary={:?} confidence={:?} score={:.2}",
                primary.cause, primary.confidence, primary.score
            ),
        );
        for evidence in primary.evidence.iter().take(6) {
            pushln(
                &mut output,
                format!(
                    "{indent}  - evidence kind={:?} strength={:.2} msg={}",
                    evidence.kind, evidence.strength, evidence.message
                ),
            );
        }
    } else {
        pushln(
            &mut output,
            format!("{indent}  - no primary candidate met the reporting threshold"),
        );
    }

    pushln(
        &mut output,
        format!("{indent}evidence missing / not strong enough:"),
    );
    if diagnosis.missing_evidence.is_empty() {
        pushln(&mut output, format!("{indent}  - none recorded"));
    } else {
        for missing in diagnosis.missing_evidence.iter().take(6) {
            pushln(&mut output, format!("{indent}  - {missing}"));
        }
    }

    if !diagnosis.candidate_rejections.is_empty() {
        pushln(&mut output, format!("{indent}why not primary:"));
        for rejection in diagnosis.candidate_rejections.iter().take(3) {
            pushln(
                &mut output,
                format!(
                    "{indent}  - {:?} score={:.2} confidence={:?}",
                    rejection.cause, rejection.score, rejection.confidence
                ),
            );
            for reason in rejection.reasons.iter().take(3) {
                pushln(&mut output, format!("{indent}    - {reason}"));
            }
        }
    }

    pushln(&mut output, format!("{indent}diagnosis candidates:"));
    for candidate in diagnosis.candidates.iter().take(3) {
        pushln(
            &mut output,
            format!(
                "{indent}  - diagnosis_candidate cause={:?} confidence={:?} score={:.2} evidence_items={}",
                candidate.cause,
                candidate.confidence,
                candidate.score,
                candidate.evidence.len()
            ),
        );
    }

    if diagnosis.candidates.is_empty() {
        pushln(&mut output, format!("{indent}  - none recorded"));
    }

    output.trim_end().to_owned()
}

pub(crate) fn render_cluster_point(point: &SpikePoint) -> String {
    let scx = if let Some(ops) = &point.scx_ops {
        format!(" scx_ops={ops}")
    } else {
        String::new()
    };
    let prev_label = classify_switch_prev_state(point.switch_prev_state);
    format!(
        "{}({:?}:{} cpu={} wakeup_target_cpu={} latency={} switch_ns={} process_pid={} wakeup_ns={} observed_runnable_depth={} target_pending_wakeups={}(diag) switch_prev_pid={} switch_prev_state={} switch_prev_state_label={}{}{}{})",
        point.task,
        point.class,
        point.comm,
        point.cpu,
        point.wakeup_target_cpu,
        format_latency(point.latency_ns),
        point.switch_ns,
        format_process_pid(point.process_pid),
        point.wakeup_ns,
        point.observed_runnable_depth,
        point.target_pending_wakeups,
        point.switch_prev_pid,
        point.switch_prev_state,
        prev_label,
        scx,
        if let Some(p) = &point.primary_cause {
            format!(" primary_cause={}", p)
        } else {
            String::new()
        },
        if !point.cause_tags.is_empty() {
            format!(" tags={}", point.cause_tags.join(","))
        } else {
            String::new()
        }
    )
}

pub(crate) fn render_frame_diagnosis(rank: usize, diag: &FrameDiagnosis) -> String {
    let mut output = String::new();
    pushln(
        &mut output,
        format!(
            "{}. elapsed={}ms frametime={:.1}ms diagnosis: {}",
            rank,
            diag.frame_elapsed_ms,
            diag.frametime_ms,
            diag.diagnosis.report_summary()
        ),
    );
    output.push_str(&render_diagnosis_detail_lines(&diag.diagnosis, "  "));
    output.trim_end().to_owned()
}

fn pressure_timeline_has_pressure(summary: &PressureTimelineSummary) -> bool {
    summary.sample_count > 0
        && (summary.max_cpu_some > 0.0
            || summary.max_mem_some.unwrap_or(0.0) > 0.0
            || summary.max_mem_full.unwrap_or(0.0) > 0.0
            || summary.max_io_some.unwrap_or(0.0) > 0.0
            || summary.max_io_full.unwrap_or(0.0) > 0.0)
}
