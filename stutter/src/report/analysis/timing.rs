//! Display timing report summaries.
//!
//! Owns KMS flip, DRM fence, and Wayland presentation summaries plus timing proximity and
//! percentile helpers. Does not own artifact loading, foreground/focus summaries, pressure
//! timelines, clustering, diagnosis, or report orchestration.

use std::collections::BTreeMap;

use super::*;

pub(crate) fn build_kms_timing_summary(
    events: &[crate::recorder::KmsFlipEventRecord],
) -> KmsTimingSummary {
    let durations_ms: Vec<f64> = events
        .iter()
        .filter_map(|event| event.duration_ns)
        .map(|ns| ns as f64 / 1_000_000.0)
        .collect();
    let duration_count = durations_ms.len();
    let mut notes = Vec::new();

    if events.is_empty() {
        notes.push("no KMS timing events present".to_owned());
    } else if duration_count == 0 {
        notes.push(
            "only completion or marker events present; request-to-done duration unavailable"
                .to_owned(),
        );
    }

    KmsTimingSummary {
        event_count: events.len(),
        duration_count,
        median_flip_ms: optional_median_ms(&durations_ms),
        p95_flip_ms: optional_percentile_ms(&durations_ms, 0.95),
        p99_flip_ms: optional_percentile_ms(&durations_ms, 0.99),
        max_flip_ms: durations_ms.iter().copied().reduce(f64::max),
        scanout_window_estimate: build_scanout_window_estimate(events),
        notes,
    }
}

fn build_scanout_window_estimate(
    events: &[crate::recorder::KmsFlipEventRecord],
) -> ScanoutWindowEstimate {
    let mut done_ns = events
        .iter()
        .filter_map(|event| event.done_ns)
        .collect::<Vec<_>>();
    done_ns.sort_unstable();
    done_ns.dedup();

    let mut notes = vec![
        "scanout_window_estimate is derived from KMS completion timestamps; it is not photon latency"
            .to_owned(),
        "estimate assumes conventional scanout and excludes monitor processing and pixel response"
            .to_owned(),
    ];

    if done_ns.len() < 2 {
        if !done_ns.is_empty() {
            notes.push(
                "at least two KMS completion timestamps are required to estimate refresh period"
                    .to_owned(),
            );
        }
        return ScanoutWindowEstimate {
            estimate_count: 0,
            notes,
            ..Default::default()
        };
    }

    let refresh_period_ns = median_delta_ns(&done_ns);
    let first_top = done_ns.first().copied();
    let last_top = done_ns.last().copied();

    ScanoutWindowEstimate {
        estimate_count: done_ns.len(),
        refresh_period_ns: Some(refresh_period_ns),
        refresh_period_ms: Some(refresh_period_ns as f64 / 1_000_000.0),
        first_estimated_top_of_screen_visible_ns: first_top,
        first_estimated_bottom_of_screen_visible_ns: first_top
            .and_then(|value| value.checked_add(refresh_period_ns)),
        last_estimated_top_of_screen_visible_ns: last_top,
        last_estimated_bottom_of_screen_visible_ns: last_top
            .and_then(|value| value.checked_add(refresh_period_ns)),
        notes,
    }
}

