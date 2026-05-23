//! Data-quality, text rendering, pressure timeline, and runtime-slice report tests.

use super::*;

#[test]
fn data_quality_is_high_for_clean_minimal_session() {
    let session = minimal_session_for_report_test();
    let validation = crate::session_io::RunValidationReport::default();

    let summary = data_quality_summary(&session, &validation);

    assert_eq!(summary.level, DataQualityLevel::High);
    assert!(
        summary
            .reasons
            .iter()
            .any(|reason| reason.contains("no data-quality problems"))
    );
}

#[test]
fn report_model_types_remain_available_through_legacy_reexports() {
    assert_eq!(DataQualityLevel::High, model::DataQualityLevel::High);
    assert!(!FocusReportSummary::default().is_visible());
    assert!(!model::FocusReportSummary::default().is_visible());
    assert!(!ForegroundReportSummary::default().is_visible());
    assert!(!model::ForegroundReportSummary::default().is_visible());
}

#[test]
fn analysis_from_report_input_model_preserves_existing_summary_behavior() {
    let session = minimal_session_for_report_test();
    let artifacts = session_io::RunArtifacts {
        session: session.clone(),
        validation: crate::session_io::RunValidationReport::default(),
        ..Default::default()
    };

    let result =
        build_report_analysis_from_input(ReportInputModel::from_artifacts(artifacts), 10, 5, None)
            .unwrap();

    assert_eq!(
        result.analysis.session.core.duration_ms,
        session.core.duration_ms
    );
    assert_eq!(result.analysis.data_quality.level, DataQualityLevel::High);
    assert!(
        result
            .analysis
            .data_quality
            .reasons
            .iter()
            .any(|reason| reason.contains("no data-quality problems"))
    );
}

#[test]
fn renderers_accept_report_model_values_without_loading_or_analysis() {
    use super::render::{
        json::render_json_pretty,
        text::{TextReportRenderInput, render_report},
    };

    let session = minimal_session_for_report_test();
    let focus = FocusReportSummary::default();
    let foreground = ForegroundReportSummary::default();
    let artifacts = session_io::RunArtifacts {
        session: session.clone(),
        validation: crate::session_io::RunValidationReport::default(),
        ..Default::default()
    };
    let data_quality = data_quality_summary(&session, &artifacts.validation);
    let pressure_timeline = PressureTimelineSummary::default();
    let runtime_slices = RuntimeSliceAnalysisSummary::default();

    let correlation_sections = TextReportCorrelationSections::new();
    let rendered = render_report(TextReportRenderInput {
        path: Path::new("runs/example"),
        session: &session,
        cluster_analysis: &SpikeClusterAnalysis {
            source: SpikeClusterSource::TopSpikesFallback,
            source_count: 0,
            clusters: Vec::new(),
        },
        frame_diagnoses: &[],
        data_quality: &data_quality,
        pressure_timeline: &pressure_timeline,
        runtime_slice_summary: &runtime_slices,
        correlation_sections: &correlation_sections,
        focus_summary: &focus,
        foreground_summary: &foreground,
        display_path_diagnosis: None,
        top: 10,
        cluster_window_ms: 5,
        filter_class: None,
    });

    assert!(rendered.contains("file: runs/example"));
    assert!(
        render_json_pretty(&session)
            .unwrap()
            .contains("\"schema_version\"")
    );
}

#[test]
fn report_text_rendering_matches_snapshot_fixture() {
    let session = minimal_session_for_report_test();
    let validation = crate::session_io::RunValidationReport::default();
    let data_quality = data_quality_summary(&session, &validation);
    let cluster_analysis = SpikeClusterAnalysis {
        source: SpikeClusterSource::TopSpikesFallback,
        source_count: 0,
        clusters: Vec::new(),
    };
    let pressure_timeline = PressureTimelineSummary::default();
    let runtime_slices = RuntimeSliceAnalysisSummary::default();
    let correlation_sections = TextReportCorrelationSections::new();

    let rendered = render_report(TextReportRenderInput {
        path: Path::new("snapshot/session.json"),
        session: &session,
        cluster_analysis: &cluster_analysis,
        frame_diagnoses: &[],
        data_quality: &data_quality,
        pressure_timeline: &pressure_timeline,
        runtime_slice_summary: &runtime_slices,
        correlation_sections: &correlation_sections,
        focus_summary: &FocusReportSummary::default(),
        foreground_summary: &ForegroundReportSummary::default(),
        display_path_diagnosis: None,
        top: 10,
        cluster_window_ms: 5,
        filter_class: None,
    });

    assert_eq!(
        rendered.trim_end(),
        include_str!("../snapshots/text_report_minimal.snap").trim_end()
    );
}

