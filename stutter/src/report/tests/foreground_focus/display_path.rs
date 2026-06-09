use super::*;

#[test]
fn dmabuf_path_summary_counts_modifier_and_cross_gpu_copy_candidates() {
    let events = vec![crate::recorder::DmaBufEventRecord {
        elapsed_ms: 1_000,
        source: "gamescope".to_owned(),
        surface_role: Some("game".to_owned()),
        format: Some("XRGB8888".to_owned()),
        modifier: Some("LINEAR".to_owned()),
        allocation_driver: Some("amdgpu".to_owned()),
        import_driver: Some("i915".to_owned()),
        scanout_capable: Some(false),
        copy_required: Some(true),
        reason: Some("modifier_mismatch".to_owned()),
        confidence: "medium".to_owned(),
        ..Default::default()
    }];

    let summary = crate::report::analysis::build_dmabuf_path_summary(&events);

    assert_eq!(summary.event_count, 1);
    assert_eq!(summary.evidence_quality, EvidenceQuality::Direct);
    assert_eq!(summary.linear_count, 1);
    assert_eq!(summary.modifier_mismatch_count, 1);
    assert_eq!(summary.cross_gpu_import_count, 1);
    assert_eq!(summary.copy_required_count, 1);
    assert_eq!(summary.top_reasons.get("modifier_mismatch"), Some(&1));
}

#[test]
fn gpu_engine_activity_summary_counts_igpu_blitter_near_outlier() {
    let samples = vec![
        crate::recorder::GpuEngineSample {
            elapsed_ms: 1_000,
            driver: Some("i915".to_owned()),
            engine: "bcs0".to_owned(),
            busy_percent: Some(71.0),
            source: "fdinfo".to_owned(),
            confidence: "high".to_owned(),
            ..Default::default()
        },
        crate::recorder::GpuEngineSample {
            elapsed_ms: 1_000,
            driver: Some("amdgpu".to_owned()),
            engine: "gfx".to_owned(),
            busy_percent: Some(60.0),
            source: "hwmon".to_owned(),
            confidence: "medium".to_owned(),
            ..Default::default()
        },
    ];
    let frames = vec![
        crate::recorder::FrameEvent {
            elapsed_ms: 980,
            frametime_ms: 16.0,
        },
        crate::recorder::FrameEvent {
            elapsed_ms: 1_000,
            frametime_ms: 45.0,
        },
    ];

    let summary = crate::report::analysis::build_gpu_engine_activity_summary(&samples, &frames);

    assert_eq!(summary.sample_count, 2);
    assert_eq!(summary.evidence_quality, EvidenceQuality::Direct);
    assert_eq!(summary.active_sample_count, 2);
    assert_eq!(summary.igpu_blitter_activity_near_outliers, 1);
    assert_eq!(summary.amdgpu_gfx_activity_near_outliers, 1);
    assert_eq!(summary.max_igpu_blitter_busy_percent, Some(71.0));
    assert_eq!(summary.max_amdgpu_gfx_busy_percent, Some(60.0));
    assert_eq!(summary.engine_counts.get("bcs0"), Some(&1));
}

