use super::DisplayPathCostSummary;

pub(super) fn print_display_path_compare(summary: &DisplayPathCostSummary) {
    println!("Display-path A/B verdict:");
    println!("  {}", summary.verdict);
    println!("  reason: {}", summary.verdict_reason.as_str());
    println!("  confidence_score: {:.2}", summary.confidence_score);
    println!();
    println!("Measured cost:");
    println!(
        "  labels:          {} -> {}",
        summary.baseline_label.as_deref().unwrap_or("baseline"),
        summary.test_label.as_deref().unwrap_or("test")
    );
    println!(
        "  FPS:             {}",
        format_optional_percent(summary.avg_fps_delta_percent)
    );
    println!(
        "  median frame:    {}",
        format_optional_ms(summary.median_frame_delta_ms)
    );
    println!(
        "  p95 frame:       {}",
        format_optional_ms(summary.p95_frame_delta_ms)
    );
    println!(
        "  p99 frame:       {}",
        format_optional_ms(summary.p99_frame_delta_ms)
    );
    println!(
        "  KMS p99:         {}",
        format_optional_ms(summary.kms_p99_delta_ms)
    );
    println!(
        "  fence p99:       {}",
        format_optional_ms(summary.display_side_fence_wait_p99_delta_ms)
    );
    println!(
        "  Wayland p99:     {}",
        format_optional_ms(summary.commit_to_present_p99_delta_ms)
    );
    println!(
        "  iGPU activity:   {}",
        format_optional_count_delta(summary.igpu_engine_activity_delta)
    );
    println!(
        "  DMABUF copies:   {}",
        format_optional_i64(summary.dmabuf_copy_required_delta)
    );
    println!("Comparison quality: {}", summary.comparison_quality);
    print_lines("Likely components:", &summary.likely_causes);
    print_lines("Evidence:", &summary.evidence);
    print_lines("Missing evidence:", &summary.missing_evidence);
    for warning in &summary.comparison_warnings {
        println!("warning: {warning}");
    }
    for note in &summary.notes {
        println!("note: {note}");
    }
}

fn print_lines(title: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }

    println!("{title}");
    for line in lines {
        println!("  - {line}");
    }
}

fn format_optional_ms(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:+.1} ms"))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn format_optional_percent(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:+.1}%"))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn format_optional_count_delta(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:+.0} samples"))
        .unwrap_or_else(|| "n/a".to_owned())
}

fn format_optional_i64(value: Option<i64>) -> String {
    value
        .map(|value| format!("{value:+}"))
        .unwrap_or_else(|| "n/a".to_owned())
}
