//! Unified display-path diagnosis summary.
//!
//! Owns report-only display-path suspicion scoring and component attribution. It does not own raw
//! probe parsing, live diagnosis, or A/B comparison.

use super::*;

pub(crate) struct DisplayPathDiagnosisInputs<'a> {
    pub(crate) frame_pacing: &'a FramePacingSummary,
    pub(crate) kms_timing: &'a KmsTimingSummary,
    pub(crate) drm_fence_timing: &'a DrmFenceTimingSummary,
    pub(crate) cross_gpu_fence: &'a CrossGpuFenceSummary,
    pub(crate) wayland_presentation: &'a WaylandPresentationSummary,
    pub(crate) direct_scanout: &'a DirectScanoutSummary,
    pub(crate) dmabuf_path: &'a DmaBufPathSummary,
    pub(crate) gpu_engine_activity: &'a GpuEngineActivitySummary,
}

pub(crate) fn build_display_path_diagnosis_summary(
    session: &SessionFile,
    inputs: DisplayPathDiagnosisInputs<'_>,
) -> DisplayPathDiagnosisSummary {
    let DisplayPathDiagnosisInputs {
        frame_pacing,
        kms_timing,
        drm_fence_timing,
        cross_gpu_fence,
        wayland_presentation,
        direct_scanout,
        dmabuf_path,
        gpu_engine_activity,
    } = inputs;
    let display_path = session.core.display_path.as_ref();
    let is_cross_gpu = display_path.and_then(|path| path.is_cross_gpu);
    let render_gpu =
        display_path.and_then(|path| path.render_driver.clone().or(path.render_gpu.clone()));
    let scanout_gpu = display_path.and_then(|path| {
        path.scanout_driver
            .clone()
            .or_else(|| path.scanout_gpu.clone())
    });
    let connector = display_path.and_then(|path| path.connector.clone());

    let render_component = render_component(gpu_engine_activity, drm_fence_timing, frame_pacing);
    let fence_component = fence_component(drm_fence_timing, cross_gpu_fence);
    let kms_component = kms_component(kms_timing, drm_fence_timing);
    let wayland_component = wayland_component(wayland_presentation, direct_scanout);
    let compositor_component =
        compositor_component(direct_scanout, gpu_engine_activity, wayland_presentation);

    let mut evidence = Vec::new();
    let mut missing_evidence = Vec::new();
    let mut notes = Vec::new();
    let mut score: f64 = 0.0;
    let mut evidence_categories = 0usize;

    if is_cross_gpu == Some(true) {
        score += 0.20;
        evidence.push(cross_gpu_evidence(
            display_path.and_then(|path| path.render_card.as_deref()),
            display_path.and_then(|path| path.scanout_card.as_deref()),
            render_gpu.as_deref(),
            scanout_gpu.as_deref(),
        ));
        evidence_categories += 1;
    } else if is_cross_gpu.is_none() {
        missing_evidence.push(
            "display topology could not determine whether render and scanout GPU differ".to_owned(),
        );
    }

    if drm_fence_timing.p99_wait_ms.is_some() {
        evidence_categories += 1;
    } else {
        missing_evidence.push("no duration-bearing DRM fence evidence".to_owned());
    }
    if drm_fence_timing
        .p99_wait_ms
        .is_some_and(|value| value > 1.0)
    {
        score += 0.25;
        evidence.push(format!(
            "display-side fence p99 wait: {:.2} ms",
            drm_fence_timing.p99_wait_ms.unwrap_or_default()
        ));
    }
    if cross_gpu_fence.high_confidence_count > 0 {
        score += 0.25;
        evidence.push(format!(
            "{} high-confidence cross-GPU fence wait candidates",
            cross_gpu_fence.high_confidence_count
        ));
    } else if cross_gpu_fence.candidate_count > 0 {
        score += 0.15;
        evidence.push(format!(
            "{} cross-GPU fence wait candidates",
            cross_gpu_fence.candidate_count
        ));
    }

    if kms_timing.p99_flip_ms.is_some() {
        evidence_categories += 1;
    } else {
        missing_evidence.push("no duration-bearing KMS/pageflip timing evidence".to_owned());
    }
    if kms_timing.p99_flip_ms.is_some_and(|value| value > 1.0) {
        score += 0.15;
        evidence.push(format!(
            "KMS/pageflip p99 duration: {:.2} ms",
            kms_timing.p99_flip_ms.unwrap_or_default()
        ));
    }

    if wayland_presentation.event_count > 0 {
        evidence_categories += 1;
    } else {
        missing_evidence.push("no Wayland presentation/direct-scanout evidence".to_owned());
    }
    if wayland_presentation
        .p99_commit_to_present_ms
        .is_some_and(|value| value > 1.0)
        || wayland_presentation.compositor_queue_candidate_count > 0
    {
        score += 0.15;
        evidence.push("Wayland presentation queue delay candidate".to_owned());
    }
    if matches!(direct_scanout.status.as_str(), "no" | "mixed") {
        score += 0.10;
        evidence.push(format!("direct scanout status: {}", direct_scanout.status));
    }

    if dmabuf_path.event_count > 0 {
        evidence_categories += 1;
    } else {
        missing_evidence.push("no DMABUF modifier/copy log evidence".to_owned());
    }
    if dmabuf_path.modifier_mismatch_count > 0 || dmabuf_path.copy_required_count > 0 {
        score += 0.10;
        evidence.push(format!(
            "DMABUF copy/modifier candidates: modifier_mismatch={} copy_required={}",
            dmabuf_path.modifier_mismatch_count, dmabuf_path.copy_required_count
        ));
    }

    if gpu_engine_activity.sample_count > 0 {
        evidence_categories += 1;
    } else {
        missing_evidence.push("no iGPU/dGPU engine activity samples".to_owned());
    }
    if gpu_engine_activity.igpu_blitter_activity_near_outliers > 0
        || gpu_engine_activity.igpu_render_activity_near_outliers > 0
    {
        score += 0.20;
        evidence.push("iGPU render/blitter activity near frame outliers".to_owned());
    }
    if gpu_engine_activity
        .max_amdgpu_gfx_busy_percent
        .is_some_and(|busy| busy >= 90.0)
    {
        score -= 0.20;
        evidence.push("AMD render GPU appears saturated near outliers".to_owned());
    } else if gpu_engine_activity
        .max_amdgpu_gfx_busy_percent
        .is_some_and(|busy| busy < 85.0)
    {
        score += 0.10;
        evidence.push(format!(
            "AMD render GPU not saturated near outliers: {:.1}%",
            gpu_engine_activity
                .max_amdgpu_gfx_busy_percent
                .unwrap_or_default()
        ));
    }

    if frame_pacing.outlier_count == 0 {
        notes.push(
            "frame pacing did not expose outliers for proximity-based display-path checks"
                .to_owned(),
        );
    }

    score = score.clamp(0.0, 1.0);
    evidence.sort();
    evidence.dedup();
    missing_evidence.sort();
    missing_evidence.dedup();

    DisplayPathDiagnosisSummary {
        verdict: suspicion_verdict(score).to_owned(),
        suspicion_score: round2(score),
        confidence: confidence_label(evidence_categories).to_owned(),
        render_gpu,
        scanout_gpu,
        connector,
        is_cross_gpu,
        direct_scanout: direct_scanout.clone(),
        cross_gpu_fence: cross_gpu_fence.clone(),
        dmabuf_path: (dmabuf_path.event_count > 0).then(|| dmabuf_path.clone()),
        gpu_engine_activity: (gpu_engine_activity.sample_count > 0)
            .then(|| gpu_engine_activity.clone()),
        render_component,
        fence_component,
        kms_component,
        wayland_component,
        compositor_component,
        evidence,
        missing_evidence,
        notes,
    }
}

