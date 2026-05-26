use stutter_report::model::*;
use std::fs;

fn main() {
    let mut model = ReportModel::new();
    model.header = ReportHeaderSummary {
        source_path: "minimal.json".to_string(),
        schema_version: 22,
        expected_schema_version: 22,
        run_id: None,
        duration_ms: 1000,
        stop_reason: "user".to_string(),
        manual_target_pids: vec![],
        target_tree_roots: vec![],
        include_comm: vec![],
        exclude_comm: vec![],
        watch_process: None,
        persistent: false,
        csv_stream: None,
        active_tasks_at_end: 0,
    };
    model.data_quality = DataQualitySummary {
        level: DataQualityLevel::High,
        reasons: vec![],
        missing_optional_files: vec![],
        validation_errors: vec![],
        validation_warnings: vec![],
        probe_activation_warnings: vec![],
        schema_version: 22,
        expected_schema_version: 22,
        event_stream_write_errors: 0,
        spike_events_truncated: false,
        spike_events_retained_count: 0,
        spike_events_dropped_count: 0,
        interval_record_count: 0,
        active_target_pids_count: 0,
        drop_counters_nonzero: false,
        percentile_scope_counts: std::collections::BTreeMap::new(),
        block_io_correlation_basis: "none".to_string(),
        block_io_correlation_confidence: "high".to_string(),
        block_io_correlation_warning: None,
        frame_timestamp_alignment: "none".to_string(),
        cpu_perf_requested: false,
        cpu_perf_open_errors: 0,
        cpu_perf_read_errors: 0,
        cpu_perf_skipped_tasks: 0,
    };

    let json = serde_json::to_string_pretty(&model).unwrap();
    fs::write("stutter-report/tests/fixtures/minimal/input.json", json).unwrap();
}
