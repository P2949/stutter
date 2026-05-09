use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;

use crate::{
    recorder::{MetadataFile, SESSION_SCHEMA_VERSION},
    report, session_io, test_fixture_builder, validate,
};

fn temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-artifact-contract-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_corpus(name: &str) -> PathBuf {
    let root = temp_dir(name);
    test_fixture_builder::write_validation_corpus(&root).unwrap();
    root
}

fn write_json_pretty<T: serde::Serialize>(path: impl AsRef<Path>, value: &T) {
    let file = fs::File::create(path).unwrap();
    serde_json::to_writer_pretty(file, value).unwrap();
}

fn read_json(path: impl AsRef<Path>) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn assert_object_has_keys(value: &Value, keys: &[&str]) {
    let object = value.as_object().unwrap();
    for key in keys {
        assert!(object.contains_key(*key), "missing key {key}: {value:#}");
    }
}

fn non_empty_line_count(path: impl AsRef<Path>) -> u64 {
    let path = path.as_ref();
    if !path.exists() {
        return 0;
    }
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count() as u64
}

#[test]
fn analysis_json_has_stable_top_level_contract() {
    let path = test_fixture_builder::fixture_path("clean_run");
    let analysis = report::build_report_analysis(&path, 10, 5, None).unwrap();
    let value = serde_json::to_value(&analysis).unwrap();

    assert_object_has_keys(
        &value,
        &[
            "session",
            "cluster_analysis",
            "frame_diagnoses",
            "pressure_timeline",
            "artifacts_summary",
            "data_quality",
        ],
    );

    assert_object_has_keys(
        value.get("data_quality").unwrap(),
        &[
            "level",
            "reasons",
            "missing_optional_files",
            "validation_errors",
            "validation_warnings",
            "schema_version",
            "expected_schema_version",
            "event_stream_write_errors",
            "spike_events_truncated",
            "spike_events_retained_count",
            "spike_events_dropped_count",
            "interval_record_count",
            "active_target_pids_count",
            "drop_counters_nonzero",
            "percentile_scope_counts",
            "block_io_correlation_basis",
            "frame_timestamp_alignment",
            "cpu_perf_requested",
            "cpu_perf_open_errors",
            "cpu_perf_read_errors",
            "cpu_perf_skipped_tasks",
        ],
    );
}

