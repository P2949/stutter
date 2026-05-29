use super::*;

#[test]
fn report_child_modules_are_not_public_submodules() {
    let source = include_str!("../mod.rs");

    let public_child_modules: Vec<&str> = source
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("pub mod "))
        .collect();

    assert!(
        public_child_modules.is_empty(),
        "report child modules must stay crate-private and be exposed intentionally through api::report: {public_child_modules:?}"
    );
}

#[test]
fn display_timing_summaries_handle_empty_optional_streams() {
    let kms = crate::report::analysis::build_kms_timing_summary(&[]);
    let fence = crate::report::analysis::build_drm_fence_timing_summary(&[], &[], &[]);
    let wayland = crate::report::analysis::build_wayland_presentation_summary(&[], &[], &[]);
    let direct_scanout = crate::report::analysis::build_direct_scanout_summary(&[], None);

    assert_eq!(kms.event_count, 0);
    assert_eq!(
        kms.evidence_quality,
        EvidenceQuality::Missing {
            reason: "no KMS timing events present".to_owned()
        }
    );
    assert_eq!(
        kms.scanout_window_estimate.evidence_quality,
        EvidenceQuality::Missing {
            reason:
                "at least two KMS completion timestamps are required to estimate scanout window"
                    .to_owned()
        }
    );
    assert_eq!(kms.notes, vec!["no KMS timing events present"]);
    assert_eq!(fence.event_count, 0);
    assert_eq!(
        fence.evidence_quality,
        EvidenceQuality::Missing {
            reason: "no DRM fence events present".to_owned()
        }
    );
    assert_eq!(fence.confidence, "missing");
    assert_eq!(wayland.event_count, 0);
    assert_eq!(
        wayland.evidence_quality,
        EvidenceQuality::Missing {
            reason: "no Wayland presentation events present".to_owned()
        }
    );
    assert_eq!(
        wayland.notes,
        vec!["no Wayland presentation events present"]
    );
    assert_eq!(direct_scanout.status, "unknown");
    assert_eq!(
        direct_scanout.evidence_quality,
        EvidenceQuality::Missing {
            reason: "direct scanout could not be determined from presentation evidence".to_owned()
        }
    );
    assert_eq!(direct_scanout.confidence, "missing");
}

