//! Regression coverage for streamed spike artifacts and fallback loading.

use super::{support::*, *};

#[test]
fn spike_event_stream_writes_ndjson() {
    let dir = temp_test_dir("spike-stream");
    fs::create_dir_all(&dir).unwrap();
    let spike_path = dir.join("spike_events.json");

    let mut recorder = recorder::LiveRecorder::default();
    recorder
        .streams
        .create_stream(&dir, ArtifactKind::SpikeEvents)
        .unwrap();

    let spike1 = spike_event(1, 1000);
    let spike2 = spike_event(2, 2000);

    events::push_artifact_event(
        &mut recorder,
        ArtifactKind::SpikeEvents,
        &spike1,
        "buffers.spike_events",
        |c| c.spike_event_count += 1,
    );

    events::push_artifact_event(
        &mut recorder,
        ArtifactKind::SpikeEvents,
        &spike2,
        "buffers.spike_events",
        |c| c.spike_event_count += 1,
    );

    // Drop the recorder to finish the writer
    drop(recorder);

    let contents = fs::read_to_string(&spike_path).unwrap();
    let lines: Vec<_> = contents.lines().collect();
    assert_eq!(lines.len(), 2);

    for line in lines {
        let value: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(value.is_object());
    }

    assert!(!contents.trim_start().starts_with('['));

    fs::remove_dir_all(dir).ok();
}

#[test]
fn load_run_artifacts_loads_streamed_spikes() {
    let dir = temp_test_dir("load-spikes");
    fs::create_dir_all(&dir).unwrap();

    let spike1 = spike_event(1, 1000);
    let spike2 = spike_event(2, 2000);

    let mut session = SessionFile::default();
    session.core.schema_version = SESSION_SCHEMA_VERSION;
    session.core.spike_events_retained_count = 2;
    fs::write(
        dir.join("session.json"),
        serde_json::to_string(&session).unwrap(),
    )
    .unwrap();

    let mut file = fs::File::create(dir.join("spike_events.json")).unwrap();
    use std::io::Write;
    writeln!(file, "{}", serde_json::to_string(&spike1).unwrap()).unwrap();
    writeln!(file, "{}", serde_json::to_string(&spike2).unwrap()).unwrap();
    drop(file);

    let artifacts =
        crate::session_io::load_run_artifacts(&dir, ArtifactSelection::report()).unwrap();

    assert_eq!(artifacts.spikes.len(), 2);
    assert_eq!(artifacts.spikes[0].task, spike1.task);
    assert_eq!(artifacts.spikes[1].task, spike2.task);

    fs::remove_dir_all(dir).ok();
}

#[test]
fn load_run_artifacts_falls_back_to_top_spikes() {
    let dir = temp_test_dir("fallback-spikes");
    fs::create_dir_all(&dir).unwrap();

    let spike1 = recorder::SessionSpike {
        task: 1,
        latency_ns: 1000,
        ..Default::default()
    };

    let mut session = SessionFile::default();
    session.core.schema_version = SESSION_SCHEMA_VERSION;
    session.top_spikes = vec![spike1.clone()];
    session.core.spike_events_retained_count = 1;
    fs::write(
        dir.join("session.json"),
        serde_json::to_string(&session).unwrap(),
    )
    .unwrap();

    // No spike_events.json

    let artifacts =
        crate::session_io::load_run_artifacts(&dir, ArtifactSelection::report()).unwrap();

    assert_eq!(artifacts.spikes.len(), 1);
    assert_eq!(artifacts.spikes[0].task, spike1.task);
    assert_eq!(artifacts.spikes[0].latency_ns, spike1.latency_ns);

    fs::remove_dir_all(dir).ok();
}
