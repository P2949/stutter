use super::*;

pub(super) fn render_header_section(path: &Path, session: &SessionFile) -> String {
    let header_summary = stutter_report::model::ReportHeaderSummary {
        file_path: path.display().to_string(),
        schema_version: session.core.schema_version.get(),
        expected_schema_version: SESSION_SCHEMA_VERSION.get(),
        run_name: session
            .core
            .run_name
            .clone()
            .unwrap_or_else(|| "-".to_owned()),
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
        watch_process: session
            .config
            .watch_process
            .clone()
            .unwrap_or_else(|| "-".to_owned()),
        persistent: session.config.persistent,
        csv_stream: match &session.config.csv_stream {
            Some(crate::config::CsvStreamTarget::File(path)) => path.display().to_string(),
            Some(crate::config::CsvStreamTarget::Stdout) => "stdout".to_owned(),
            None => "-".to_owned(),
        },
        active_target_pids_count: session.core.active_target_pids_count,
    };

    stutter_report::render::text::header::render_header(&header_summary)
}

pub(super) fn render_data_quality_section(data_quality: &DataQualitySummary) -> String {
    let mapped_data_quality = stutter_report::model::DataQualitySummary {
        level: match data_quality.level {
            crate::report::model::DataQualityLevel::High => {
                stutter_report::model::DataQualityLevel::High
            }
            crate::report::model::DataQualityLevel::Medium => {
                stutter_report::model::DataQualityLevel::Medium
            }
            crate::report::model::DataQualityLevel::Low => {
                stutter_report::model::DataQualityLevel::Low
            }
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
        percentile_scope_counts: data_quality
            .percentile_scope_counts
            .iter()
            .map(|(k, &v)| (k.clone(), v))
            .collect(),
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

    stutter_report::render::text::quality::render_data_quality(&mapped_data_quality)
}
