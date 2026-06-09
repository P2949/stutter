use super::*;
use crate::summary::format_latency_signed;

pub(crate) fn render_focus_summary_text(focus: &FocusReportSummary) -> String {
    let mut writer = ReportTextWriter::new();

    if !focus.is_visible() {
        return writer.finish();
    }

    writer.line("Auto focus:");
    writer.line(format!(
        "  mode: {}",
        focus.mode.as_deref().unwrap_or("unknown")
    ));
    writer.line(format!(
        "  final focus: {}",
        focus.final_focus.as_deref().unwrap_or("none")
    ));
    writer.line(format!(
        "  situation: {}",
        focus.situation.as_deref().unwrap_or("unknown")
    ));
    render_focus_confidence(&mut writer, focus);
    writer.line(format!(
        "  roots: [{}]",
        focus
            .roots
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ));
    writer.line(format!("  focus switches: {}", focus.focus_switches));
    render_indented_reasons(&mut writer, &focus.reasons);
    writer.blank();
    writer.finish()
}

pub(crate) fn render_foreground_summary_text(foreground: &ForegroundReportSummary) -> String {
    let mut writer = ReportTextWriter::new();

    if !foreground.is_visible() {
        return writer.finish();
    }

    writer.line("Foreground window:");
    writer.line(format!(
        "  source: {}",
        foreground.source.as_deref().unwrap_or("unknown")
    ));
    render_foreground_identity(&mut writer, foreground);
    render_foreground_title(&mut writer, foreground);
    render_foreground_confidence(&mut writer, foreground);
    if let Some(status) = foreground.provider_status.as_deref() {
        writer.line(format!("  provider status: {status}"));
    }
    if let Some(stale_ms) = foreground.stale_ms {
        writer.line(format!("  stale: yes, {stale_ms} ms"));
    } else {
        writer.line("  stale: no");
    }
    writer.line(format!("  events: {}", foreground.event_count));
    render_indented_reasons(&mut writer, &foreground.reasons);
    writer.blank();
    writer.finish()
}

pub(crate) fn render_check_summary(summary: &RegressionCheckSummary, top: usize) -> String {
    let mut writer = ReportTextWriter::new();
    render_check_header(&mut writer, summary);
    render_check_worst_regressions(&mut writer, summary);
    render_check_violations(&mut writer, summary, top);
    render_check_deltas(
        &mut writer,
        "top regressions",
        &summary.diff.regressions,
        top,
    );
    render_check_deltas(
        &mut writer,
        "top improvements",
        &summary.diff.improvements,
        top,
    );
    writer.finish()
}

fn render_focus_confidence(writer: &mut ReportTextWriter, focus: &FocusReportSummary) {
    if let Some(confidence) = focus.confidence {
        writer.line(format!("  confidence: {:.2}", confidence));
    } else {
        writer.line("  confidence: unknown");
    }
}

fn render_foreground_identity(writer: &mut ReportTextWriter, foreground: &ForegroundReportSummary) {
    if let Some(pid) = foreground.final_pid {
        writer.line(format!("  final pid: {pid}"));
    } else {
        writer.line("  final pid: none");
    }

    let app_or_class = foreground
        .final_app_id
        .as_deref()
        .or(foreground.final_class.as_deref())
        .unwrap_or("unknown");
    writer.line(format!("  app_id/class: {app_or_class}"));
    writer.line(format!(
        "  window_id: {}",
        foreground.final_window_id.as_deref().unwrap_or("unknown")
    ));
    writer.line(format!(
        "  workspace: {}",
        foreground.final_workspace.as_deref().unwrap_or("unknown")
    ));
}

fn render_foreground_title(writer: &mut ReportTextWriter, foreground: &ForegroundReportSummary) {
    if let Some(title) = foreground.final_title.as_deref() {
        writer.line(format!("  title: {title}"));
    } else if foreground.enabled || foreground.event_count > 0 {
        writer.line("  title: redacted (pass --foreground-include-title to record it)");
    }
}

fn render_foreground_confidence(
    writer: &mut ReportTextWriter,
    foreground: &ForegroundReportSummary,
) {
    if let Some(confidence) = foreground.confidence {
        writer.line(format!("  confidence: {:.2}", confidence));
    } else {
        writer.line("  confidence: unknown");
    }
}

fn render_indented_reasons(writer: &mut ReportTextWriter, reasons: &[String]) {
    if reasons.is_empty() {
        return;
    }

    writer.line("  reasons:");
    for reason in reasons {
        writer.line(format!("    - {reason}"));
    }
}

fn render_check_header(writer: &mut ReportTextWriter, summary: &RegressionCheckSummary) {
    writer.line("stutter check");
    writer.line("=============");
    writer.line(format!("baseline: {}", summary.baseline_path.display()));
    writer.line(format!("current: {}", summary.current_path.display()));
    writer.line(format!(
        "result: {}",
        if summary.passed { "passed" } else { "failed" }
    ));
    if let Some(threshold) = summary.max_regression_p99_ms {
        writer.line(format!("max_regression_p99_ms: {threshold}"));
    }
    if let Some(threshold) = summary.max_max_regression_ms {
        writer.line(format!("max_max_regression_ms: {threshold}"));
    }
}

fn render_check_worst_regressions(writer: &mut ReportTextWriter, summary: &RegressionCheckSummary) {
    if let Some(worst) = &summary.diff.worst_p99_regression {
        writer.line(format!(
            "worst_p99_regression: {} on comm={} process={}",
            format_latency_signed(worst.delta_p99_ns),
            worst.identity.comm,
            worst.identity.process_comm
        ));
    } else {
        writer.line("worst_p99_regression: none");
    }

    if let Some(worst) = &summary.diff.worst_max_regression {
        writer.line(format!(
            "worst_max_regression: {} on comm={} process={}",
            format_latency_signed(worst.delta_max_ns),
            worst.identity.comm,
            worst.identity.process_comm
        ));
    } else {
        writer.line("worst_max_regression: none");
    }
}

fn render_check_violations(
    writer: &mut ReportTextWriter,
    summary: &RegressionCheckSummary,
    top: usize,
) {
    if summary.violations.is_empty() {
        return;
    }

    writer.blank();
    writer.line("violations");
    writer.line("----------");
    for violation in summary.violations.iter().take(top) {
        writer.line(format!(
            "metric={:?} class={:?} comm={} process={} delta={} threshold={} new_task={}",
            violation.metric,
            violation.class,
            violation.comm,
            violation.process_comm,
            format_latency_signed(violation.delta_ns),
            format_latency(violation.threshold_ns as u64),
            violation.new_task
        ));
    }
}

fn render_check_deltas(
    writer: &mut ReportTextWriter,
    title: &str,
    deltas: &[TaskDeltaSummary],
    top: usize,
) {
    if deltas.is_empty() {
        return;
    }

    writer.blank();
    writer.line(title);
    writer.line("-".repeat(title.len()));
    for delta in deltas.iter().take(top) {
        writer.line(format!(
            "class={:?} comm={} process={} p99_delta={} max_delta={} over_1ms_delta={}",
            delta.identity.class,
            delta.identity.comm,
            delta.identity.process_comm,
            format_latency_signed(delta.delta_p99_ns),
            format_latency_signed(delta.delta_max_ns),
            delta.delta_over_1ms
        ));
    }
}
