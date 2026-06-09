use std::{
    fs,
    path::{Path, PathBuf},
};

use super::*;
use crate::{
    process_tree::TaskClass,
    recorder::{RecordedLatency, SESSION_SCHEMA_VERSION, SessionFile, SessionTask},
    report::diff::run_diff_summary_from_sessions,
    session_io::RunValidationReport,
};

fn test_session(tasks: Vec<SessionTask>) -> SessionFile {
    SessionFile {
        core: crate::recorder::SessionMetadataCore {
            schema_version: SESSION_SCHEMA_VERSION,
            run_name: Some("test-run".to_owned()),
            started_at: crate::recorder::RecordedTime::default(),
            ended_at: crate::recorder::RecordedTime::default(),
            duration_ms: 1000,
            interval_record_count: 7,
            ..Default::default()
        },
        stop_reason: "test".to_owned(),
        config: crate::recorder::RecordedConfig::default(),
        tasks,
        top_spikes: Vec::new(),
    }
}

fn test_task(task: u32, class: TaskClass, comm: &str, p99_ns: u64, max_ns: u64) -> SessionTask {
    SessionTask {
        task,
        active: true,
        class,
        process_pid: Some(task),
        process_comm: "game".into(),
        comm: comm.to_owned(),
        latency: RecordedLatency {
            samples: 10,
            p99_ns,
            max_ns,
            over_1ms: 1,
            over_2ms: 2,
            over_5ms: 3,
            ..Default::default()
        },
        ..Default::default()
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-summary-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_session(dir: &Path, session: &SessionFile) {
    fs::create_dir_all(dir).unwrap();
    fs::write(
        dir.join("session.json"),
        serde_json::to_string(session).unwrap(),
    )
    .unwrap();
}

#[test]
fn compact_summary_filters_top_tasks_by_class() {
    let session = test_session(vec![
        test_task(1, TaskClass::Game, "game", 2_000_000, 5_000_000),
        test_task(2, TaskClass::Helper, "helper", 9_000_000, 9_000_000),
    ]);

    let summary = compact_run_summary_from_session(
        Path::new("/tmp/run"),
        &session,
        &RunValidationReport::default(),
        10,
        Some(TaskClass::Game),
    );

    assert_eq!(summary.top_tasks_by_max_latency.len(), 1);
    assert_eq!(summary.top_tasks_by_max_latency[0].comm, "game");
    assert_eq!(summary.threshold_totals.over_1ms, 1);
}

#[test]
fn compact_summary_json_does_not_include_histogram() {
    let session = test_session(vec![test_task(
        1,
        TaskClass::Game,
        "game",
        2_000_000,
        5_000_000,
    )]);

    let summary = compact_run_summary_from_session(
        Path::new("/tmp/run"),
        &session,
        &RunValidationReport::default(),
        10,
        None,
    );

    let json = serde_json::to_string(&summary).unwrap();
    assert!(!json.contains("histogram"));
    assert!(json.contains("worst_task_by_max_latency"));
}

#[test]
fn run_diff_summary_tracks_regressions_and_new_scored_tasks() {
    let baseline = test_session(vec![test_task(
        1,
        TaskClass::Game,
        "game",
        2_000_000,
        5_000_000,
    )]);
    let current = test_session(vec![
        test_task(1, TaskClass::Game, "game", 3_000_000, 8_000_000),
        test_task(2, TaskClass::Game, "new-game", 1_000_000, 2_000_000),
    ]);

    let diff = run_diff_summary_from_sessions(
        Path::new("/tmp/base"),
        Path::new("/tmp/current"),
        &baseline,
        &current,
        None,
    );

    assert_eq!(diff.compared_tasks, 1);
    assert_eq!(diff.regressions.len(), 1);
    assert_eq!(diff.regressions[0].delta_max_ns, 3_000_000);
    assert_eq!(diff.new_scored_tasks.len(), 1);
}

#[test]
fn batch_summary_collects_runs_and_structured_errors() {
    let root = temp_dir("batch");
    let run_a = root.join("a");
    let run_b = root.join("b");
    let bad = root.join("bad");

    write_session(
        &run_a,
        &test_session(vec![test_task(
            1,
            TaskClass::Game,
            "game",
            2_000_000,
            5_000_000,
        )]),
    );
    write_session(
        &run_b,
        &test_session(vec![test_task(
            1,
            TaskClass::Game,
            "game",
            3_000_000,
            6_000_000,
        )]),
    );
    fs::create_dir_all(&bad).unwrap();
    fs::write(bad.join("session.json"), "not json").unwrap();

    let summary = build_batch_run_summary(&root, None, 10, None).unwrap();

    assert_eq!(summary.run_count, 2);
    assert!(summary.worst_p99.is_some());
    assert_eq!(summary.errors.len(), 1);
    assert!(summary.errors[0].error.contains("failed to parse"));

    fs::remove_dir_all(root).ok();
}
