//! Foreground, focus, event warning, timing, and wake-graph report tests.

use super::*;
use crate::sched_state::classify_switch_prev_state;

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
    assert_eq!(kms.notes, vec!["no KMS timing events present"]);
    assert_eq!(fence.event_count, 0);
    assert_eq!(fence.confidence, "missing");
    assert_eq!(wayland.event_count, 0);
    assert_eq!(
        wayland.notes,
        vec!["no Wayland presentation events present"]
    );
    assert_eq!(direct_scanout.status, "unknown");
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
    assert_eq!(kms.median_flip_ms, Some(3.0));
    assert_eq!(
        kms.scanout_window_estimate.refresh_period_ns,
        Some(16_666_667)
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
    assert_eq!(fence.max_wait_ms, Some(3.0));
    assert_eq!(fence.display_gpu_wait_count, 1);
    assert_eq!(fence.cross_gpu_candidate_count, 1);
    assert_eq!(fence.waits_near_frame_outliers, 1);
    assert_eq!(fence.waits_near_kms_delays, 1);
    assert_eq!(fence.top_waits.len(), 1);
    assert_eq!(cross_gpu_fence.candidate_count, 1);
    assert_eq!(cross_gpu_fence.high_confidence_count, 1);
    assert_eq!(cross_gpu_fence.confidence, "high");
    assert_eq!(cross_gpu_fence.display_side_wait_count, 1);
    assert_eq!(cross_gpu_fence.waits_near_frame_outliers, 1);
    assert_eq!(cross_gpu_fence.waits_near_kms_delays, 1);
    assert_eq!(cross_gpu_fence.top_candidates[0].signal_ns, Some(900_000));
    assert_eq!(wayland.presented_count, 1);
    assert_eq!(wayland.zero_copy_ratio, Some(1.0));
    assert_eq!(wayland.p99_commit_to_present_ms, Some(4.0));
    assert_eq!(wayland.outputs_seen, vec!["DP-1"]);
    assert_eq!(wayland.source_counts.get("gamescope"), Some(&1));
    assert_eq!(wayland.surface_role_counts.get("game"), Some(&1));
    assert_eq!(wayland.delays_near_frame_outliers, 1);
    assert_eq!(wayland.delays_near_kms_delays, 1);
    assert_eq!(wayland.compositor_queue_candidate_count, 1);
    assert_eq!(direct_scanout.status, "yes");
    assert_eq!(direct_scanout.zero_copy_ratio, Some(1.0));
    assert_eq!(direct_scanout.direct_scanout_event_count, 1);
}

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
    assert_eq!(mixed_summary.direct_scanout_event_count, 1);
    assert_eq!(mixed_summary.composited_event_count, 1);
}

#[test]
fn report_includes_foreground_summary_when_events_present() {
    let mut session = minimal_session_for_report_test();
    session.config.foreground_window = true;
    session.config.foreground_source = "sway".to_owned();
    session.core.foreground_event_count = 1;

    let summary = foreground_report_summary(
        &session,
        &[foreground_event(
            1_000,
            Some(4242),
            Some("steam_app_379430"),
            Some("steam_app_379430"),
            None,
            Some("gaming"),
            0.95,
        )],
    );

    assert!(summary.enabled);
    assert_eq!(summary.source.as_deref(), Some("sway"));
    assert_eq!(summary.final_pid, Some(4242));
    assert_eq!(summary.final_app_id.as_deref(), Some("steam_app_379430"));
    assert_eq!(summary.final_class.as_deref(), Some("steam_app_379430"));
    assert_eq!(summary.final_window_id.as_deref(), Some("7"));
    assert_eq!(summary.final_workspace.as_deref(), Some("gaming"));
    assert_eq!(summary.event_count, 1);
    assert_eq!(summary.confidence, Some(0.95));
}

#[test]
fn report_redacts_missing_title_cleanly() {
    let summary = ForegroundReportSummary {
        enabled: true,
        source: Some("sway".to_owned()),
        final_pid: Some(4242),
        final_app_id: Some("steam_app_379430".to_owned()),
        final_class: Some("steam_app_379430".to_owned()),
        final_title: None,
        final_window_id: Some("7".to_owned()),
        final_workspace: Some("gaming".to_owned()),
        event_count: 1,
        confidence: Some(0.95),
        provider_status: Some("available".to_owned()),
        stale_ms: None,
        reasons: Vec::new(),
    };

    let text = render_foreground_summary_text(&summary);

    assert!(text.contains("Foreground window:"));
    assert!(text.contains("title: redacted (pass --foreground-include-title to record it)"));
    assert!(!text.contains("Private"));
}

