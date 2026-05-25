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
    sched_state::classify_switch_prev_state,
};

mod summary_sections;
pub(crate) use summary_sections::{
    render_check_summary, render_display_path_diagnosis_text, render_focus_summary_text,
    render_foreground_summary_text,
};

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

    let header_summary = stutter_report::model::ReportHeaderSummary {
        file_path: path.display().to_string(),
        schema_version: session.core.schema_version,
        expected_schema_version: SESSION_SCHEMA_VERSION,
        run_name: session.core.run_name.clone().unwrap_or_else(|| "-".to_owned()),
        duration_ms: session.core.duration_ms,
        stop_reason: session.stop_reason.clone(),
        manual_pids: session.config.manual_pids.clone(),
        tree_roots: session.config.tree_roots.clone(),
        include_comm: session.config.include_comm.clone(),
        exclude_comm: session.config.exclude_comm.clone(),
        event_stream_warning: event_stream_warning(
            session.core.event_stream_write_errors,
            session.core.first_event_stream_write_error.as_deref(),
        ),
        watch_process: session.config.watch_process.clone().unwrap_or_else(|| "-".to_owned()),
        persistent: session.config.persistent,
        csv_stream: match &session.config.csv_stream {
            Some(crate::config::CsvStreamTarget::File(path)) => path.display().to_string(),
            Some(crate::config::CsvStreamTarget::Stdout) => "stdout".to_owned(),
            None => "-".to_owned(),
        },
        active_target_pids_count: session.core.active_target_pids_count,
    };
    output.push_str(&stutter_report::render::text::header::render_header(&header_summary));

    let mapped_data_quality = stutter_report::model::DataQualitySummary {
        level: match data_quality.level {
            crate::report::model::DataQualityLevel::High => stutter_report::model::DataQualityLevel::High,
            crate::report::model::DataQualityLevel::Medium => stutter_report::model::DataQualityLevel::Medium,
            crate::report::model::DataQualityLevel::Low => stutter_report::model::DataQualityLevel::Low,
        },
        schema_version: data_quality.schema_version,
        expected_schema_version: data_quality.expected_schema_version,
        event_stream_write_errors: data_quality.event_stream_write_errors,
        spike_events_retained_count: data_quality.spike_events_retained_count,
        spike_events_dropped_count: data_quality.spike_events_dropped_count,
        spike_events_truncated: data_quality.spike_events_truncated,
        interval_record_count: data_quality.interval_record_count,
        active_target_pids_count: data_quality.active_target_pids_count,
        drop_counters_nonzero: data_quality.drop_counters_nonzero,
        percentile_scope_counts: data_quality.percentile_scope_counts.iter().map(|(k, &v)| (k.clone(), v as u64)).collect(),
        block_io_correlation_basis: data_quality.block_io_correlation_basis.to_string(),
        block_io_correlation_confidence: data_quality.block_io_correlation_confidence.to_string(),
        block_io_correlation_warning: data_quality.block_io_correlation_warning.clone(),
        probe_activation_warnings: data_quality.probe_activation_warnings.clone(),
        frame_timestamp_alignment: data_quality.frame_timestamp_alignment.to_string(),
        cpu_perf_requested: data_quality.cpu_perf_requested,
        cpu_perf_open_errors: data_quality.cpu_perf_open_errors,
        cpu_perf_read_errors: data_quality.cpu_perf_read_errors,
        cpu_perf_skipped_tasks: data_quality.cpu_perf_skipped_tasks,
        reasons: data_quality.reasons.clone(),
        missing_optional_files: data_quality.missing_optional_files.clone(),
        validation_warnings: data_quality.validation_warnings.clone(),
        validation_errors: data_quality.validation_errors.clone(),
    };
    output.push_str(&stutter_report::render::text::quality::render_data_quality(&mapped_data_quality));

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
        let block_zero_keys = session.core.drop_counters.block_zero_keys;
        let basis = block_io_correlation_basis(session);
        let has_block_fallback_warning = basis == "dev+sector"
            && (session.core.block_io_event_count > 0
                || block_fallback_key_collisions > 0
                || block_zero_keys > 0);
        let should_show_block_io_warning =
            has_block_fallback_warning || (basis == "unavailable" && session.config.block_io);
        if should_show_block_io_warning {
            pushln(&mut output, "block i/o correlation warning");
            pushln(&mut output, "----------------------------");
            if let Some(warning) = block_io_correlation_warning(session) {
                pushln(&mut output, format!("note: {warning}"));
            }
            if block_zero_keys > 0 {
                pushln(
                    &mut output,
                    format!(
                        "note: block_zero_keys={block_zero_keys}; block I/O samples with reserved zero keys were dropped, so block I/O latency coverage may be incomplete."
                    ),
                );
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

    let drm_fence_missing_start = session.core.drop_counters.drm_fence_missing_start;
    if drm_fence_missing_start > 0 {
        pushln(&mut output, "drm fence warning");
        pushln(&mut output, "-----------------");
        pushln(
            &mut output,
            format!(
                "note: drm_fence_missing_start={drm_fence_missing_start}; DRM fence wait-done events were observed without matching wait-start records, so some fence latency durations are incomplete."
            ),
        );
        pushln(&mut output, "");
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
        stutter_report::render::text::cluster::render_cluster_source(&stutter_report::model::SpikeClusterAnalysis {
            source: match cluster_analysis.source {
                crate::report::model::SpikeClusterSource::SpikeEvents => stutter_report::model::SpikeClusterSource::SpikeEvents,
                crate::report::model::SpikeClusterSource::TopSpikesFallback => stutter_report::model::SpikeClusterSource::TopSpikesFallback,
            },
            source_count: cluster_analysis.source_count,
            clusters: vec![],
        }, cluster_window_ms),
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
            let mapped_cluster = map_cluster(cluster);
            pushln(&mut output, stutter_report::render::text::cluster::render_cluster(rank + 1, &mapped_cluster));
        }
    }
    pushln(&mut output, "");

    if !frame_diagnoses.is_empty() {
        pushln(&mut output, "frame spike diagnoses");
        pushln(&mut output, "---------------------");
        for (rank, diag) in frame_diagnoses.iter().take(top).enumerate() {
            let mapped_diag = stutter_report::model::FrameDiagnosis {
                frame_elapsed_ms: diag.frame_elapsed_ms,
                frametime_ms: diag.frametime_ms,
                diagnosis: map_diagnosis(&diag.diagnosis),
            };
            pushln(&mut output, stutter_report::render::text::frame::render_frame_diagnosis(rank + 1, &mapped_diag));
        }
        pushln(&mut output, "");
    }

    let mapped_correlation_sections = stutter_report::model::TextReportCorrelationSections {
        sections: correlation_sections.sections.iter().map(|s| stutter_report::model::TextReportCorrelationSection {
            title: s.title.clone(),
            lines: s.lines.clone(),
        }).collect(),
    };
    stutter_report::render::text::correlation::render_correlation_sections(&mut output, &mapped_correlation_sections);

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

fn map_diagnosis(d: &crate::diagnosis::Diagnosis) -> stutter_report::model::Diagnosis {
    stutter_report::model::Diagnosis {
        primary: d.primary.as_ref().map(|p| stutter_report::model::DiagnosisPrimary {
            cause: format!("{:?}", p.cause),
            confidence: format!("{:?}", p.confidence),
            score: p.score,
            evidence: p.evidence.iter().map(|e| stutter_report::model::DiagnosisEvidence {
                kind: format!("{:?}", e.kind),
                strength: e.strength,
                message: e.message.clone(),
            }).collect(),
        }),
        candidates: d.candidates.iter().map(|c| stutter_report::model::DiagnosisCandidate {
            cause: format!("{:?}", c.cause),
            confidence: format!("{:?}", c.confidence),
            score: c.score,
            evidence: c.evidence.iter().map(|e| stutter_report::model::DiagnosisEvidence {
                kind: format!("{:?}", e.kind),
                strength: e.strength,
                message: e.message.clone(),
            }).collect(),
        }).collect(),
        missing_evidence: d.missing_evidence.clone(),
        candidate_rejections: d.candidate_rejections.iter().map(|r| stutter_report::model::DiagnosisRejection {
            cause: format!("{:?}", r.cause),
            score: r.score,
            confidence: format!("{:?}", r.confidence),
            reasons: r.reasons.clone(),
        }).collect(),
        secondary_causes: d.secondary_causes.iter().map(|s| format!("{:?}", s)).collect(),
        report_summary: d.report_summary().to_owned(),
    }
}

pub(crate) fn map_spike_point(p: &crate::spike::SpikePoint) -> stutter_report::model::SpikePoint {
    stutter_report::model::SpikePoint {
        task: p.task,
        class: format!("{:?}", p.class),
        process_pid: p.process_pid,
        comm: p.comm.clone(),
        cpu: p.cpu,
        wakeup_target_cpu: p.wakeup_target_cpu,
        latency_ns: p.latency_ns,
        wakeup_ns: p.wakeup_ns,
        switch_ns: p.switch_ns,
        target_pending_wakeups: p.target_pending_wakeups,
        observed_runnable_depth: p.observed_runnable_depth,
        switch_prev_pid: p.switch_prev_pid,
        switch_prev_state: p.switch_prev_state,
        switch_prev_state_label: p.switch_prev_state_label.clone(),
        scx_ops: p.scx_ops.clone(),
        primary_cause: p.primary_cause.clone(),
        cause_tags: p.cause_tags.clone(),
    }
}

pub(crate) fn map_cluster(c: &crate::spike::SpikeCluster) -> stutter_report::model::SpikeCluster {
    stutter_report::model::SpikeCluster {
        points: c.points.iter().map(map_spike_point).collect(),
        distinct_tasks: c.distinct_tasks,
        min_switch_ns: c.min_switch_ns,
        max_switch_ns: c.max_switch_ns,
        max_latency_ns: c.max_latency_ns,
        diagnosis: c.diagnosis.as_ref().map(map_diagnosis),
        wake_graph: c.wake_graph.iter().map(|w| stutter_report::model::WakeGraphEdge {
            waker_tid: w.waker_tid,
            waker_comm: w.waker_comm.clone(),
            wakee_tid: w.wakee_tid,
            wakee_comm: w.wakee_comm.clone(),
            count: w.count,
            max_latency_ns: w.max_latency_ns,
        }).collect(),
    }
}

fn pressure_timeline_has_pressure(summary: &PressureTimelineSummary) -> bool {
    summary.sample_count > 0
        && (summary.max_cpu_some > 0.0
            || summary.max_mem_some.unwrap_or(0.0) > 0.0
            || summary.max_mem_full.unwrap_or(0.0) > 0.0
            || summary.max_io_some.unwrap_or(0.0) > 0.0
            || summary.max_io_full.unwrap_or(0.0) > 0.0)
}
