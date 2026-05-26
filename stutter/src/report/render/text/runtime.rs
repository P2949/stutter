use super::*;

pub(crate) fn render_runtime_slice_summary(
    summary: &RuntimeSliceAnalysisSummary,
    top: usize,
) -> String {
    let mut writer = ReportTextWriter::new();

    if !summary.available && summary.missing_reason.is_none() {
        return writer.finish();
    }

    writer.line("Runtime slices:");
    writer.line(format!("  samples: {}", summary.sample_count));
    if !summary.source_counts.is_empty() {
        let sources = summary
            .source_counts
            .iter()
            .map(|(source, count)| format!("{source}={count}"))
            .collect::<Vec<_>>()
            .join(" ");
        writer.line(format!("  source: {sources}"));
    }
    if let Some(reason) = &summary.missing_reason {
        writer.line(format!("  missing: {reason}"));
    }
    for note in &summary.data_quality_notes {
        writer.line(format!("  note: {note}"));
    }
    if summary.available {
        writer.line("  context: supporting evidence only; not a primary diagnosis by itself");
    }

    render_runtime_threads(
        &mut writer,
        "  top CPU-consuming threads near spikes:",
        &summary.high_runtime_threads,
        top,
    );
    render_runtime_threads(
        &mut writer,
        "  top runqueue-waiting threads near spikes:",
        &summary.high_wait_threads,
        top,
    );

    writer.blank();
    writer.finish()
}

fn render_runtime_threads(
    writer: &mut ReportTextWriter,
    title: &str,
    threads: &[RuntimeThreadSummary],
    top: usize,
) {
    if threads.is_empty() {
        return;
    }

    writer.line(title);
    for thread in threads.iter().take(top) {
        writer.line(format!(
            "    task={} comm={} process={} runtime={:.1}% wait={}",
            thread.task,
            thread.comm,
            thread.process_comm,
            thread.max_runtime_ratio * 100.0,
            format_optional_ratio(thread.max_wait_ratio),
        ));
    }
}
