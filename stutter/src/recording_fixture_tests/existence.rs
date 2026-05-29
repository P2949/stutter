use super::*;

#[test]
fn minimal_recording_fixture_files_exist() {
    let mut temp = env::temp_dir();
    temp.push(format!(
        "stutter-test-minimal-fixture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    write_minimal_recording_fixture(&temp);

    assert!(temp.join("session.json").exists());
    assert!(temp.join("metadata.json").exists());

    let _ = fs::remove_dir_all(temp);
}

#[test]
fn report_rejects_missing_required_artifacts() {
    let mut temp = env::temp_dir();
    temp.push(format!(
        "stutter-test-missing-artifacts-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&temp).unwrap();

    // Call the report loading helper. It should fail because the directory is empty.
    let result = report::print_report(report::PrintReportInput {
        path: &temp,
        json: false,
        analysis_json: false,
        json_summary: false,
        top: 10,
        cluster_window_ms: 500,
        filter_class: None,
        flamegraph: None,
    });

    assert!(result.is_err());
    let err_msg = format!("{:?}", result.err().unwrap()).to_lowercase();

    let contains_marker = err_msg.contains("metadata")
        || err_msg.contains("session")
        || err_msg.contains("recording")
        || err_msg.contains("missing");

    assert!(
        contains_marker,
        "Error message '{}' did not contain any of the required markers (metadata, session, recording, missing)",
        err_msg
    );

    // Cleanup
    let _ = fs::remove_dir_all(temp);
}
