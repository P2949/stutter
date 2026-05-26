use super::{
    drm_fence::{
        drm_fence_driver_path_suggests_cross_gpu, drm_fence_is_display_side,
        drm_fence_wait_near_frame_outlier, drm_fence_wait_near_kms_delay,
    },
    *,
};

pub(crate) fn build_cross_gpu_fence_summary(
    events: &[crate::recorder::DrmFenceEventRecord],
    kms_events: &[crate::recorder::KmsFlipEventRecord],
    frame_events: &[crate::recorder::FrameEvent],
    topology: Option<&crate::display_topology::DisplayTopologySnapshot>,
) -> CrossGpuFenceSummary {
    let mut candidates = events
        .iter()
        .filter(|event| event.duration_ns.is_some())
        .filter_map(|event| cross_gpu_fence_candidate(event, kms_events, frame_events))
        .collect::<Vec<_>>();
    candidates
        .sort_by_key(|candidate| std::cmp::Reverse(candidate.duration_ms.unwrap_or(0.0) as u64));
    let high_confidence_count = candidates
        .iter()
        .filter(|candidate| candidate.confidence == "high")
        .count();
    let display_waits_ms = events
        .iter()
        .filter(|event| drm_fence_is_display_side(event))
        .filter_map(|event| event.duration_ns)
        .map(|duration_ns| duration_ns as f64 / 1_000_000.0)
        .collect::<Vec<_>>();
    let display_side_wait_count = events
        .iter()
        .filter(|event| event.duration_ns.is_some() && drm_fence_is_display_side(event))
        .count();
    let render_side_wait_count = events
        .iter()
        .filter(|event| event.duration_ns.is_some() && event.gpu_role.as_deref() == Some("render"))
        .count();
    let waits_near_frame_outliers = candidates
        .iter()
        .filter(|candidate| candidate.near_frame_outlier)
        .count();
    let waits_near_kms_delays = candidates
        .iter()
        .filter(|candidate| candidate.near_kms_delay)
        .count();
    let mut notes = Vec::new();

    if events.is_empty() {
        notes.push("no DRM fence events present".to_owned());
    } else if candidates.is_empty() {
        notes.push(
            "no cross-GPU fence wait candidates found; driver/role/proximity evidence may be missing"
                .to_owned(),
        );
    }

    if topology
        .and_then(|topology| topology.guessed_path.as_ref())
        .and_then(|path| path.is_cross_gpu)
        == Some(true)
    {
        notes.push("display topology also suggests a cross-GPU render-to-scanout path".to_owned());
    }

    let confidence = if candidates.is_empty() {
        "missing"
    } else if high_confidence_count > 0 {
        "high"
    } else if candidates
        .iter()
        .any(|candidate| candidate.confidence == "medium")
    {
        "medium"
    } else {
        "low"
    }
    .to_owned();
    let evidence_quality = if candidates.is_empty() {
        missing_evidence("no cross-GPU fence wait candidates found")
    } else {
        EvidenceQuality::Derived
    };

    CrossGpuFenceSummary {
        evidence_quality,
        candidate_count: candidates.len(),
        high_confidence_count,
        display_side_wait_count,
        render_side_wait_count,
        waits_near_frame_outliers,
        waits_near_kms_delays,
        p95_display_wait_ms: optional_percentile_ms(&display_waits_ms, 0.95),
        p99_display_wait_ms: optional_percentile_ms(&display_waits_ms, 0.99),
        top_candidates: candidates.into_iter().take(8).collect(),
        confidence,
        notes,
    }
}

fn cross_gpu_fence_candidate(
    event: &crate::recorder::DrmFenceEventRecord,
    kms_events: &[crate::recorder::KmsFlipEventRecord],
    frame_events: &[crate::recorder::FrameEvent],
) -> Option<CrossGpuFenceCandidate> {
    if !drm_fence_is_display_side(event) || !drm_fence_driver_path_suggests_cross_gpu(event) {
        return None;
    }

    let near_frame_outlier = drm_fence_wait_near_frame_outlier(event, frame_events);
    let near_kms_delay = drm_fence_wait_near_kms_delay(event, kms_events);
    let stable_identity =
        event.seqno.is_some() && (event.context.is_some() || event.timeline_hash.is_some());
    let confidence = if stable_identity
        && event.importer_driver.as_deref() == Some("i915")
        && event.exporter_driver.as_deref() == Some("amdgpu")
        && near_frame_outlier
        && near_kms_delay
    {
        "high"
    } else if stable_identity
        || (event.importer_driver.is_some() && event.exporter_driver.is_some())
    {
        "medium"
    } else {
        "low"
    }
    .to_owned();

    Some(CrossGpuFenceCandidate {
        elapsed_ms: event.elapsed_ms,
        duration_ms: event
            .duration_ns
            .map(|duration_ns| duration_ns as f64 / 1_000_000.0),
        wait_start_ns: event.wait_start_ns,
        wait_done_ns: event.wait_done_ns,
        signal_ns: event.signal_ns,
        importer_driver: event.importer_driver.clone(),
        exporter_driver: event.exporter_driver.clone(),
        context: event.context,
        seqno: event.seqno,
        timeline_hash: event.timeline_hash,
        near_frame_outlier,
        near_kms_delay,
        confidence,
    })
}
