use super::pushln;
use crate::model::ReportHeaderSummary;

pub fn render_header(summary: &ReportHeaderSummary) -> String {
    let mut output = String::new();

    pushln(&mut output, format!("file: {}", summary.file_path));
    pushln(&mut output, format!("schema: {}", summary.schema_version));
    pushln(
        &mut output,
        format!("expected_schema: {}", summary.expected_schema_version),
    );
    pushln(&mut output, format!("run: {}", summary.run_name));
    pushln(&mut output, format!("duration_ms: {}", summary.duration_ms));
    pushln(&mut output, format!("stop_reason: {}", summary.stop_reason));
    pushln(
        &mut output,
        format!("manual_pids: {:?}", summary.manual_pids),
    );
    pushln(&mut output, format!("tree_roots: {:?}", summary.tree_roots));
    pushln(
        &mut output,
        format!("include_comm: {:?}", summary.include_comm),
    );
    pushln(
        &mut output,
        format!("exclude_comm: {:?}", summary.exclude_comm),
    );

    if let Some(warning) = &summary.event_stream_warning {
        pushln(&mut output, warning);
        pushln(&mut output, "");
    }
    pushln(
        &mut output,
        format!("watch_process: {}", summary.watch_process),
    );
    pushln(&mut output, format!("persistent: {}", summary.persistent));
    pushln(&mut output, format!("csv_stream: {}", summary.csv_stream));
    pushln(
        &mut output,
        format!("active_tasks_at_end: {}", summary.active_target_pids_count),
    );
    pushln(&mut output, "");

    output
}
