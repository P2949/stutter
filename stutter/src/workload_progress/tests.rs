use super::*;

pub(crate) fn markers(duration_ns: u64, id: &str) -> FixedWorkMeasurement {
    let start = WorkloadProgress {
        schema_version: 1,
        execution_id: id.to_owned(),
        identity: FixedWorkIdentity {
            kind: WorkloadKind::Compile,
            source_tree: format!("sha256:{}", "a".repeat(64)),
            target: "all".to_owned(),
            build_state: "clean".to_owned(),
            cache_policy: "disabled".to_owned(),
            job_count: 4,
            command: vec!["build-tool".to_owned(), "all".to_owned()],
            working_directory: "/work/project".to_owned(),
            toolchain: "fixture-toolchain".to_owned(),
            environment: "fixture-environment".to_owned(),
        },
        source: ProgressSource::ScenarioCompletion,
        phase: WorkloadPhase::Start,
        target_root_pid: 42,
        target_starttime_ticks: 100,
        monotonic_ns: 1_000,
        elapsed_ns: 0,
        completed_units: 0,
        total_units: Some(1),
    };
    let complete = WorkloadProgress {
        phase: WorkloadPhase::Complete,
        monotonic_ns: 1_000 + duration_ns,
        elapsed_ns: duration_ns,
        completed_units: 1,
        ..start.clone()
    };
    FixedWorkMeasurement { start, complete }
}

#[test]
fn markers_round_trip_and_measure_exact_monotonic_elapsed_time() {
    let measurement = markers(31_400_000_000, "baseline");
    let restored: FixedWorkMeasurement =
        serde_json::from_str(&serde_json::to_string(&measurement).unwrap()).unwrap();
    assert_eq!(restored.elapsed_ns(), Ok(31_400_000_000));
    let result = completed_fixed_work([&restored.start, &restored.complete].into_iter()).unwrap();
    assert_eq!(result, measurement);
    assert!(
        serde_json::to_string(&restored.start)
            .unwrap()
            .contains("WORKLOAD_START")
    );
    assert!(
        serde_json::to_string(&restored.complete)
            .unwrap()
            .contains("WORKLOAD_COMPLETE")
    );
}

#[test]
fn missing_duplicate_reversed_failed_or_mismatched_markers_are_not_measurements() {
    let good = markers(10, "execution");
    for markers in [
        vec![],
        vec![&good.start],
        vec![&good.complete],
        vec![&good.complete, &good.start],
        vec![&good.start, &good.start, &good.complete],
        vec![&good.start, &good.complete, &good.start, &good.complete],
    ] {
        assert!(completed_fixed_work(markers.into_iter()).is_err());
    }
    for case in 0..10 {
        let mut bad = good.clone();
        match case {
            0 => bad.complete.phase = WorkloadPhase::Failed,
            1 => bad.complete.execution_id = "other".to_owned(),
            2 => bad.complete.identity.job_count = 8,
            3 => bad.complete.source = ProgressSource::ExternalMarker,
            4 => bad.complete.target_root_pid = 99,
            5 => bad.complete.target_starttime_ticks = 101,
            6 => bad.complete.elapsed_ns = 9,
            7 => bad.complete.completed_units = 0,
            8 => bad.complete.total_units = Some(2),
            _ => bad.complete.schema_version = 2,
        }
        assert!(bad.validate().is_err(), "case {case}");
    }
}

#[test]
fn incomplete_fixed_work_identity_is_refused() {
    let identity = markers(1, "execution").start.identity;
    for case in 0..9 {
        let mut bad = identity.clone();
        match case {
            0 => bad.source_tree = "unverified-label".to_owned(),
            1 => bad.target.clear(),
            2 => bad.build_state.clear(),
            3 => bad.cache_policy.clear(),
            4 => bad.job_count = 0,
            5 => bad.command.clear(),
            6 => bad.working_directory.clear(),
            7 => bad.toolchain.clear(),
            _ => bad.environment.clear(),
        }
        assert!(bad.validate().is_err(), "case {case}");
    }
}

#[test]
fn fixed_work_markers_are_reliable_and_survive_target_scoped_window_aggregation() {
    use crate::{
        autotune::{objective::ObjectiveSignalQuality, rolling_window::RollingWindow},
        session_events::{MonitorEvent, MonitorEventDeliveryClass},
    };
    let work = markers(20_000_000_000, "execution");
    let mut window = RollingWindow::new(std::time::Duration::from_secs(60));
    for (elapsed_ms, progress) in [(1_000, work.start.clone()), (21_000, work.complete.clone())] {
        let record = WorkloadProgressRecord {
            elapsed_ms,
            progress,
        };
        let event = MonitorEvent::WorkloadProgress {
            record: Box::new(record.clone()),
        };
        assert_eq!(event.kind(), "workload_progress");
        assert_eq!(event.elapsed_ms(), Some(elapsed_ms));
        assert_eq!(event.delivery_class(), MonitorEventDeliveryClass::Reliable);
        window.push_workload_progress(record);
    }
    assert_eq!(window.objective_signals().compile_fixed_work, None);
    let signals = window.objective_signals_for_target(42, &std::collections::BTreeSet::from([42]));
    assert_eq!(signals.compile_fixed_work, Some(work));
    assert_eq!(
        signals.signal_quality.compile_throughput,
        ObjectiveSignalQuality::Direct
    );
    assert_eq!(
        window
            .objective_signals_for_target(99, &std::collections::BTreeSet::from([99]))
            .compile_fixed_work,
        None
    );
    window.prune_to(100_000);
    assert_eq!(
        window
            .objective_signals_for_target(42, &std::collections::BTreeSet::from([42]))
            .compile_fixed_work,
        None
    );
}