#[test]
fn old_session_schema_warns_not_errors() {
    let root = write_corpus("old-schema");
    let dir = root.join("old_schema_warning");

    let validation = session_io::validate_run_dir(&dir).unwrap();
    assert!(validation.errors.is_empty(), "{:?}", validation.errors);
    assert!(
        validation
            .warnings
            .iter()
            .any(|warning| warning.contains("older than current")),
        "warnings={:?}",
        validation.warnings
    );

    let analysis = report::build_report_analysis(&dir, 10, 5, None).unwrap();
    assert_eq!(
        analysis.data_quality.schema_version,
        SESSION_SCHEMA_VERSION - 1
    );
    assert_eq!(
        analysis.data_quality.expected_schema_version,
        SESSION_SCHEMA_VERSION
    );
    assert!(
        analysis
            .data_quality
            .validation_warnings
            .iter()
            .any(|warning| warning.contains("older than current")),
        "warnings={:?}",
        analysis.data_quality.validation_warnings
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn newer_session_schema_errors() {
    let root = write_corpus("newer-schema");
    let dir = root.join("clean_run");

    let mut session = session_io::load_session(&dir).unwrap();
    session.core.schema_version = SESSION_SCHEMA_VERSION + 1;
    write_json_pretty(dir.join(session_io::SESSION_FILE), &session);

    let metadata = MetadataFile {
        core: session.core.clone(),
    };
    write_json_pretty(dir.join(session_io::METADATA_FILE), &metadata);

    let validation = session_io::validate_run_dir(&dir).unwrap();
    assert!(
        validation
            .errors
            .iter()
            .any(|error| error.contains("newer than current")),
        "errors={:?}",
        validation.errors
    );

    let output = validate::validate_run_for_command(&dir, false);
    assert!(!output.passed);

    fs::remove_dir_all(root).ok();
}

#[test]
fn metadata_session_count_mismatch_warns() {
    let root = write_corpus("count-mismatch");
    let dir = root.join("clean_run");

    let session = session_io::load_session(&dir).unwrap();
    let metadata = MetadataFile {
        core: {
            let mut core = session.core.clone();
            core.spike_events_retained_count = session.core.spike_events_retained_count + 1;
            core
        },
    };
    write_json_pretty(dir.join(session_io::METADATA_FILE), &metadata);

    let artifacts =
        session_io::load_run_artifacts(&dir, session_io::ArtifactLoadOptions::REPORT).unwrap();
    assert!(
        artifacts
            .validation
            .warnings
            .iter()
            .any(|warning| warning.contains("spike count mismatch")),
        "warnings={:?}",
        artifacts.validation.warnings
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn present_invalid_ndjson_errors() {
    let root = write_corpus("invalid-ndjson");
    let dir = root.join("clean_run");
    fs::write(dir.join(session_io::INTERVALS_FILE), "invalid json\n").unwrap();

    let validation = session_io::validate_run_dir(&dir).unwrap();
    assert!(
        validation
            .errors
            .iter()
            .any(|error| error.contains("interval.json invalid")),
        "errors={:?}",
        validation.errors
    );

    fs::remove_dir_all(root).ok();
}

#[test]
fn docs_example_artifacts_validate() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("docs")
        .join("examples")
        .join("artifacts")
        .join(format!("v{}", SESSION_SCHEMA_VERSION));

    let mut validated_any = false;
    for entry in fs::read_dir(&root).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let validation = session_io::validate_run_dir(&path).unwrap();
        assert!(
            validation.errors.is_empty(),
            "{} errors: {:?}",
            path.display(),
            validation.errors
        );

        let analysis = report::build_report_analysis(&path, 10, 5, None).unwrap();
        assert!(
            analysis.data_quality.validation_errors.is_empty(),
            "{} validation errors: {:?}",
            path.display(),
            analysis.data_quality.validation_errors
        );
        serde_json::to_value(&analysis).unwrap();

        let session = session_io::load_session(&path).unwrap();
        assert_eq!(
            session.core.interval_record_count,
            non_empty_line_count(path.join(session_io::INTERVALS_FILE)),
            "{} interval count mismatch",
            path.display()
        );
        assert_eq!(
            session.core.spike_events_retained_count,
            non_empty_line_count(path.join(session_io::SPIKES_FILE)),
            "{} spike count mismatch",
            path.display()
        );
        assert_eq!(
            session.core.gpu_sample_count,
            non_empty_line_count(path.join(session_io::GPU_SAMPLES_FILE)),
            "{} gpu sample count mismatch",
            path.display()
        );
        assert_eq!(
            session.core.block_io_event_count,
            non_empty_line_count(path.join(session_io::BLOCK_IO_EVENTS_FILE)),
            "{} block io count mismatch",
            path.display()
        );
        let frame_count = if path.join(session_io::FRAME_EVENTS_FILE).exists() {
            non_empty_line_count(path.join(session_io::FRAME_EVENTS_FILE))
        } else {
            non_empty_line_count(path.join(session_io::FRAME_EVENTS_STREAM_FILE))
        };
        assert_eq!(
            session.core.frame_event_count,
            frame_count,
            "{} frame count mismatch",
            path.display()
        );

        let metadata = read_json(path.join(session_io::METADATA_FILE));
        assert_eq!(
            metadata
                .get("spike_events_retained_count")
                .and_then(Value::as_u64),
            Some(session.core.spike_events_retained_count),
            "{} metadata spike count mismatch",
            path.display()
        );

        validated_any = true;
    }

    assert!(
        validated_any,
        "no public examples found in {}",
        root.display()
    );
}
