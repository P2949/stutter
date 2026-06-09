use super::*;

pub(super) fn render_pre_task_warning_sections(session: &SessionFile, top: usize) -> String {
    let mut writer = ReportTextWriter::new();
    push_spike_event_warning(&mut writer, session);
    push_scx_events(&mut writer, session);
    push_correlation_artifacts(&mut writer, session);
    push_drm_fence_warning(&mut writer, session);
    push_percentile_warnings(&mut writer, session, top);
    writer.finish()
}

pub(super) fn render_correlation_sections(
    correlation_sections: &TextReportCorrelationSections,
) -> String {
    let mapped_correlation_sections = stutter_report::model::TextReportCorrelationSections {
        sections: correlation_sections
            .sections
            .iter()
            .map(|s| stutter_report::model::TextReportCorrelationSection {
                title: s.title.clone(),
                lines: s.lines.clone(),
            })
            .collect(),
    };
    let mut output = String::new();

    stutter_report::render::text::correlation::render_correlation_sections(
        &mut output,
        &mapped_correlation_sections,
    );
    output
}

fn push_spike_event_warning(writer: &mut ReportTextWriter, session: &SessionFile) {
    if !session.core.spike_events_truncated {
        return;
    }

    writer.line("spike event warning");
    writer.line("-------------------");
    writer.line(format!(
        "spike_events_truncated=true retained_spike_events={} note=spike_events.json is capped; top_spikes and threshold counters remain available",
        session.core.spike_events_retained_count
    ));
    writer.blank();
}

fn push_scx_events(writer: &mut ReportTextWriter, session: &SessionFile) {
    if session.core.scx_event_count == 0 {
        return;
    }

    writer.line(format!("scx_events: {}", session.core.scx_event_count));
    writer.blank();
}

fn push_correlation_artifacts(writer: &mut ReportTextWriter, session: &SessionFile) {
    if !has_correlation_artifacts(session) {
        return;
    }

    writer.line("correlation artifacts");
    writer.line("---------------------");
    writer.line(format!("irq_events: {}", session.core.irq_event_count));
    writer.line(format!("gpu_samples: {}", session.core.gpu_sample_count));
    writer.line(format!("frame_events: {}", session.core.frame_event_count));
    push_frame_alignment(writer, session);
    writer.line(format!(
        "migration_events: {}",
        session.core.migration_event_count.unwrap_or(0)
    ));
    writer.line(format!(
        "cpu_freq_samples: {}",
        session.core.cpu_freq_sample_count.unwrap_or(0)
    ));
    push_io_artifact_line(writer, session);
    writer.line(format!(
        "runtime_slices: {}",
        session.core.runtime_slice_count
    ));
    writer.blank();
    push_block_io_warning(writer, session);
}

fn has_correlation_artifacts(session: &SessionFile) -> bool {
    session.core.irq_event_count > 0
        || session.core.gpu_sample_count > 0
        || session.core.frame_event_count > 0
        || session.core.block_io_event_count > 0
        || session.core.runtime_slice_count > 0
        || session.core.migration_event_count.unwrap_or(0) > 0
        || session.core.cpu_freq_sample_count.unwrap_or(0) > 0
}

fn push_frame_alignment(writer: &mut ReportTextWriter, session: &SessionFile) {
    if session.core.frame_event_count == 0 {
        return;
    }

    let alignment = if session.core.mangohud_first_frame_monotonic_ns.is_some() {
        "monotonic_observed"
    } else {
        "approximate_first_row"
    };
    writer.line(format!("frame_timestamp_alignment={alignment}"));
}

fn push_io_artifact_line(writer: &mut ReportTextWriter, session: &SessionFile) {
    writer.line(format!(
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
    ));
}

fn push_block_io_warning(writer: &mut ReportTextWriter, session: &SessionFile) {
    let collisions = session.core.drop_counters.block_fallback_key_collisions;
    let zero_keys = session.core.drop_counters.block_zero_keys;
    let basis = block_io_correlation_basis(session);
    let has_fallback_warning = basis == "dev+sector"
        && (session.core.block_io_event_count > 0 || collisions > 0 || zero_keys > 0);
    let should_show = has_fallback_warning || (basis == "unavailable" && session.config.block_io);

    if !should_show {
        return;
    }

    writer.line("block i/o correlation warning");
    writer.line("----------------------------");
    if let Some(warning) = block_io_correlation_warning(session) {
        writer.line(format!("note: {warning}"));
    }
    if zero_keys > 0 {
        writer.line(format!(
            "note: block_zero_keys={zero_keys}; block I/O samples with reserved zero keys were dropped, so block I/O latency coverage may be incomplete."
        ));
    }
    if collisions > 0 {
        writer.line(format!(
            "note: block_fallback_key_collisions={collisions}; ambiguous fallback samples were dropped, so block I/O latency coverage may be incomplete."
        ));
    }
    writer.blank();
}

fn push_drm_fence_warning(writer: &mut ReportTextWriter, session: &SessionFile) {
    let missing_start = session.core.drop_counters.drm_fence_missing_start;
    if missing_start == 0 {
        return;
    }

    writer.line("drm fence warning");
    writer.line("-----------------");
    writer.line(format!(
        "note: drm_fence_missing_start={missing_start}; DRM fence wait-done events were observed without matching wait-start records, so some fence latency durations are incomplete."
    ));
    writer.blank();
}

fn push_percentile_warnings(writer: &mut ReportTextWriter, session: &SessionFile, top: usize) {
    let truncated = session
        .tasks
        .iter()
        .filter(|task| task.latency.truncated_samples > 0)
        .collect::<Vec<_>>();

    if truncated.is_empty() {
        return;
    }

    writer.line("percentile warnings");
    writer.line("-------------------");
    for task in truncated.iter().take(top) {
        writer.line(format!(
            "task={} comm={} truncated_samples={} percentile_scope={} note={}",
            task.task,
            task.comm,
            task.latency.truncated_samples,
            task.latency.percentile_scope,
            percentile_warning_note(&task.latency.percentile_scope)
        ));
    }
    writer.blank();
}