#[test]
fn reader_skips_existing_input_and_buffers_partial_lines() {
    use std::io::Write;
    let root = crate::test_support::TestRoot::new("workload-reader");
    let path = root.join("markers.jsonl");
    std::fs::write(&path, b"historical input is not replayed\n").unwrap();
    let mut reader = WorkloadProgressReader::open_tail(&path).unwrap();
    let work = markers(10, "execution");
    let line = serde_json::to_vec(&work.start).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    file.write_all(&line[..line.len() / 2]).unwrap();
    assert!(reader.poll(0, u64::MAX).unwrap().is_empty());
    file.write_all(&line[line.len() / 2..]).unwrap();
    file.write_all(b"\n").unwrap();
    let records = reader.poll(0, u64::MAX).unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].progress, work.start);
    assert!(reader.poll(0, u64::MAX).unwrap().is_empty());
    std::fs::write(&path, b"").unwrap();
    assert!(reader.poll(0, u64::MAX).is_err());
}

#[test]
fn reader_refuses_replacement_or_malformed_input() {
    use std::io::Write;
    let root = crate::test_support::TestRoot::new("workload-reader-invalid");
    let path = root.join("markers.jsonl");
    std::fs::write(&path, b"").unwrap();
    let mut reader = WorkloadProgressReader::open_tail(&path).unwrap();
    let old = root.join("old.jsonl");
    std::fs::rename(&path, &old).unwrap();
    std::fs::write(&path, b"").unwrap();
    assert!(reader.poll(0, u64::MAX).is_err());
    let mut reader = WorkloadProgressReader::open_tail(&path).unwrap();
    std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(b"not-json\n")
        .unwrap();
    assert!(reader.poll(0, u64::MAX).is_err());
}

#[test]
fn optional_fixed_work_artifacts_round_trip_without_changing_old_recordings() {
    use std::io::Write;

    use crate::artifacts::{ArtifactKind, ArtifactSelection, artifact_path};
    let root = crate::test_support::TestRoot::new("workload-artifact");
    crate::recording_fixture_tests::write_minimal_recording_fixture(root.path());
    let old =
        crate::session_io::load_run_artifacts(root.path(), ArtifactSelection::report()).unwrap();
    assert!(old.workload_progress.is_empty());
    let work = markers(10, "execution");
    let records = [
        WorkloadProgressRecord {
            elapsed_ms: 1,
            progress: work.start,
        },
        WorkloadProgressRecord {
            elapsed_ms: 2,
            progress: work.complete,
        },
    ];
    let mut file =
        std::fs::File::create(artifact_path(root.path(), ArtifactKind::WorkloadProgress)).unwrap();
    for record in &records {
        serde_json::to_writer(&mut file, record).unwrap();
        file.write_all(b"\n").unwrap();
    }
    let artifacts =
        crate::session_io::load_run_artifacts(root.path(), ArtifactSelection::report()).unwrap();
    assert_eq!(artifacts.workload_progress, records);
}

#[test]
fn command_runner_emits_completion_only_for_successful_unchanged_work() {
    use std::process::Command;
    for (command, expected_phase) in [
        ("test -f source.txt", WorkloadPhase::Complete),
        ("exit 7", WorkloadPhase::Failed),
        ("printf changed >> source.txt", WorkloadPhase::Failed),
    ] {
        let root = crate::test_support::TestRoot::new("fixed-work-command");
        std::fs::write(root.join("source.txt"), b"source\n").unwrap();
        for args in [
            vec!["init", "--quiet"],
            vec!["add", "source.txt"],
            vec![
                "-c",
                "user.name=Stutter test",
                "-c",
                "user.email=stutter-test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "-m",
                "fixture",
            ],
        ] {
            assert!(
                Command::new("git")
                    .args(args)
                    .current_dir(root.path())
                    .status()
                    .unwrap()
                    .success()
            );
        }
        let tree = Command::new("git")
            .args(["rev-parse", "HEAD^{tree}"])
            .current_dir(root.path())
            .output()
            .unwrap();
        let mut identity = markers(1, "unused").start.identity;
        identity.kind = WorkloadKind::Custom;
        identity.source_tree = format!("git:{}", String::from_utf8(tree.stdout).unwrap().trim());
        identity.working_directory = root.path().to_string_lossy().into_owned();
        identity.command = vec!["sh".to_owned(), "-c".to_owned(), command.to_owned()];
        let mut emitted = Vec::new();
        let result = run_fixed_work(
            identity,
            "actual-command".to_owned(),
            std::process::id(),
            |marker| {
                emitted.push(marker.clone());
                Ok(())
            },
        );
        assert_eq!(emitted.len(), 2);
        assert_eq!(emitted[0].phase, WorkloadPhase::Start);
        assert_eq!(emitted[1].phase, expected_phase);
        assert_eq!(result.is_ok(), expected_phase == WorkloadPhase::Complete);
        if result.is_ok() {
            assert!(
                completed_fixed_work(emitted.iter())
                    .unwrap()
                    .elapsed_ns()
                    .unwrap()
                    > 0
            );
        }
    }
}