#[test]
fn display_timing_summaries_compute_basic_percentiles() {
    let kms_events = vec![
        crate::recorder::KmsFlipEventRecord {
            elapsed_ms: 1_000,
            duration_ns: Some(2_000_000),
            done_ns: Some(1_000_000_000),
            ..Default::default()
        },
        crate::recorder::KmsFlipEventRecord {
            elapsed_ms: 1_016,
            duration_ns: Some(4_000_000),
            done_ns: Some(1_016_666_667),
            ..Default::default()
        },
    ];
    let fence_events = vec![crate::recorder::DrmFenceEventRecord {
        elapsed_ms: 1_001,
        duration_ns: Some(3_000_000),
        source: "i915".to_owned(),
        gpu_role: Some("display".to_owned()),
        importer_driver: Some("i915".to_owned()),
        exporter_driver: Some("amdgpu".to_owned()),
        context: Some(7),
        seqno: Some(9),
        signal_ns: Some(900_000),
        wait_start_ns: Some(1_000_000),
        wait_done_ns: Some(4_000_000),
        correlation_basis: "context_seqno".to_owned(),
        confidence: "high".to_owned(),
        ..Default::default()
    }];
    let frame_events = vec![
        crate::recorder::FrameEvent {
            elapsed_ms: 980,
            frametime_ms: 16.0,
        },
        crate::recorder::FrameEvent {
            elapsed_ms: 1_000,
            frametime_ms: 40.0,
        },
    ];
    let wayland_events = vec![crate::recorder::WaylandPresentationEventRecord {
        elapsed_ms: 1_002,
        source: "gamescope".to_owned(),
        surface_role: Some("game".to_owned()),
        commit_to_present_ns: Some(4_000_000),
        presented_ns: Some(10),
        zero_copy: Some(true),
        output_name: Some("DP-1".to_owned()),
        ..Default::default()
    }];

    let kms = crate::report::analysis::build_kms_timing_summary(&kms_events);
    let fence = crate::report::analysis::build_drm_fence_timing_summary(
        &fence_events,
        &kms_events,
        &frame_events,
    );
    let cross_gpu_fence = crate::report::analysis::build_cross_gpu_fence_summary(
        &fence_events,
        &kms_events,
        &frame_events,
        None,
    );
    let wayland = crate::report::analysis::build_wayland_presentation_summary(
        &wayland_events,
        &kms_events,
        &frame_events,
    );
    let direct_scanout =
        crate::report::analysis::build_direct_scanout_summary(&wayland_events, None);

    assert_eq!(kms.duration_count, 2);
    assert_eq!(kms.evidence_quality, EvidenceQuality::Direct);
    assert_eq!(kms.median_flip_ms, Some(3.0));
    assert_eq!(
        kms.scanout_window_estimate.refresh_period_ns,
        Some(16_666_667)
    );
    assert_eq!(
        kms.scanout_window_estimate.evidence_quality,
        EvidenceQuality::Derived
    );
    assert_eq!(
        kms.scanout_window_estimate
            .first_estimated_top_of_screen_visible_ns,
        Some(1_000_000_000)
    );
    assert!(
        kms.scanout_window_estimate
            .notes
            .iter()
            .any(|note| note.contains("not photon latency"))
    );
    assert_eq!(fence.wait_interval_count, 1);
    assert_eq!(fence.evidence_quality, EvidenceQuality::Direct);
    assert_eq!(fence.max_wait_ms, Some(3.0));
    assert_eq!(fence.display_gpu_wait_count, 1);
    assert_eq!(fence.cross_gpu_candidate_count, 1);
    assert_eq!(fence.waits_near_frame_outliers, 1);
    assert_eq!(fence.waits_near_kms_delays, 1);
    assert_eq!(fence.top_waits.len(), 1);
    assert_eq!(cross_gpu_fence.candidate_count, 1);
    assert_eq!(cross_gpu_fence.evidence_quality, EvidenceQuality::Derived);
    assert_eq!(cross_gpu_fence.high_confidence_count, 1);
    assert_eq!(cross_gpu_fence.confidence, "high");
    assert_eq!(cross_gpu_fence.display_side_wait_count, 1);
    assert_eq!(cross_gpu_fence.waits_near_frame_outliers, 1);
    assert_eq!(cross_gpu_fence.waits_near_kms_delays, 1);
    assert_eq!(cross_gpu_fence.top_candidates[0].signal_ns, Some(900_000));
    assert_eq!(wayland.presented_count, 1);
    assert_eq!(wayland.evidence_quality, EvidenceQuality::Direct);
    assert_eq!(wayland.zero_copy_ratio, Some(1.0));
    assert_eq!(wayland.p99_commit_to_present_ms, Some(4.0));
    assert_eq!(wayland.outputs_seen, vec!["DP-1"]);
    assert_eq!(wayland.source_counts.get("gamescope"), Some(&1));
    assert_eq!(wayland.surface_role_counts.get("game"), Some(&1));
    assert_eq!(wayland.delays_near_frame_outliers, 1);
    assert_eq!(wayland.delays_near_kms_delays, 1);
    assert_eq!(wayland.compositor_queue_candidate_count, 1);
    assert_eq!(direct_scanout.status, "yes");
    assert_eq!(direct_scanout.evidence_quality, EvidenceQuality::Derived);
    assert_eq!(direct_scanout.zero_copy_ratio, Some(1.0));
    assert_eq!(direct_scanout.direct_scanout_event_count, 1);
}
