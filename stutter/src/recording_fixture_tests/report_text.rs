use super::*;

#[test]
fn minimal_recording_report_text_does_not_panic() {
    let mut temp = env::temp_dir();
    temp.push(format!(
        "stutter-test-report-render-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    write_minimal_recording_fixture(&temp);

    // Load the session to pass to the renderer.
    let session_path = temp.join("session.json");
    let session_data = fs::read_to_string(&session_path).unwrap();
    let session: SessionFile = serde_json::from_str(&session_data).unwrap();

    let data_quality = report::data_quality_summary(&session, &RunArtifacts::default().validation);
    let pressure_timeline = report::PressureTimelineSummary::default();
    let runtime_slices = report::RuntimeSliceAnalysisSummary::default();

    // Call the report rendering helper.
    // We use default/empty values for clusters and artifacts to keep it minimal.
    let correlation_sections = report::TextReportCorrelationSections::new();
    let output = report::render_report(report::TextReportRenderInput {
        path: &temp,
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
        focus_summary: &report::FocusReportSummary::default(),
        foreground_summary: &report::ForegroundReportSummary::default(),
        display_path_diagnosis: None,
        top: 10,
        cluster_window_ms: 500,
        filter_class: None,
    });

    // Assert rendered text contains stable words from report.rs
    assert!(output.contains("stutter report"));
    assert!(output.contains("duration_ms"));

    let _ = fs::remove_dir_all(temp);
}