#[test]
fn display_path_diagnosis_combines_component_evidence_and_missing_inputs() {
    let mut session = minimal_session_for_report_test();
    session.core.display_path = Some(crate::recorder::DisplayPathMetadata {
        render_driver: Some("amdgpu".to_owned()),
        scanout_driver: Some("i915".to_owned()),
        render_card: Some("card1".to_owned()),
        scanout_card: Some("card0".to_owned()),
        connector: Some("DP-1".to_owned()),
        is_cross_gpu: Some(true),
        ..Default::default()
    });
    let frame_pacing = FramePacingSummary {
        outlier_count: 1,
        ..Default::default()
    };
    let kms = KmsTimingSummary {
        event_count: 2,
        p99_flip_ms: Some(1.4),
        ..Default::default()
    };
    let fence = DrmFenceTimingSummary {
        event_count: 2,
        p99_wait_ms: Some(1.7),
        display_gpu_wait_count: 1,
        waits_near_kms_delays: 1,
        ..Default::default()
    };
    let cross_gpu = CrossGpuFenceSummary {
        candidate_count: 1,
        high_confidence_count: 1,
        ..Default::default()
    };
    let wayland = WaylandPresentationSummary {
        event_count: 2,
        p99_commit_to_present_ms: Some(2.1),
        compositor_queue_candidate_count: 1,
        ..Default::default()
    };
    let direct = DirectScanoutSummary {
        status: "no".to_owned(),
        confidence: "medium".to_owned(),
        composited_event_count: 2,
        ..Default::default()
    };
    let dmabuf = DmaBufPathSummary {
        event_count: 1,
        modifier_mismatch_count: 1,
        copy_required_count: 1,
        ..Default::default()
    };
    let engine = GpuEngineActivitySummary {
        sample_count: 2,
        igpu_blitter_activity_near_outliers: 1,
        max_amdgpu_gfx_busy_percent: Some(60.0),
        ..Default::default()
    };

    let diagnosis = crate::report::analysis::build_display_path_diagnosis_summary(
        &session,
        crate::report::analysis::DisplayPathDiagnosisInputs {
            frame_pacing: &frame_pacing,
            kms_timing: &kms,
            drm_fence_timing: &fence,
            cross_gpu_fence: &cross_gpu,
            wayland_presentation: &wayland,
            direct_scanout: &direct,
            dmabuf_path: &dmabuf,
            gpu_engine_activity: &engine,
        },
    );

    assert_eq!(diagnosis.verdict, "very_likely");
    assert_eq!(diagnosis.confidence, "high");
    assert_eq!(diagnosis.is_cross_gpu, Some(true));
    assert_eq!(diagnosis.fence_component.status, "likely");
    assert_eq!(diagnosis.compositor_component.status, "likely");
    assert!(diagnosis.suspicion_score >= 0.75);
    assert!(
        diagnosis
            .evidence
            .iter()
            .any(|evidence| evidence.contains("iGPU render/blitter"))
    );
    assert!(diagnosis.missing_evidence.is_empty());
}

#[test]
fn direct_scanout_summary_reports_no_and_mixed_from_cooperative_flags() {
    let composited = crate::recorder::WaylandPresentationEventRecord {
        elapsed_ms: 1_000,
        source: "gamescope".to_owned(),
        surface_role: Some("game".to_owned()),
        zero_copy: Some(false),
        flags: vec![
            "format_modifier_mismatch".to_owned(),
            "composited".to_owned(),
        ],
        ..Default::default()
    };
    let direct = crate::recorder::WaylandPresentationEventRecord {
        elapsed_ms: 1_016,
        source: "gamescope".to_owned(),
        surface_role: Some("gamescope_output".to_owned()),
        zero_copy: Some(true),
        flags: vec!["direct_scanout".to_owned()],
        ..Default::default()
    };

    let no_summary = crate::report::analysis::build_direct_scanout_summary(
        std::slice::from_ref(&composited),
        None,
    );
    assert_eq!(no_summary.status, "no");
    assert_eq!(no_summary.evidence_quality, EvidenceQuality::Derived);
    assert_eq!(no_summary.composited_event_count, 1);
    assert!(
        no_summary
            .blocking_reasons
            .iter()
            .any(|reason| reason.starts_with("format_modifier_mismatch:"))
    );

    let mixed_summary =
        crate::report::analysis::build_direct_scanout_summary(&[composited, direct], None);
    assert_eq!(mixed_summary.status, "mixed");
    assert_eq!(mixed_summary.evidence_quality, EvidenceQuality::Direct);
    assert_eq!(mixed_summary.direct_scanout_event_count, 1);
    assert_eq!(mixed_summary.composited_event_count, 1);
}