#[test]
fn data_quality_is_low_for_validation_errors() {
    let session = minimal_session_for_report_test();
    let validation = crate::session_io::RunValidationReport {
        errors: vec!["bad session".to_owned()],
        ..Default::default()
    };

    let summary = data_quality_summary(&session, &validation);

    assert_eq!(summary.level, DataQualityLevel::Low);
    assert!(
        summary
            .reasons
            .iter()
            .any(|reason| reason.contains("validation errors"))
    );
}

#[test]
fn data_quality_warns_on_degraded_drm_fence_evidence() {
    let session = minimal_session_for_report_test();
    let validation = crate::session_io::RunValidationReport {
        warnings: vec![
            "DRM fence events contain only signal/marker evidence; wait duration attribution is low confidence"
                .to_owned(),
        ],
        ..Default::default()
    };

    let summary = data_quality_summary(&session, &validation);

    assert_eq!(summary.level, DataQualityLevel::Medium);
    assert!(
        summary
            .reasons
            .iter()
            .any(|reason| reason.contains("DRM fence latency evidence"))
    );
}

#[test]
fn data_quality_warns_on_truncated_spikes() {
    let mut session = minimal_session_for_report_test();
    session.core.spike_events_truncated = true;
    session.core.spike_events_retained_count = 500_000;
    session.core.spike_events_dropped_count = 1;

    let validation = crate::session_io::RunValidationReport::default();

    let summary = data_quality_summary(&session, &validation);

    assert_eq!(summary.level, DataQualityLevel::Medium);
    assert!(
        summary
            .reasons
            .iter()
            .any(|reason| reason.contains("spike event stream was truncated"))
    );
}

#[test]
fn data_quality_reports_block_fallback_key_collisions_specifically() {
    let mut session = minimal_session_for_report_test();
    session.core.block_io_correlation_basis = "dev+sector".to_owned();
    session.core.block_io_correlation_confidence = "medium".to_owned();
    session.core.drop_counters.block_fallback_key_collisions = 3;

    let validation = crate::session_io::RunValidationReport::default();

    let summary = data_quality_summary(&session, &validation);

    assert_eq!(summary.level, DataQualityLevel::Medium);
    assert!(summary.reasons.iter().any(|reason| {
        reason.contains("block I/O fallback key collisions detected")
            && reason.contains("3")
            && reason.contains("coverage may be incomplete")
    }));
}

#[test]
fn data_quality_warns_on_replaced_wakeup_entries() {
    let mut session = minimal_session_for_report_test();
    session.core.drop_counters.wakeup_data_replaced_entries = 2;

    let summary =
        data_quality_summary(&session, &crate::session_io::RunValidationReport::default());

    assert_eq!(summary.level, DataQualityLevel::Medium);
    assert!(summary.reasons.iter().any(|reason| {
        reason.contains("wakeup timestamp records were replaced") && reason.contains("2")
    }));
    assert!(
        !summary
            .reasons
            .iter()
            .any(|reason| reason.contains("no data-quality problems"))
    );
}

#[test]
fn data_quality_warns_on_consumed_wakeup_read_failures() {
    let mut session = minimal_session_for_report_test();
    session.core.drop_counters.wakeup_data_consumed_read_failed = 3;

    let summary =
        data_quality_summary(&session, &crate::session_io::RunValidationReport::default());

    assert_eq!(summary.level, DataQualityLevel::Medium);
    assert!(summary.reasons.iter().any(|reason| {
        reason.contains("sched_switch tracepoint reads failed") && reason.contains("3")
    }));
}