#[test]
fn spike_cluster_gets_nearest_foreground_context() {
    let mut clusters = vec![cluster_at(1_500)];
    let events = vec![
        foreground_event(
            1_000,
            Some(1111),
            Some("steamwebhelper"),
            Some("steamwebhelper"),
            None,
            None,
            0.60,
        ),
        foreground_event(
            1_400,
            Some(4242),
            Some("steam_app_379430"),
            Some("steam_app_379430"),
            None,
            Some("gaming"),
            0.95,
        ),
        foreground_event(
            1_600,
            Some(9999),
            Some("future"),
            Some("future"),
            None,
            None,
            0.95,
        ),
    ];

    annotate_clusters_with_foreground(&mut clusters, &events, 1_000);

    assert_eq!(clusters[0].foreground_pid, Some(4242));
    assert_eq!(
        clusters[0].foreground_app_id.as_deref(),
        Some("steam_app_379430")
    );
    assert_eq!(
        clusters[0].foreground_class.as_deref(),
        Some("steam_app_379430")
    );
    assert_eq!(clusters[0].foreground_confidence, Some(0.95));
}

#[test]
fn foreground_report_summary_uses_final_event_and_redacted_title() {
    let mut session = minimal_session_for_report_test();
    session.config.foreground_window = true;
    session.config.foreground_source = "sway".to_owned();
    session.core.foreground_event_count = 2;
    session.core.foreground_source = Some("sway".to_owned());
    session.core.final_foreground_pid = Some(12345);
    session.core.final_foreground_app_id = Some("steam_app_379430".to_owned());
    session.core.final_foreground_class = Some("steam_app_379430".to_owned());

    let events = vec![
        foreground_event(
            100,
            Some(1000),
            Some("steam"),
            Some("Steam"),
            None,
            Some("gaming"),
            0.90,
        ),
        foreground_event(
            200,
            Some(12345),
            Some("steam_app_379430"),
            Some("steam_app_379430"),
            None,
            Some("gaming"),
            0.95,
        ),
    ];

    let summary = foreground_report_summary(&session, &events);

    assert!(summary.enabled);
    assert_eq!(summary.source.as_deref(), Some("sway"));
    assert_eq!(summary.final_pid, Some(12345));
    assert_eq!(summary.final_app_id.as_deref(), Some("steam_app_379430"));
    assert_eq!(summary.final_class.as_deref(), Some("steam_app_379430"));
    assert_eq!(summary.final_title, None);
    assert_eq!(summary.final_window_id.as_deref(), Some("7"));
    assert_eq!(summary.final_workspace.as_deref(), Some("gaming"));
    assert_eq!(summary.event_count, 2);
    assert_eq!(summary.confidence, Some(0.95));
    assert_eq!(summary.provider_status.as_deref(), Some("available"));
}

#[test]
fn render_foreground_summary_text_mentions_redacted_title() {
    let summary = ForegroundReportSummary {
        enabled: true,
        source: Some("sway".to_owned()),
        final_pid: Some(12345),
        final_app_id: Some("steam_app_379430".to_owned()),
        final_class: Some("steam_app_379430".to_owned()),
        final_title: None,
        final_window_id: Some("7".to_owned()),
        final_workspace: Some("gaming".to_owned()),
        event_count: 7,
        confidence: Some(0.95),
        provider_status: Some("available".to_owned()),
        stale_ms: None,
        reasons: vec!["focused Sway node from swaymsg get_tree".to_owned()],
    };

    let text = render_foreground_summary_text(&summary);

    assert!(text.contains("Foreground window:"));
    assert!(text.contains("  source: sway"));
    assert!(text.contains("  final pid: 12345"));
    assert!(text.contains("  app_id/class: steam_app_379430"));
    assert!(text.contains("  window_id: 7"));
    assert!(text.contains("  workspace: gaming"));
    assert!(text.contains("  confidence: 0.95"));
    assert!(text.contains("  stale: no"));
    assert!(text.contains("  events: 7"));
    assert!(text.contains("  title: redacted (pass --foreground-include-title to record it)"));
}

#[test]
fn foreground_for_cluster_uses_nearest_event_at_or_before_cluster_time() {
    let cluster = cluster_at(1_500);
    let events = vec![
        foreground_event(500, Some(1), Some("old"), Some("Old"), None, None, 0.50),
        foreground_event(
            1_200,
            Some(2),
            Some("game"),
            Some("Game"),
            None,
            Some("gaming"),
            0.95,
        ),
        foreground_event(
            1_600,
            Some(3),
            Some("future"),
            Some("Future"),
            None,
            None,
            0.95,
        ),
    ];

    let selected = foreground_for_cluster(&cluster, &events, 1_000).unwrap();

    assert_eq!(selected.pid, Some(2));
    assert_eq!(selected.app_id.as_deref(), Some("game"));
}

