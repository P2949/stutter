//! Report regression tests split out of `report::mod` to keep production module size bounded.

mod foreground_focus;
mod quality_pressure;
mod rendering_frame;

use super::*;

fn foreground_event(
    elapsed_ms: u64,
    pid: Option<u32>,
    app_id: Option<&str>,
    class: Option<&str>,
    title: Option<&str>,
    workspace: Option<&str>,
    confidence: f32,
) -> ForegroundEvent {
    ForegroundEvent {
        elapsed_ms,
        source: crate::foreground::ForegroundSource::Sway,
        status: crate::foreground::ForegroundProviderStatus::Available,
        pid,
        app_id: app_id.map(str::to_owned),
        class: class.map(str::to_owned),
        title: title.map(str::to_owned),
        window_id: Some("7".to_owned()),
        workspace: workspace.map(str::to_owned),
        confidence,
        stale_ms: None,
        reason: "test foreground event".to_owned(),
    }
}

fn cluster_at(elapsed_ms: u64) -> SpikeCluster {
    SpikeCluster {
        points: vec![SpikePoint {
            elapsed_ms: Some(elapsed_ms),
            ..SpikePoint::default()
        }],
        distinct_tasks: 1,
        min_switch_ns: 0,
        max_switch_ns: 0,
        max_latency_ns: 0,
        diagnosis: None,
        diagnosis_explanation: None,
        anchor_task: None,
        anchor_class: None,
        anchor_comm: None,
        anchor_kind: None,
        foreground_pid: None,
        foreground_app_id: None,
        foreground_class: None,
        foreground_confidence: None,
        wake_graph: Vec::new(),
    }
}

fn minimal_session_for_report_test() -> SessionFile {
    SessionFile {
        core: crate::recorder::SessionMetadataCore {
            schema_version: SESSION_SCHEMA_VERSION,
            duration_ms: 1000,
            interval_record_count: 1,
            active_target_pids_count: 1,
            block_io_correlation_basis: "request-pointer".to_owned(),
            ..Default::default()
        },
        stop_reason: "test".to_owned(),
        ..Default::default()
    }
}

fn spike_point_for_report_test(
    task: u32,
    class: TaskClass,
    comm: &str,
    latency_ns: u64,
) -> SpikePoint {
    SpikePoint {
        task,
        class,
        process_pid: Some(task),
        comm: comm.to_owned(),
        latency_ns,
        wakeup_ns: 10_000_000,
        switch_ns: 10_000_000 + latency_ns,
        elapsed_ms: Some(100),
        ..Default::default()
    }
}

fn pressure_interval(
    elapsed_ms: u64,
    cpu_some: f64,
    mem_some: f64,
    mem_full: f64,
    io_some: f64,
    io_full: f64,
) -> IntervalRecord {
    IntervalRecord {
        elapsed_ms,
        task: 42,
        active: true,
        class: TaskClass::Unknown,
        comm: "worker".to_owned(),
        process_pid: Some(42),
        process_comm: "worker".into(),
        cpu_psi_some: cpu_some,
        mem_psi_some: mem_some,
        mem_psi_full: mem_full,
        io_psi_some: io_some,
        io_psi_full: io_full,
        percentile_scope: "exact".to_owned(),
        ..Default::default()
    }
}

fn pressure_cluster() -> SpikeCluster {
    cluster_from_points(
        vec![
            SpikePoint {
                elapsed_ms: Some(100),
                ..spike_point_for_report_test(1, TaskClass::Unknown, "worker-a", 2_000_000)
            },
            SpikePoint {
                elapsed_ms: Some(110),
                ..spike_point_for_report_test(2, TaskClass::Unknown, "worker-b", 2_000_000)
            },
        ],
        2,
    )
}

fn test_html_report_model() -> HtmlReportModel {
    let mut session = minimal_session_for_report_test();
    session.tasks.push(SessionTask {
        task: 42,
        active: true,
        class: TaskClass::Game,
        process_pid: Some(42),
        process_comm: "test-game".into(),
        comm: "test-game".to_owned(),
        latency: crate::recorder::RecordedLatency {
            samples: 100,
            stored_samples: 100,
            percentile_scope: "exact".to_owned(),
            avg_ns: 750_000,
            p99_ns: 2_000_000,
            max_ns: 5_000_000,
            over_1ms: 7,
            over_2ms: 3,
            over_5ms: 1,
            ..Default::default()
        },
        ..Default::default()
    });

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
        pressure_timeline: PressureTimelineSummary::default(),
        runtime_slices: RuntimeSliceAnalysisSummary::default(),
        diagnosis_thresholds: crate::diagnosis::DiagnosisConfig::default().threshold_table(),
        artifacts_summary: artifacts_summary_from_session(&session),
        data_quality: data_quality_summary(&session, &validation),
        focus_summary: FocusReportSummary::default(),
        foreground_summary: ForegroundReportSummary::default(),
        kms_timing: KmsTimingSummary::default(),
        drm_fence_timing: DrmFenceTimingSummary::default(),
        wayland_presentation: WaylandPresentationSummary::default(),
    };

    build_html_report_model(
        &session,
        &session_io::RunArtifacts::default(),
        &analysis,
        10,
        None,
        Some("stutter report\n==============".to_owned()),
    )
    .unwrap()
}