#[test]
fn data_quality_warns_on_untracked_cpu_accounting() {
    let mut session = minimal_session_for_report_test();
    session.core.drop_counters.cpu_accounting_untracked = 4;

    let summary =
        data_quality_summary(&session, &crate::session_io::RunValidationReport::default());

    assert_eq!(summary.level, DataQualityLevel::Medium);
    assert!(summary.reasons.iter().any(|reason| {
        reason.contains("CPU accounting skipped 4 events") && reason.contains("runnable-depth")
    }));
}

#[test]
fn data_quality_warns_on_unavailable_requested_block_io_correlation() {
    let mut session = minimal_session_for_report_test();
    session.config.block_io = true;
    session.core.block_io_correlation_basis = "unavailable".to_owned();
    session.core.block_io_correlation_confidence = "none".to_owned();

    let summary =
        data_quality_summary(&session, &crate::session_io::RunValidationReport::default());

    assert_eq!(summary.level, DataQualityLevel::Medium);
    assert_eq!(summary.block_io_correlation_basis, "unavailable");
    assert_eq!(summary.block_io_correlation_confidence, "none");
    assert!(
        summary
            .block_io_correlation_warning
            .as_deref()
            .is_some_and(|warning| warning.contains("unavailable"))
    );
    assert!(
        summary
            .reasons
            .iter()
            .any(|reason| reason.contains("Block I/O correlation is unavailable"))
    );
}

#[test]
fn data_quality_warns_on_unverified_native_cgroup_filter() {
    let mut session = minimal_session_for_report_test();
    session.core.native_cgroup_filter =
        crate::ebpf_loader::NativeCgroupFilterStatus::unverified_directory_inode(
            "/sys/fs/cgroup/game.slice".to_owned(),
            42,
        );

    let summary =
        data_quality_summary(&session, &crate::session_io::RunValidationReport::default());

    assert_eq!(summary.level, DataQualityLevel::Medium);
    assert!(summary.reasons.iter().any(|reason| {
        reason.contains("native cgroup filtering") && reason.contains("not runtime-verified")
    }));
}

#[test]
fn data_quality_warns_on_missing_optional_artifacts() {
    let session = minimal_session_for_report_test();
    let validation = crate::session_io::RunValidationReport {
        missing_optional_files: vec!["frame_correlation.json".to_owned()],
        ..Default::default()
    };

    let summary = data_quality_summary(&session, &validation);

    assert_eq!(summary.level, DataQualityLevel::Medium);
    assert!(
        summary
            .reasons
            .iter()
            .any(|reason| reason.contains("optional correlation artifacts"))
    );
}

#[test]
fn data_quality_warns_on_cpu_perf_errors() {
    let mut session = minimal_session_for_report_test();
    session.config.cpu_perf = true;
    session.core.cpu_perf_open_errors = 1;
    session.core.cpu_perf_skipped_tasks = 2;
    let validation = crate::session_io::RunValidationReport::default();

    let summary = data_quality_summary(&session, &validation);

    assert_eq!(summary.level, DataQualityLevel::Medium);
    assert!(summary.cpu_perf_requested);
    assert_eq!(summary.cpu_perf_open_errors, 1);
    assert!(
        summary
            .reasons
            .iter()
            .any(|reason| reason.contains("CPU perf counters had open/read errors"))
    );
    assert!(
        summary
            .reasons
            .iter()
            .any(|reason| reason.contains("CPU perf skipped 2 active tasks"))
    );
}