#[test]
fn foreground_for_cluster_respects_max_stale_ms() {
    let cluster = cluster_at(2_000);
    let events = vec![foreground_event(
        500,
        Some(1),
        Some("old"),
        Some("Old"),
        None,
        None,
        0.50,
    )];

    assert!(foreground_for_cluster(&cluster, &events, 1_000).is_none());
}

#[test]
fn annotate_clusters_with_foreground_sets_cluster_fields() {
    let mut clusters = vec![cluster_at(1_500)];
    let events = vec![foreground_event(
        1_200,
        Some(12345),
        Some("steam_app_379430"),
        Some("steam_app_379430"),
        None,
        Some("gaming"),
        0.95,
    )];

    annotate_clusters_with_foreground(&mut clusters, &events, 1_000);

    assert_eq!(clusters[0].foreground_pid, Some(12345));
    assert_eq!(
        clusters[0].foreground_app_id.as_deref(),
        Some("steam_app_379430")
    );
    assert_eq!(
        clusters[0].foreground_class.as_deref(),
        Some("steam_app_379430")
    );
    assert_eq!(clusters[0].foreground_confidence, Some(0.95));
}

#[test]
fn report_analysis_json_contains_foreground_summary() {
    let mut session = minimal_session_for_report_test();
    session.config.foreground_window = true;
    session.config.foreground_source = "sway".to_owned();
    session.core.foreground_event_count = 1;
    let summary = foreground_report_summary(
        &session,
        &[foreground_event(
            100,
            Some(12345),
            Some("steam_app_379430"),
            Some("steam_app_379430"),
            None,
            Some("gaming"),
            0.95,
        )],
    );

    let json = serde_json::to_string(&summary).unwrap();

    assert!(json.contains("\"enabled\":true"));
    assert!(json.contains("\"source\":\"sway\""));
    assert!(json.contains("\"final_pid\":12345"));
    assert!(json.contains("\"event_count\":1"));
}
use crate::{
    autotune::state::SituationKind,
    recorder::{FocusEvent, RecordedConfig, SessionMetadataCore},
};

#[test]
fn focus_report_summary_prefers_latest_changed_focus_event() {
    let session = SessionFile {
        core: SessionMetadataCore {
            focus_mode: Some("auto-focus".to_owned()),
            final_focus_kind: Some("Browser".to_owned()),
            focus_switch_count: 2,
            ..Default::default()
        },
        config: RecordedConfig {
            auto_focus: true,
            ..Default::default()
        },
        ..Default::default()
    };
    let events = vec![
        FocusEvent {
            elapsed_ms: 100,
            action: "changed".to_owned(),
            kind: Some("Browser".to_owned()),
            confidence: 0.62,
            situation: Some(SituationKind::BrowserFocused),
            root_pids: vec![111.into()],
            member_pids: vec![111.into(), 112.into()],
            reasons: vec!["browser parent with active renderer".to_owned()],
            ..Default::default()
        },
        FocusEvent {
            elapsed_ms: 200,
            action: "changed".to_owned(),
            kind: Some("Compile".to_owned()),
            confidence: 0.87,
            score: 0.91,
            situation: Some(SituationKind::CompileLoad),
            root_pids: vec![1234.into()],
            member_pids: vec![1234.into(), 1235.into()],
            reasons: vec![
                "cargo root with 14 active compiler descendants".to_owned(),
                "linker/write IO evidence observed".to_owned(),
            ],
            ..Default::default()
        },
    ];

    let summary = focus_report_summary(&session, &events);

    assert_eq!(summary.mode.as_deref(), Some("auto-focus"));
    assert_eq!(summary.final_focus.as_deref(), Some("Compile"));
    assert_eq!(summary.situation.as_deref(), Some("CompileLoad"));
    assert_eq!(summary.confidence, Some(0.87));
    assert_eq!(summary.score, Some(0.91));
    assert_eq!(summary.roots, vec![1234]);
    assert_eq!(summary.member_pids, vec![1234, 1235]);
    assert_eq!(summary.focus_switches, 2);
    assert_eq!(summary.reasons.len(), 2);
}