fn render_component(
    gpu_engine: &GpuEngineActivitySummary,
    fence: &DrmFenceTimingSummary,
    frame_pacing: &FramePacingSummary,
) -> DisplayPathComponent {
    let mut component = DisplayPathComponent {
        status: "unknown".to_owned(),
        ..Default::default()
    };
    if gpu_engine
        .max_amdgpu_gfx_busy_percent
        .is_some_and(|busy| busy >= 90.0)
    {
        component.status = "likely".to_owned();
        component.score = 0.8;
        component
            .evidence
            .push("AMD/render GPU busy >= 90% near frame outliers".to_owned());
    } else if fence.render_gpu_wait_count > fence.display_gpu_wait_count
        && frame_pacing.outlier_count > 0
    {
        component.status = "candidate".to_owned();
        component.score = 0.4;
        component
            .evidence
            .push("render-side fence waits exceed display-side waits".to_owned());
    } else if gpu_engine.sample_count > 0 {
        component.status = "healthy".to_owned();
        component.score = 0.1;
    }
    component
}

fn fence_component(
    fence: &DrmFenceTimingSummary,
    cross_gpu: &CrossGpuFenceSummary,
) -> DisplayPathComponent {
    let mut evidence = Vec::new();
    let mut score: f64 = 0.0;
    if fence.p99_wait_ms.is_some_and(|value| value > 1.0) {
        score = score.max(0.6);
        evidence.push(format!(
            "DRM fence p99 wait {:.2} ms",
            fence.p99_wait_ms.unwrap_or_default()
        ));
    }
    if cross_gpu.high_confidence_count > 0 {
        score = score.max(0.85);
        evidence.push(format!(
            "{} high-confidence cross-GPU waits",
            cross_gpu.high_confidence_count
        ));
    } else if cross_gpu.candidate_count > 0 {
        score = score.max(0.55);
        evidence.push(format!(
            "{} cross-GPU wait candidates",
            cross_gpu.candidate_count
        ));
    }
    DisplayPathComponent {
        status: component_status(score, fence.event_count > 0),
        score,
        evidence,
    }
}

