use std::collections::BTreeMap;

use super::*;

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
    let evidence_quality = if events.is_empty() {
        missing_evidence("no Wayland presentation events present")
    } else if durations_ms.is_empty() {
        approximate_evidence(
            "Wayland presentation events were present, but commit-to-present duration was unavailable",
        )
    } else {
        EvidenceQuality::Direct
    };

    WaylandPresentationSummary {
        evidence_quality,
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
    let evidence_quality = match (status.as_str(), confidence.as_str()) {
        ("unknown", _) => {
            missing_evidence("direct scanout could not be determined from presentation evidence")
        }
        (_, "high") => EvidenceQuality::Direct,
        _ => EvidenceQuality::Derived,
    };
    let mut blocking_reasons = blocking_reason_counts
        .into_iter()
        .map(|(reason, count)| format!("{reason}:{count}"))
        .collect::<Vec<_>>();
    blocking_reasons.sort();
    evidence.truncate(8);

    DirectScanoutSummary {
        evidence_quality,
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