fn median_delta_ns(sorted_timestamps_ns: &[u64]) -> u64 {
    let mut deltas = sorted_timestamps_ns
        .windows(2)
        .filter_map(|window| window[1].checked_sub(window[0]))
        .filter(|delta| *delta > 0)
        .collect::<Vec<_>>();
    deltas.sort_unstable();
    deltas[deltas.len() / 2]
}

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

    DrmFenceTimingSummary {
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

    CrossGpuFenceSummary {
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

fn drm_fence_is_display_side(event: &crate::recorder::DrmFenceEventRecord) -> bool {
    event.gpu_role.as_deref() == Some("display")
        || event.importer_driver.as_deref() == Some("i915")
        || event.driver.as_deref() == Some("i915")
}

fn drm_fence_driver_path_suggests_cross_gpu(event: &crate::recorder::DrmFenceEventRecord) -> bool {
    let display_side = event.importer_driver.as_deref() == Some("i915")
        || event.driver.as_deref() == Some("i915")
        || event.source == "i915";
    let render_side = event.exporter_driver.as_deref() == Some("amdgpu")
        || event.source == "amdgpu"
        || event.driver.as_deref() == Some("amdgpu");

    display_side && render_side
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

fn drm_fence_wait_near_frame_outlier(
    event: &crate::recorder::DrmFenceEventRecord,
    frame_events: &[crate::recorder::FrameEvent],
) -> bool {
    let median = calculate_median_frametime(frame_events);
    let spikes = identify_frame_spikes(frame_events, median);
    drm_fence_event_near_frame_spikes(event, &spikes)
}

fn drm_fence_event_near_frame_spikes(
    event: &crate::recorder::DrmFenceEventRecord,
    spikes: &[crate::recorder::FrameEvent],
) -> bool {
    spikes
        .iter()
        .any(|frame| elapsed_near(event.elapsed_ms, frame.elapsed_ms, 50))
}

fn drm_fence_wait_near_kms_delay(
    event: &crate::recorder::DrmFenceEventRecord,
    kms_events: &[crate::recorder::KmsFlipEventRecord],
) -> bool {
    drm_fence_event_near_kms_delay(event, kms_events, 1_000_000)
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

fn elapsed_near(left_ms: u64, right_ms: u64, window_ms: u64) -> bool {
    left_ms.abs_diff(right_ms) <= window_ms
}

pub(crate) fn build_wayland_presentation_summary(
    events: &[crate::recorder::WaylandPresentationEventRecord],
    kms_events: &[crate::recorder::KmsFlipEventRecord],
    frame_events: &[crate::recorder::FrameEvent],
) -> WaylandPresentationSummary {
    let durations_ms: Vec<f64> = events
        .iter()
        .filter_map(|event| event.commit_to_present_ns)
        .map(|ns| ns as f64 / 1_000_000.0)
        .collect();
    let zero_copy_count = events
        .iter()
        .filter(|event| event.zero_copy == Some(true))
        .count();
    let presented_count = events
        .iter()
        .filter(|event| event.presented_ns.is_some())
        .count();
    let mut source_counts = BTreeMap::new();
    let mut surface_role_counts = BTreeMap::new();
    for event in events {
        *source_counts.entry(event.source.clone()).or_insert(0) += 1;
        if let Some(role) = &event.surface_role {
            *surface_role_counts.entry(role.clone()).or_insert(0) += 1;
        }
    }
    let mut outputs_seen = events
        .iter()
        .filter_map(|event| event.output_name.clone())
        .collect::<Vec<_>>();
    outputs_seen.sort();
    outputs_seen.dedup();
    let delays_near_frame_outliers = wayland_delays_near_frame_outliers(events, frame_events);
    let delays_near_kms_delays = wayland_delays_near_kms_delays(events, kms_events);
    let frame_outliers =
        identify_frame_spikes(frame_events, calculate_median_frametime(frame_events));
    let compositor_queue_candidate_count = events
        .iter()
        .filter(|event| {
            event.commit_to_present_ns.is_some()
                && event
                    .surface_role
                    .as_deref()
                    .is_some_and(|role| role == "game" || role == "gamescope_output")
                && event.source == "gamescope"
        })
        .filter(|event| {
            frame_outliers
                .iter()
                .any(|frame| elapsed_near(event.elapsed_ms, frame.elapsed_ms, 16))
        })
        .count();

    let mut notes = Vec::new();
    if events.is_empty() {
        notes.push("no Wayland presentation events present".to_owned());
    }
    if events.iter().any(|event| event.source == "self_test") {
        notes.push(
            "self-test presentation events measure stutter's test surface, not the game surface"
                .to_owned(),
        );
    }
    if events.iter().any(|event| event.source == "gamescope") {
        notes.push(
            "Gamescope presentation events are cooperative compositor evidence; they are not visible for arbitrary clients without cooperation"
                .to_owned(),
        );
    }
    if compositor_queue_candidate_count > 0 {
        notes.push(
            "candidate: compositor/presentation queue delay near frame outliers; compare with KMS and scheduler evidence before attributing cause"
                .to_owned(),
        );
    }

    WaylandPresentationSummary {
        event_count: events.len(),
        presented_count,
        discarded_count: events.iter().filter(|event| event.discarded).count(),
        zero_copy_count,
        zero_copy_ratio: (!events.is_empty())
            .then_some(zero_copy_count as f64 / events.len() as f64),
        source_counts,
        surface_role_counts,
        median_commit_to_present_ms: optional_median_ms(&durations_ms),
        p95_commit_to_present_ms: optional_percentile_ms(&durations_ms, 0.95),
        p99_commit_to_present_ms: optional_percentile_ms(&durations_ms, 0.99),
        max_commit_to_present_ms: durations_ms.iter().copied().reduce(f64::max),
        delays_near_frame_outliers,
        delays_near_kms_delays,
        compositor_queue_candidate_count,
        outputs_seen,
        notes,
    }
}

pub(crate) fn build_direct_scanout_summary(
    events: &[crate::recorder::WaylandPresentationEventRecord],
    topology: Option<&crate::display_topology::DisplayTopologySnapshot>,
) -> DirectScanoutSummary {
    let relevant_events = events
        .iter()
        .filter(|event| direct_scanout_relevant_event(event))
        .collect::<Vec<_>>();
    let mut notes = Vec::new();
    let mut evidence = Vec::new();
    let mut blocking_reason_counts = BTreeMap::<String, usize>::new();

    if events.is_empty() {
        notes.push(
            "direct scanout is unknown because no Wayland presentation events are present"
                .to_owned(),
        );
    } else if relevant_events.is_empty() {
        notes.push(
            "direct scanout is unknown because presentation events were not tagged as game or gamescope_output surfaces"
                .to_owned(),
        );
    }

    if topology
        .and_then(|topology| topology.guessed_path.as_ref())
        .and_then(|path| path.is_cross_gpu)
        == Some(true)
    {
        notes.push(
            "display topology appears cross-GPU; direct scanout depends on compositor and buffer-import support"
                .to_owned(),
        );
    }

    let mut direct_count = 0usize;
    let mut composited_count = 0usize;
    let mut zero_copy_known_count = 0usize;
    let mut zero_copy_true_count = 0usize;
    let mut explicit_flag_count = 0usize;

    for event in &relevant_events {
        let normalized_flags = event
            .flags
            .iter()
            .map(|flag| flag.to_ascii_lowercase())
            .collect::<Vec<_>>();
        let has_direct_flag = normalized_flags.iter().any(|flag| {
            matches!(
                flag.as_str(),
                "direct_scanout" | "direct-scanout" | "zero_copy"
            )
        });
        let blocking_flags = normalized_flags
            .iter()
            .filter(|flag| direct_scanout_blocking_flag(flag))
            .cloned()
            .collect::<Vec<_>>();

        if has_direct_flag {
            explicit_flag_count += 1;
        }
        if !blocking_flags.is_empty() {
            explicit_flag_count += blocking_flags.len();
        }

        match event.zero_copy {
            Some(true) => {
                zero_copy_known_count += 1;
                zero_copy_true_count += 1;
            }
            Some(false) => {
                zero_copy_known_count += 1;
                *blocking_reason_counts
                    .entry("zero_copy_false".to_owned())
                    .or_insert(0) += 1;
            }
            None => {}
        }

        for reason in blocking_flags {
            *blocking_reason_counts.entry(reason).or_insert(0) += 1;
        }

        let direct = has_direct_flag || event.zero_copy == Some(true);
        let composited = event.zero_copy == Some(false)
            || normalized_flags
                .iter()
                .any(|flag| direct_scanout_blocking_flag(flag));

        if direct {
            direct_count += 1;
            evidence.push(format!(
                "direct evidence at {}ms from {} role={}",
                event.elapsed_ms,
                event.source,
                event.surface_role.as_deref().unwrap_or("unknown")
            ));
        }
        if composited {
            composited_count += 1;
            evidence.push(format!(
                "composited/copy evidence at {}ms from {} role={}",
                event.elapsed_ms,
                event.source,
                event.surface_role.as_deref().unwrap_or("unknown")
            ));
        }
    }

    let zero_copy_ratio = (zero_copy_known_count > 0)
        .then_some(zero_copy_true_count as f64 / zero_copy_known_count as f64);
    let status = if relevant_events.is_empty() || (direct_count == 0 && composited_count == 0) {
        "unknown"
    } else if direct_count > 0 && composited_count > 0 {
        "mixed"
    } else if direct_count > 0 {
        "yes"
    } else {
        "no"
    }
    .to_owned();
    let confidence = match status.as_str() {
        "unknown" => "missing",
        _ if explicit_flag_count > 0 && relevant_events.len() >= 2 => "high",
        _ if zero_copy_known_count > 0 => "medium",
        _ => "low",
    }
    .to_owned();
    let mut blocking_reasons = blocking_reason_counts
        .into_iter()
        .map(|(reason, count)| format!("{reason}:{count}"))
        .collect::<Vec<_>>();
    blocking_reasons.sort();
    evidence.truncate(8);

    DirectScanoutSummary {
        status,
        confidence,
        zero_copy_ratio,
        direct_scanout_event_count: direct_count,
        composited_event_count: composited_count,
        blocking_reasons,
        evidence,
        notes,
    }
}

pub(crate) fn build_dmabuf_path_summary(
    events: &[crate::recorder::DmaBufEventRecord],
) -> DmaBufPathSummary {
    let mut top_reasons = BTreeMap::new();
    for reason in events.iter().filter_map(|event| event.reason.as_deref()) {
        *top_reasons.entry(reason.to_owned()).or_insert(0) += 1;
    }

    let linear_count = events
        .iter()
        .filter(|event| {
            event.linear == Some(true)
                || event.modifier.as_deref().is_some_and(is_linear_modifier)
                || event
                    .modifier_name
                    .as_deref()
                    .is_some_and(is_linear_modifier)
        })
        .count();
    let scanout_capable_count = events
        .iter()
        .filter(|event| event.scanout_capable == Some(true))
        .count();
    let copy_required_count = events
        .iter()
        .filter(|event| event.copy_required == Some(true))
        .count();
    let modifier_mismatch_count = events
        .iter()
        .filter(|event| {
            event.reason.as_deref().is_some_and(|reason| {
                reason.contains("modifier_mismatch") || reason.contains("modifier mismatch")
            })
        })
        .count();
    let cross_gpu_import_count = events
        .iter()
        .filter(|event| dmabuf_cross_gpu(event))
        .count();

    let mut notes = Vec::new();
    if events.is_empty() {
        notes.push("no DMABUF path events present".to_owned());
    }
    if modifier_mismatch_count > 0 {
        notes.push("DMABUF log reported modifier mismatch candidates".to_owned());
    }
    if copy_required_count > 0 {
        notes.push("DMABUF log reported copy or linearization-required candidates".to_owned());
    }
    if cross_gpu_import_count > 0 {
        notes.push(
            "DMABUF allocation/import evidence crossed GPU or DRM-card boundaries".to_owned(),
        );
    }

    DmaBufPathSummary {
        event_count: events.len(),
        linear_count,
        scanout_capable_count,
        copy_required_count,
        modifier_mismatch_count,
        cross_gpu_import_count,
        top_reasons,
        notes,
    }
}

fn is_linear_modifier(value: &str) -> bool {
    value.eq_ignore_ascii_case("linear")
        || value.eq_ignore_ascii_case("drm_format_modifier_linear")
        || value == "0"
}

fn dmabuf_cross_gpu(event: &crate::recorder::DmaBufEventRecord) -> bool {
    let cross_driver = event
        .allocation_driver
        .as_deref()
        .zip(event.import_driver.as_deref())
        .is_some_and(|(allocation, import)| !allocation.eq_ignore_ascii_case(import));
    let cross_card = event
        .allocation_card
        .as_deref()
        .zip(event.import_card.as_deref())
        .is_some_and(|(allocation, import)| allocation != import);
    cross_driver || cross_card
}

pub(crate) fn build_gpu_engine_activity_summary(
    engine_samples: &[crate::recorder::GpuEngineSample],
    frame_events: &[crate::recorder::FrameEvent],
) -> GpuEngineActivitySummary {
    let frame_outliers =
        identify_frame_spikes(frame_events, calculate_median_frametime(frame_events));
    let mut engine_counts = BTreeMap::new();
    let mut driver_counts = BTreeMap::new();
    let mut max_igpu_render_busy_percent: Option<f64> = None;
    let mut max_igpu_blitter_busy_percent: Option<f64> = None;
    let mut max_amdgpu_gfx_busy_percent: Option<f64> = None;
    let mut igpu_render_activity_near_outliers = 0usize;
    let mut igpu_blitter_activity_near_outliers = 0usize;
    let mut amdgpu_gfx_activity_near_outliers = 0usize;

    for sample in engine_samples {
        *engine_counts.entry(sample.engine.clone()).or_insert(0) += 1;
        if let Some(driver) = sample.driver.as_deref() {
            *driver_counts.entry(driver.to_owned()).or_insert(0) += 1;
        }

        let near_outlier = frame_outliers
            .iter()
            .any(|frame| elapsed_near(sample.elapsed_ms, frame.elapsed_ms, 20));
        let busy = sample.busy_percent.unwrap_or(0.0);
        if !near_outlier || busy < 10.0 {
            continue;
        }

        if gpu_engine_is_igpu(sample) && gpu_engine_is_render(&sample.engine) {
            igpu_render_activity_near_outliers += 1;
            max_igpu_render_busy_percent =
                Some(max_igpu_render_busy_percent.unwrap_or(0.0).max(busy));
        }
        if gpu_engine_is_igpu(sample) && gpu_engine_is_blitter(&sample.engine) {
            igpu_blitter_activity_near_outliers += 1;
            max_igpu_blitter_busy_percent =
                Some(max_igpu_blitter_busy_percent.unwrap_or(0.0).max(busy));
        }
        if gpu_engine_is_amdgpu(sample) && gpu_engine_is_gfx(&sample.engine) {
            amdgpu_gfx_activity_near_outliers += 1;
            max_amdgpu_gfx_busy_percent =
                Some(max_amdgpu_gfx_busy_percent.unwrap_or(0.0).max(busy));
        }
    }

    let mut notes = Vec::new();
    if engine_samples.is_empty() {
        notes.push("no GPU engine samples present".to_owned());
    } else if frame_outliers.is_empty() {
        notes.push("no frame outliers available for GPU engine proximity checks".to_owned());
    }
    if igpu_blitter_activity_near_outliers > 0 || igpu_render_activity_near_outliers > 0 {
        notes.push("iGPU render/blitter activity appeared near frame outliers".to_owned());
    }
    if amdgpu_gfx_activity_near_outliers > 0 {
        notes.push("AMD GFX activity appeared near frame outliers".to_owned());
    }

    GpuEngineActivitySummary {
        sample_count: engine_samples.len(),
        active_sample_count: engine_samples
            .iter()
            .filter(|sample| sample.busy_percent.is_some_and(|busy| busy >= 10.0))
            .count(),
        engine_counts,
        driver_counts,
        igpu_render_activity_near_outliers,
        igpu_blitter_activity_near_outliers,
        amdgpu_gfx_activity_near_outliers,
        max_igpu_render_busy_percent,
        max_igpu_blitter_busy_percent,
        max_amdgpu_gfx_busy_percent,
        notes,
    }
}

fn gpu_engine_is_igpu(sample: &crate::recorder::GpuEngineSample) -> bool {
    matches!(sample.driver.as_deref(), Some("i915" | "xe"))
}

fn gpu_engine_is_amdgpu(sample: &crate::recorder::GpuEngineSample) -> bool {
    sample.driver.as_deref() == Some("amdgpu")
}

fn gpu_engine_is_render(engine: &str) -> bool {
    matches!(engine, "render" | "rcs" | "rcs0" | "3d")
}

fn gpu_engine_is_blitter(engine: &str) -> bool {
    matches!(engine, "blitter" | "bcs" | "bcs0")
}

fn gpu_engine_is_gfx(engine: &str) -> bool {
    matches!(engine, "gfx" | "gfx0" | "render" | "3d")
}

fn direct_scanout_relevant_event(event: &crate::recorder::WaylandPresentationEventRecord) -> bool {
    event
        .surface_role
        .as_deref()
        .is_some_and(|role| matches!(role, "game" | "gamescope_output"))
        || event.source == "gamescope"
}

fn direct_scanout_blocking_flag(flag: &str) -> bool {
    matches!(
        flag,
        "composited"
            | "overlay_active"
            | "scaling"
            | "fractional_scaling"
            | "hdr"
            | "vrr_constraint"
            | "format_modifier_mismatch"
            | "cursor_plane_fallback"
            | "multi_monitor_constraint"
    )
}

fn wayland_delays_near_frame_outliers(
    events: &[crate::recorder::WaylandPresentationEventRecord],
    frame_events: &[crate::recorder::FrameEvent],
) -> usize {
    let frame_outliers =
        identify_frame_spikes(frame_events, calculate_median_frametime(frame_events));
    events
        .iter()
        .filter(|event| event.commit_to_present_ns.is_some())
        .filter(|event| {
            frame_outliers
                .iter()
                .any(|frame| elapsed_near(event.elapsed_ms, frame.elapsed_ms, 16))
        })
        .count()
}

fn wayland_delays_near_kms_delays(
    events: &[crate::recorder::WaylandPresentationEventRecord],
    kms_events: &[crate::recorder::KmsFlipEventRecord],
) -> usize {
    events
        .iter()
        .filter(|event| event.commit_to_present_ns.is_some())
        .filter(|event| {
            kms_events.iter().any(|kms| {
                kms.duration_ns.is_some() && elapsed_near(event.elapsed_ms, kms.elapsed_ms, 16)
            })
        })
        .count()
}

fn optional_median_ms(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| {
        let mut values = values.to_vec();
        median_f64(&mut values)
    })
}

fn optional_percentile_ms(values: &[f64], percentile: f64) -> Option<f64> {
    (!values.is_empty()).then(|| {
        let mut values = values.to_vec();
        percentile_f64(&mut values, percentile)
    })
}