fn kms_component(kms: &KmsTimingSummary, fence: &DrmFenceTimingSummary) -> DisplayPathComponent {
    let mut evidence = Vec::new();
    let mut score: f64 = 0.0;
    if kms.p99_flip_ms.is_some_and(|value| value > 1.0) {
        score = score.max(0.65);
        evidence.push(format!(
            "KMS p99 flip duration {:.2} ms",
            kms.p99_flip_ms.unwrap_or_default()
        ));
    }
    if fence.waits_near_kms_delays > 0 {
        score = score.max(0.45);
        evidence.push(format!(
            "{} fence waits near KMS delays",
            fence.waits_near_kms_delays
        ));
    }
    DisplayPathComponent {
        status: component_status(score, kms.event_count > 0),
        score,
        evidence,
    }
}

fn wayland_component(
    wayland: &WaylandPresentationSummary,
    direct_scanout: &DirectScanoutSummary,
) -> DisplayPathComponent {
    let mut evidence = Vec::new();
    let mut score: f64 = 0.0;
    if wayland
        .p99_commit_to_present_ms
        .is_some_and(|value| value > 1.0)
    {
        score = score.max(0.65);
        evidence.push(format!(
            "Wayland commit-to-present p99 {:.2} ms",
            wayland.p99_commit_to_present_ms.unwrap_or_default()
        ));
    }
    if wayland.discarded_count > 0 {
        score = score.max(0.55);
        evidence.push(format!(
            "{} discarded presentation events",
            wayland.discarded_count
        ));
    }
    if matches!(direct_scanout.status.as_str(), "no" | "mixed") {
        score = score.max(0.45);
        evidence.push(format!("direct scanout {}", direct_scanout.status));
    }
    DisplayPathComponent {
        status: component_status(score, wayland.event_count > 0),
        score,
        evidence,
    }
}

fn compositor_component(
    direct_scanout: &DirectScanoutSummary,
    gpu_engine: &GpuEngineActivitySummary,
    wayland: &WaylandPresentationSummary,
) -> DisplayPathComponent {
    let mut evidence = Vec::new();
    let mut score: f64 = 0.0;
    if gpu_engine.igpu_blitter_activity_near_outliers > 0
        || gpu_engine.igpu_render_activity_near_outliers > 0
    {
        score = score.max(0.75);
        evidence.push("iGPU render/blitter activity near frame outliers".to_owned());
    }
    if matches!(direct_scanout.status.as_str(), "no" | "mixed") {
        score = score.max(0.50);
        evidence.push(format!("direct scanout {}", direct_scanout.status));
    }
    if wayland.compositor_queue_candidate_count > 0 {
        score = score.max(0.60);
        evidence.push(format!(
            "{} compositor queue candidates",
            wayland.compositor_queue_candidate_count
        ));
    }
    DisplayPathComponent {
        status: component_status(
            score,
            gpu_engine.sample_count > 0 || wayland.event_count > 0,
        ),
        score,
        evidence,
    }
}

fn component_status(score: f64, has_evidence: bool) -> String {
    if !has_evidence {
        "unknown"
    } else if score >= 0.75 {
        "likely"
    } else if score >= 0.35 {
        "candidate"
    } else {
        "healthy"
    }
    .to_owned()
}

fn suspicion_verdict(score: f64) -> &'static str {
    if score >= 0.75 {
        "very_likely"
    } else if score >= 0.50 {
        "likely"
    } else if score >= 0.25 {
        "possible"
    } else {
        "low"
    }
}

fn confidence_label(evidence_categories: usize) -> &'static str {
    match evidence_categories {
        5.. => "high",
        3 | 4 => "medium",
        1 | 2 => "low",
        _ => "missing",
    }
}

fn cross_gpu_evidence(
    render_card: Option<&str>,
    scanout_card: Option<&str>,
    render_gpu: Option<&str>,
    scanout_gpu: Option<&str>,
) -> String {
    let render = render_card.or(render_gpu).unwrap_or("render GPU");
    let scanout = scanout_card.or(scanout_gpu).unwrap_or("scanout GPU");
    format!("render and scanout GPU differ: {render} -> {scanout}")
}

fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}
