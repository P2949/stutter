use super::*;

pub(crate) fn render_pressure_timeline_summary(summary: &PressureTimelineSummary) -> String {
    let mut writer = ReportTextWriter::new();
    let windows_near_spikes = summary
        .windows
        .iter()
        .filter(|window| window.near_spike)
        .count();

    writer.line("pressure timeline");
    writer.line("-----------------");
    writer.line(format!(
        "samples={} windows_near_spikes={} max_cpu_some={:.2}",
        summary.sample_count, windows_near_spikes, summary.max_cpu_some
    ));
    writer.line(format!(
        "max_mem_some={} max_mem_full={} max_io_some={} max_io_full={}",
        format_pressure_option(summary.max_mem_some),
        format_pressure_option(summary.max_mem_full),
        format_pressure_option(summary.max_io_some),
        format_pressure_option(summary.max_io_full),
    ));

    writer.finish()
}

pub(super) fn pressure_timeline_has_pressure(summary: &PressureTimelineSummary) -> bool {
    summary.sample_count > 0
        && (summary.max_cpu_some > 0.0
            || summary.max_mem_some.unwrap_or(0.0) > 0.0
            || summary.max_mem_full.unwrap_or(0.0) > 0.0
            || summary.max_io_some.unwrap_or(0.0) > 0.0
            || summary.max_io_full.unwrap_or(0.0) > 0.0)
}
