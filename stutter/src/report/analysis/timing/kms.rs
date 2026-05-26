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
    let evidence_quality = if events.is_empty() {
        missing_evidence("no KMS timing events present")
    } else if duration_count == 0 {
        approximate_evidence(
            "KMS events were present, but request-to-done duration was unavailable",
        )
    } else {
        EvidenceQuality::Direct
    };

    KmsTimingSummary {
        evidence_quality,
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
            evidence_quality: missing_evidence(
                "at least two KMS completion timestamps are required to estimate scanout window",
            ),
            estimate_count: 0,
            notes,
            ..Default::default()
        };
    }

    let refresh_period_ns = median_delta_ns(&done_ns);
    let first_top = done_ns.first().copied();
    let last_top = done_ns.last().copied();

    ScanoutWindowEstimate {
        evidence_quality: EvidenceQuality::Derived,
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