#[test]
fn render_focus_summary_text_includes_visible_reasons() {
    let summary = FocusReportSummary {
        mode: Some("auto-focus".to_owned()),
        final_focus: Some("Compile".to_owned()),
        display_name: Some("cargo build".to_owned()),
        situation: Some("CompileLoad".to_owned()),
        confidence: Some(0.87),
        score: Some(0.91),
        roots: vec![1234],
        member_pids: vec![1234, 1235],
        focus_switches: 2,
        reasons: vec![
            "cargo root with 14 active compiler descendants".to_owned(),
            "CPU delta 780% over 1s".to_owned(),
        ],
    };

    let text = render_focus_summary_text(&summary);

    assert!(text.contains("Auto focus:"));
    assert!(text.contains("  mode: auto-focus"));
    assert!(text.contains("  final focus: Compile"));
    assert!(text.contains("  situation: CompileLoad"));
    assert!(text.contains("  confidence: 0.87"));
    assert!(text.contains("  roots: [1234]"));
    assert!(text.contains("  focus switches: 2"));
    assert!(text.contains("    - cargo root with 14 active compiler descendants"));
    assert!(text.contains("    - CPU delta 780% over 1s"));
}

#[test]
fn event_stream_warning_is_absent_without_errors() {
    assert!(event_stream_warning(0, None).is_none());
}

#[test]
fn event_stream_warning_includes_count_and_first_error() {
    let warning = event_stream_warning(2, Some("spike_events: No space left on device")).unwrap();

    assert!(warning.contains("2 write error"));
    assert!(warning.contains("event artifact files may be incomplete"));
    assert!(warning.contains("spike_events: No space left on device"));
}

#[test]
fn event_stream_warning_handles_missing_first_error() {
    let warning = event_stream_warning(1, None).unwrap();

    assert!(warning.contains("1 write error"));
    assert!(warning.contains("first error was not recorded"));
}

#[test]
fn classify_switch_prev_state_zero_is_running() {
    assert_eq!(classify_switch_prev_state(0), "running");
}

#[test]
fn classify_switch_prev_state_interruptible() {
    assert_eq!(classify_switch_prev_state(1), "interruptible_sleep");
}

#[test]
fn classify_switch_prev_state_uninterruptible() {
    assert_eq!(classify_switch_prev_state(2), "uninterruptible_sleep");
}

#[test]
fn classify_switch_prev_state_other_sleep() {
    assert_eq!(classify_switch_prev_state(8), "traced");
}

#[test]
fn classify_switch_prev_state_interruptible_wins_when_multiple_bits_set() {
    assert_eq!(classify_switch_prev_state(3), "interruptible_sleep");
}

#[test]
fn test_build_wake_graph_grouping_and_sorting() {
    let points = vec![
        SpikePoint {
            task: 101,
            comm: "wakee1".to_owned(),
            waker_tid: 201,
            waker_comm: "waker1".to_owned(),
            latency_ns: 1000,
            ..SpikePoint::default()
        },
        SpikePoint {
            task: 101,
            comm: "wakee1".to_owned(),
            waker_tid: 201,
            waker_comm: "waker1".to_owned(),
            latency_ns: 2000,
            ..SpikePoint::default()
        },
        SpikePoint {
            task: 102,
            comm: "wakee2".to_owned(),
            waker_tid: 201,
            waker_comm: "waker1".to_owned(),
            latency_ns: 500,
            ..SpikePoint::default()
        },
        SpikePoint {
            task: 101,
            comm: "wakee1".to_owned(),
            waker_tid: 202,
            waker_comm: "waker2".to_owned(),
            latency_ns: 5000,
            ..SpikePoint::default()
        },
    ];

    let graph = build_wake_graph(&points);

    // Should have 3 edges:
    // 1. (201, waker1) -> (101, wakee1) count=2 max_lat=2000
    // 2. (202, waker2) -> (101, wakee1) count=1 max_lat=5000
    // 3. (201, waker1) -> (102, wakee2) count=1 max_lat=500

    // Sorted by count desc, then max_lat desc
    assert_eq!(graph.len(), 3);

    assert_eq!(graph[0].waker_tid, 201);
    assert_eq!(graph[0].wakee_tid, 101);
    assert_eq!(graph[0].count, 2);
    assert_eq!(graph[0].max_latency_ns, 2000);

    assert_eq!(graph[1].waker_tid, 202);
    assert_eq!(graph[1].count, 1);
    assert_eq!(graph[1].max_latency_ns, 5000);

    assert_eq!(graph[2].waker_tid, 201);
    assert_eq!(graph[2].wakee_tid, 102);
    assert_eq!(graph[2].count, 1);
    assert_eq!(graph[2].max_latency_ns, 500);
}
