use super::*;

pub(crate) fn markers(duration_ns: u64, id: &str) -> FixedWorkMeasurement {
    let start = WorkloadProgress {
        schema_version: 1, execution_id: id.to_owned(),
        identity: FixedWorkIdentity { kind: WorkloadKind::Compile, source_tree: format!("sha256:{}", "a".repeat(64)),
            target: "all".to_owned(), build_state: "clean".to_owned(), cache_policy: "disabled".to_owned(), job_count: 4,
            command: vec!["build-tool".to_owned(), "all".to_owned()], working_directory: "/work/project".to_owned(),
            toolchain: "fixture-toolchain".to_owned(), environment: "fixture-environment".to_owned() },
        source: ProgressSource::ScenarioCompletion, phase: WorkloadPhase::Start, target_root_pid: 42,
        target_starttime_ticks: 100, monotonic_ns: 1_000, elapsed_ns: 0, completed_units: 0, total_units: Some(1),
    };
    let complete = WorkloadProgress { phase: WorkloadPhase::Complete, monotonic_ns: 1_000 + duration_ns,
        elapsed_ns: duration_ns, completed_units: 1, ..start.clone() };
    FixedWorkMeasurement { start, complete }
}

#[test]
fn markers_round_trip_and_measure_exact_monotonic_elapsed_time() {
    let measurement = markers(31_400_000_000, "baseline");
    let restored: FixedWorkMeasurement = serde_json::from_str(&serde_json::to_string(&measurement).unwrap()).unwrap();
    assert_eq!(restored.elapsed_ns(), Ok(31_400_000_000));
    let result = completed_fixed_work([&restored.start, &restored.complete].into_iter()).unwrap();
    assert_eq!(result, measurement);
    assert!(serde_json::to_string(&restored.start).unwrap().contains("WORKLOAD_START"));
    assert!(serde_json::to_string(&restored.complete).unwrap().contains("WORKLOAD_COMPLETE"));
}

#[test]
fn missing_duplicate_reversed_failed_or_mismatched_markers_are_not_measurements() {
    let good = markers(10, "execution");
    for markers in [vec![], vec![&good.start], vec![&good.complete], vec![&good.complete, &good.start],
        vec![&good.start, &good.start, &good.complete], vec![&good.start, &good.complete, &good.start, &good.complete]] {
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
            0 => bad.source_tree = "unverified-label".to_owned(), 1 => bad.target.clear(),
            2 => bad.build_state.clear(), 3 => bad.cache_policy.clear(), 4 => bad.job_count = 0,
            5 => bad.command.clear(), 6 => bad.working_directory.clear(), 7 => bad.toolchain.clear(),
            _ => bad.environment.clear(),
        }
        assert!(bad.validate().is_err(), "case {case}");
    }
}
