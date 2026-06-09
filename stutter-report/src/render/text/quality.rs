use super::pushln;
use crate::model::DataQualitySummary;

pub fn render_data_quality(data_quality: &DataQualitySummary) -> String {
    let mut output = String::new();

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
    output
}