#[test]
fn render_report_includes_data_quality_section() {
    let session = minimal_session_for_report_test();
    let artifacts = session_io::RunArtifacts::default();
    let data_quality = data_quality_summary(&session, &artifacts.validation);
    let pressure_timeline = PressureTimelineSummary::default();
    let runtime_slices = RuntimeSliceAnalysisSummary::default();

    let correlation_sections = TextReportCorrelationSections::new();
    let output = render_report(TextReportRenderInput {
        path: Path::new("session.json"),
        session: &session,
        cluster_analysis: &SpikeClusterAnalysis {
            source: SpikeClusterSource::TopSpikesFallback,
            source_count: 0,
            clusters: vec![],
        },
        frame_diagnoses: &[],
        data_quality: &data_quality,
        pressure_timeline: &pressure_timeline,
        runtime_slice_summary: &runtime_slices,
        correlation_sections: &correlation_sections,
        focus_summary: &FocusReportSummary::default(),
        foreground_summary: &ForegroundReportSummary::default(),
        display_path_diagnosis: None,
        top: 10,
        cluster_window_ms: 500,
        filter_class: None,
    });

    assert!(output.contains("data quality"));
    assert!(output.contains("level: High"));
}

#[test]
fn pressure_timeline_empty_without_intervals() {
    let summary = build_pressure_timeline(&[], &[pressure_cluster()], 5);

    assert_eq!(summary.sample_count, 0);
    assert_eq!(summary.max_cpu_some, 0.0);
    assert_eq!(summary.max_mem_some, None);
    assert!(summary.windows.is_empty());
}

#[test]
fn pressure_timeline_marks_near_spike() {
    let intervals = vec![
        pressure_interval(96, 10.0, 0.0, 0.0, 0.0, 0.0),
        pressure_interval(120, 20.0, 0.0, 0.0, 0.0, 0.0),
    ];

    let summary = build_pressure_timeline(&intervals, &[pressure_cluster()], 5);

    assert!(summary.windows[0].near_spike);
    assert!(!summary.windows[1].near_spike);
}

#[test]
fn pressure_timeline_sorts_windows() {
    let intervals = vec![
        pressure_interval(300, 1.0, 0.0, 0.0, 0.0, 0.0),
        pressure_interval(100, 2.0, 0.0, 0.0, 0.0, 0.0),
    ];

    let summary = build_pressure_timeline(&intervals, &[], 5);

    assert_eq!(
        summary
            .windows
            .iter()
            .map(|window| window.elapsed_ms)
            .collect::<Vec<_>>(),
        vec![100, 300]
    );
}

#[test]
fn pressure_timeline_max_cpu_some() {
    let intervals = vec![
        pressure_interval(100, 1.0, 0.0, 0.0, 0.0, 0.0),
        pressure_interval(200, 42.0, 0.0, 0.0, 0.0, 0.0),
        pressure_interval(300, 3.0, 0.0, 0.0, 0.0, 0.0),
    ];

    let summary = build_pressure_timeline(&intervals, &[], 5);

    assert_eq!(summary.max_cpu_some, 42.0);
}

#[test]
fn pressure_timeline_includes_memory_io_fields() {
    let intervals = vec![pressure_interval(100, 1.0, 2.0, 3.0, 4.0, 5.0)];

    let summary = build_pressure_timeline(&intervals, &[], 5);
    let window = &summary.windows[0];

    assert_eq!(summary.max_mem_some, Some(2.0));
    assert_eq!(summary.max_mem_full, Some(3.0));
    assert_eq!(summary.max_io_some, Some(4.0));
    assert_eq!(summary.max_io_full, Some(5.0));
    assert_eq!(window.mem_some, Some(2.0));
    assert_eq!(window.mem_full, Some(3.0));
    assert_eq!(window.io_some, Some(4.0));
    assert_eq!(window.io_full, Some(5.0));
}

