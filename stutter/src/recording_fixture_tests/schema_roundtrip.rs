use super::*;

#[test]
fn recording_schema_round_trip_keeps_core_fields() {
    let mut temp = env::temp_dir();
    temp.push(format!(
        "stutter-test-schema-trip-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));

    write_minimal_recording_fixture(&temp);

    // This test performs a schema serde round-trip check only.
    // There is currently no standalone public loading helper in report.rs to use for full coverage.
    let session_path = temp.join("session.json");
    let file = fs::File::open(&session_path).unwrap();
    let reader = std::io::BufReader::new(file);
    let session: SessionFile = serde_json::from_reader(reader).unwrap();

    // Assert core fields survive (matching values from write_minimal_recording_fixture)
    assert_eq!(session.core.schema_version, SESSION_SCHEMA_VERSION);
    assert_eq!(session.tasks.len(), 1);
    assert_eq!(session.tasks[0].comm, "game");
    assert_eq!(session.core.spike_events_retained_count, 0);
    assert!(!session.core.spike_events_truncated);
    assert_eq!(session.core.block_io_correlation_basis, "dev+sector");

    // Assert drop counters (should be default/zero in the minimal fixture)
    assert_eq!(session.core.drop_counters.total(), 0);

    let _ = fs::remove_dir_all(temp);
}
