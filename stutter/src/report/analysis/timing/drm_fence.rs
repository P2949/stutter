use super::*;

pub(crate) fn build_drm_fence_timing_summary(
    events: &[crate::recorder::DrmFenceEventRecord],
    kms_events: &[crate::recorder::KmsFlipEventRecord],
    frame_events: &[crate::recorder::FrameEvent],
) -> DrmFenceTimingSummary {
    let waits_ms: Vec<f64> = events
        .iter()
        .filter_map(|event| event.duration_ns)
        .map(|ns| ns as f64 / 1_000_000.0)
        .collect();
    let wait_interval_count = waits_ms.len();
    let render_gpu_wait_count = events
        .iter()
        .filter(|event| matches!(event.gpu_role.as_deref(), Some("render")))
        .count();
    let display_gpu_wait_count = events
        .iter()
        .filter(|event| matches!(event.gpu_role.as_deref(), Some("display")))
        .count();
    let waits_near_frame_outliers = drm_fence_waits_near_frame_outliers(events, frame_events);
    let waits_near_kms_delays = drm_fence_waits_near_kms_delays(events, kms_events);
    let cross_gpu_candidate_count = events
        .iter()
        .filter(|event| {
            event.duration_ns.is_some()
                && (event.importer_driver.as_deref() == Some("i915")
                    || event.gpu_role.as_deref() == Some("display"))
                && (event.exporter_driver.as_deref() == Some("amdgpu")
                    || event.source == "amdgpu"
                    || render_gpu_wait_count > 0)
        })
        .count();
    let mut notes = Vec::new();

    if events.is_empty() {
        notes.push("no DRM fence events present".to_owned());
    } else if wait_interval_count == 0 {
        notes.push("only signal or marker events present; wait duration unavailable".to_owned());
    }
    let evidence_quality = if events.is_empty() {
        missing_evidence("no DRM fence events present")
    } else if wait_interval_count == 0 {
        missing_evidence("DRM fence events were present, but wait duration was unavailable")
    } else {
        EvidenceQuality::Direct
    };

    DrmFenceTimingSummary {
        evidence_quality,
        event_count: events.len(),
        wait_interval_count,
        median_wait_ms: optional_median_ms(&waits_ms),
        p95_wait_ms: optional_percentile_ms(&waits_ms, 0.95),
        p99_wait_ms: optional_percentile_ms(&waits_ms, 0.99),
        max_wait_ms: waits_ms.iter().copied().reduce(f64::max),
        render_gpu_wait_count,
        display_gpu_wait_count,
        cross_gpu_candidate_count,
        waits_near_frame_outliers,
        waits_near_kms_delays,
        top_waits: top_drm_fence_waits(events),
        notes,
        confidence: if events.is_empty() {
            "missing".to_owned()
        } else {
            "low".to_owned()
        },
    }
}

pub(super) fn drm_fence_is_display_side(event: &crate::recorder::DrmFenceEventRecord) -> bool {
    event.gpu_role.as_deref() == Some("display")
        || event.importer_driver.as_deref() == Some("i915")
        || event.driver.as_deref() == Some("i915")
}

pub(super) fn drm_fence_driver_path_suggests_cross_gpu(
    event: &crate::recorder::DrmFenceEventRecord,
) -> bool {
    let display_side = event.importer_driver.as_deref() == Some("i915")
        || event.driver.as_deref() == Some("i915")
        || event.source == "i915";
    let render_side = event.exporter_driver.as_deref() == Some("amdgpu")
        || event.source == "amdgpu"
        || event.driver.as_deref() == Some("amdgpu");

    display_side && render_side
}

pub(super) fn drm_fence_wait_near_frame_outlier(
    event: &crate::recorder::DrmFenceEventRecord,
    frame_events: &[crate::recorder::FrameEvent],
) -> bool {
    let median = calculate_median_frametime(frame_events);
    let spikes = identify_frame_spikes(frame_events, median);
    drm_fence_event_near_frame_spikes(event, &spikes)
}

pub(super) fn drm_fence_wait_near_kms_delay(
    event: &crate::recorder::DrmFenceEventRecord,
    kms_events: &[crate::recorder::KmsFlipEventRecord],
) -> bool {
    drm_fence_event_near_kms_delay(event, kms_events, 1_000_000)
}

fn top_drm_fence_waits(
    events: &[crate::recorder::DrmFenceEventRecord],
) -> Vec<DrmFenceWaitSummary> {
    let mut waits = events
        .iter()
        .filter(|event| event.duration_ns.is_some())
        .collect::<Vec<_>>();
    waits.sort_by_key(|event| std::cmp::Reverse(event.duration_ns.unwrap_or(0)));
    waits
        .into_iter()
        .take(5)
        .map(|event| DrmFenceWaitSummary {
            elapsed_ms: event.elapsed_ms,
            duration_ms: event.duration_ns.map(|ns| ns as f64 / 1_000_000.0),
            source: event.source.clone(),
            gpu_role: event.gpu_role.clone(),
            context: event.context,
            seqno: event.seqno,
            correlation_basis: event.correlation_basis.clone(),
            confidence: event.confidence.clone(),
        })
        .collect()
}

fn drm_fence_waits_near_frame_outliers(
    events: &[crate::recorder::DrmFenceEventRecord],
    frame_events: &[crate::recorder::FrameEvent],
) -> usize {
    let median = calculate_median_frametime(frame_events);
    let spikes = identify_frame_spikes(frame_events, median);
    events
        .iter()
        .filter(|event| event.duration_ns.is_some())
        .filter(|event| drm_fence_event_near_frame_spikes(event, &spikes))
        .count()
}

fn drm_fence_waits_near_kms_delays(
    events: &[crate::recorder::DrmFenceEventRecord],
    kms_events: &[crate::recorder::KmsFlipEventRecord],
) -> usize {
    let threshold_ns = 1_000_000;
    events
        .iter()
        .filter(|event| event.duration_ns.is_some())
        .filter(|event| drm_fence_event_near_kms_delay(event, kms_events, threshold_ns))
        .count()
}

fn drm_fence_event_near_frame_spikes(
    event: &crate::recorder::DrmFenceEventRecord,
    spikes: &[crate::recorder::FrameEvent],
) -> bool {
    spikes
        .iter()
        .any(|frame| elapsed_near(event.elapsed_ms, frame.elapsed_ms, 50))
}

fn drm_fence_event_near_kms_delay(
    event: &crate::recorder::DrmFenceEventRecord,
    kms_events: &[crate::recorder::KmsFlipEventRecord],
    threshold_ns: u64,
) -> bool {
    kms_events.iter().any(|kms| {
        kms.duration_ns
            .is_some_and(|duration| duration >= threshold_ns)
            && elapsed_near(event.elapsed_ms, kms.elapsed_ms, 50)
    })
}
