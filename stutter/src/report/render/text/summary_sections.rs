//! Text report summary sections extracted from the main renderer.

use super::*;
use crate::summary::format_latency_signed;

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
