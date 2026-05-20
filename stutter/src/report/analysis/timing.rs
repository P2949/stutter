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
        .filter(|event| {
            spikes
                .iter()
                .any(|frame| elapsed_near(event.elapsed_ms, frame.elapsed_ms, 50))
        })
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
        .filter(|event| {
            kms_events.iter().any(|kms| {
                kms.duration_ns
                    .is_some_and(|duration| duration >= threshold_ns)
                    && elapsed_near(event.elapsed_ms, kms.elapsed_ms, 50)
            })
        })
        .count()
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