#[test]
fn render_report_includes_pressure_timeline_when_pressure_present() {
    let session = minimal_session_for_report_test();
    let cluster_analysis = SpikeClusterAnalysis {
        source: SpikeClusterSource::TopSpikesFallback,
        source_count: 2,
        clusters: vec![pressure_cluster()],
    };
    let artifacts = session_io::RunArtifacts {
        intervals: vec![pressure_interval(100, 40.0, 2.0, 0.0, 0.0, 0.0)],
        ..Default::default()
    };
    let data_quality = data_quality_summary(&session, &artifacts.validation);
    let pressure_timeline =
        build_pressure_timeline(&artifacts.intervals, &cluster_analysis.clusters, 5);
    let runtime_slices = RuntimeSliceAnalysisSummary::default();

    let correlation_sections = TextReportCorrelationSections::new();
    let output = render_report(TextReportRenderInput {
        path: Path::new("session.json"),
        session: &session,
        cluster_analysis: &cluster_analysis,
        frame_diagnoses: &[],
        data_quality: &data_quality,
        pressure_timeline: &pressure_timeline,
        runtime_slice_summary: &runtime_slices,
        correlation_sections: &correlation_sections,
        focus_summary: &FocusReportSummary::default(),
        foreground_summary: &ForegroundReportSummary::default(),
        display_path_diagnosis: None,
        top: 10,
        cluster_window_ms: 5,
        filter_class: None,
    });

    assert!(output.contains("pressure timeline"));
    assert!(output.contains("samples=1"));
    assert!(output.contains("windows_near_spikes=1"));
    assert!(output.contains("max_cpu_some=40.00"));
}

#[test]
fn analysis_json_contains_pressure_timeline() {
    let session = minimal_session_for_report_test();
    let validation = crate::session_io::RunValidationReport::default();
    let analysis = ReportAnalysisJson {
        session: session.clone(),
        cluster_analysis: SpikeClusterAnalysis {
            source: SpikeClusterSource::TopSpikesFallback,
            source_count: 0,
            clusters: vec![],
        },
        frame_diagnoses: vec![],
        frame_pacing: FramePacingSummary::default(),
        pressure_timeline: build_pressure_timeline(
            &[pressure_interval(100, 10.0, 0.0, 0.0, 0.0, 0.0)],
            &[],
            5,
        ),
        runtime_slices: RuntimeSliceAnalysisSummary::default(),
        diagnosis_thresholds: crate::diagnosis::DiagnosisConfig::default().threshold_table(),
        artifacts_summary: artifacts_summary_from_session(&session),
        data_quality: data_quality_summary(&session, &validation),
        focus_summary: FocusReportSummary::default(),
        foreground_summary: ForegroundReportSummary::default(),
        kms_timing: KmsTimingSummary::default(),
        drm_fence_timing: DrmFenceTimingSummary::default(),
        cross_gpu_fence: CrossGpuFenceSummary::default(),
        wayland_presentation: WaylandPresentationSummary::default(),
        direct_scanout: DirectScanoutSummary::default(),
        dmabuf_path: DmaBufPathSummary::default(),
        gpu_engine_activity: GpuEngineActivitySummary::default(),
        display_path_diagnosis: DisplayPathDiagnosisSummary::default(),
    };

    let value = serde_json::to_value(&analysis).unwrap();

    assert!(value.get("pressure_timeline").is_some());
    assert_eq!(value["pressure_timeline"]["sample_count"].as_u64(), Some(1));
}

#[test]
fn runtime_slice_summary_reports_sources_and_top_threads() {
    let mut session = minimal_session_for_report_test();
    session.config.runtime_slices = true;
    session.core.runtime_slice_count = 1;
    let artifacts = session_io::RunArtifacts {
        session: session.clone(),
        runtime_slices: vec![crate::metrics::RuntimeSliceRecord {
            elapsed_ms: 1000,
            task: 42,
            process_pid: Some(40),
            class: TaskClass::Game,
            comm: "RenderThread".to_owned(),
            process_comm: "Game.exe".into(),
            source: crate::metrics::RuntimeSliceSource::ProcSchedstat,
            interval_ms: 1000,
            runtime_delta_ns: 850_000_000,
            runqueue_wait_delta_ns: Some(75_000_000),
            timeslices_delta: Some(12),
            runtime_ratio: Some(0.85),
            wait_ratio: Some(0.075),
            valid: true,
            ..Default::default()
        }],
        ..Default::default()
    };

    let summary = runtime_slice_analysis_summary(&session, &artifacts);

    assert!(summary.available);
    assert_eq!(summary.sample_count, 1);
    assert_eq!(summary.source_counts.get("proc_schedstat"), Some(&1));
    assert_eq!(summary.high_runtime_threads[0].task, 42);
}
