use crate::{
    recorder::{DrmFenceEventRecord, KmsFlipEventRecord},
    report::ReportAnalysisJson,
};

pub(super) struct CompareEvidenceDeltas {
    pub(super) avg_fps_delta_percent: Option<f64>,
    pub(super) fence_delta_ms: Option<f64>,
    pub(super) kms_delta_ms: Option<f64>,
    pub(super) wayland_delta_ms: Option<f64>,
    pub(super) igpu_delta: Option<f64>,
    pub(super) dmabuf_copy_delta: Option<i64>,
}

pub(super) fn compare_evidence(
    baseline: &ReportAnalysisJson,
    test: &ReportAnalysisJson,
    deltas: CompareEvidenceDeltas,
) -> Vec<String> {
    let mut evidence = Vec::new();
    if let Some(delta) = deltas.avg_fps_delta_percent {
        evidence.push(format!("avg FPS delta: {delta:+.1}%"));
    }
    if let Some(delta) = deltas.fence_delta_ms {
        evidence.push(format!("display fence p99 delta: {delta:+.2} ms"));
    }
    if let Some(delta) = deltas.kms_delta_ms {
        evidence.push(format!("KMS p99 delta: {delta:+.2} ms"));
    }
    if let Some(delta) = deltas.wayland_delta_ms {
        evidence.push(format!(
            "Wayland commit-to-present p99 delta: {delta:+.2} ms"
        ));
    }
    if let Some(delta) = deltas.igpu_delta
        && delta != 0.0
    {
        evidence.push(format!(
            "iGPU render/blitter activity delta: {delta:+.0} samples"
        ));
    }
    if let Some(delta) = deltas.dmabuf_copy_delta
        && delta != 0
    {
        evidence.push(format!("DMABUF copy-required delta: {delta:+}"));
    }
    let suspicion_delta = test.display_path_diagnosis.suspicion_score
        - baseline.display_path_diagnosis.suspicion_score;
    if suspicion_delta.abs() >= 0.05 {
        evidence.push(format!(
            "display-path suspicion score delta: {suspicion_delta:+.2}"
        ));
    }
    evidence
}

pub(super) fn igpu_engine_activity(analysis: &ReportAnalysisJson) -> f64 {
    let summary = &analysis.gpu_engine_activity;
    (summary.igpu_render_activity_near_outliers + summary.igpu_blitter_activity_near_outliers)
        as f64
}

pub(super) fn display_path_label(analysis: &ReportAnalysisJson) -> Option<String> {
    analysis
        .session
        .core
        .display_path
        .as_ref()
        .and_then(|display| display.label.clone())
        .or_else(|| analysis.session.config.display_path_label.clone())
}

pub(super) fn fps(analysis: &ReportAnalysisJson) -> Option<f64> {
    (analysis.frame_pacing.frame_count > 0 && analysis.session.core.duration_ms > 0).then_some(
        analysis.frame_pacing.frame_count as f64
            / (analysis.session.core.duration_ms as f64 / 1000.0),
    )
}

pub(super) fn delta(test: Option<f64>, baseline: Option<f64>) -> Option<f64> {
    test.zip(baseline).map(|(test, baseline)| test - baseline)
}

pub(super) fn count_long_kms_flips(events: &[KmsFlipEventRecord]) -> usize {
    events
        .iter()
        .filter(|event| {
            event
                .duration_ns
                .is_some_and(|duration| duration >= 1_000_000)
        })
        .count()
}

pub(super) fn role_fence_p99_ms(events: &[DrmFenceEventRecord], role: &str) -> Option<f64> {
    let mut values = events
        .iter()
        .filter(|event| event.gpu_role.as_deref() == Some(role))
        .filter_map(|event| event.duration_ns)
        .map(|duration| duration as f64 / 1_000_000.0)
        .collect::<Vec<_>>();
    percentile(&mut values, 0.99)
}

pub(super) fn scheduler_p99_ms(analysis: &ReportAnalysisJson) -> Option<f64> {
    analysis
        .session
        .tasks
        .iter()
        .map(|task| task.latency.p99_ns as f64 / 1_000_000.0)
        .reduce(f64::max)
}

fn percentile(values: &mut [f64], percentile: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((values.len() - 1) as f64 * percentile).round() as usize;
    values.get(idx).copied()
}
