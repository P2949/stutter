use super::*;

pub(crate) fn render_display_path_diagnosis_text(
    diagnosis: &DisplayPathDiagnosisSummary,
) -> String {
    let mut writer = ReportTextWriter::new();

    if diagnosis.verdict.is_empty() {
        return writer.finish();
    }

    writer.line("Display path diagnosis:");
    writer.line(format!(
        "  suspicion: {} score={:.2} confidence={}",
        diagnosis.verdict, diagnosis.suspicion_score, diagnosis.confidence
    ));
    if let Some(is_cross_gpu) = diagnosis.is_cross_gpu {
        writer.line(format!("  cross_gpu: {is_cross_gpu}"));
    }
    if let Some(render) = diagnosis.render_gpu.as_deref() {
        writer.line(format!("  render_gpu: {render}"));
    }
    if let Some(scanout) = diagnosis.scanout_gpu.as_deref() {
        writer.line(format!("  scanout_gpu: {scanout}"));
    }
    writer.line(format!(
        "  direct_scanout: {}",
        diagnosis.direct_scanout.status
    ));
    writer.line(format!(
        "  components: render={} fence={} kms={} wayland={} compositor={}",
        diagnosis.render_component.status,
        diagnosis.fence_component.status,
        diagnosis.kms_component.status,
        diagnosis.wayland_component.status,
        diagnosis.compositor_component.status
    ));
    render_list(&mut writer, "  evidence:", &diagnosis.evidence);
    render_list(
        &mut writer,
        "  missing evidence:",
        &diagnosis.missing_evidence,
    );
    writer.blank();
    writer.finish()
}

fn render_list(writer: &mut ReportTextWriter, title: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }

    writer.line(title);
    for line in lines.iter().take(8) {
        writer.line(format!("    - {line}"));
    }
}
