use super::*;

fn temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-daemon-state-test-{name}-{}-{}",
        std::process::id(),
        crate::audit::unix_nanos_now()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn temporary_files_in(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("tmp"))
        .collect()
}

#[test]
fn daemon_state_default_serializes_with_schema_version() {
    let state = DaemonState::default();

    let json = serde_json::to_string(&state).unwrap();

    assert!(json.contains("\"schema_version\":1"));
    assert!(json.contains("\"mode\":\"observe\""));
    assert!(json.contains("\"phase\":\"disabled\""));
}

#[test]
fn daemon_phase_helpers_report_lifecycle_labels_and_terminal_states() {
    assert_eq!(DaemonPhase::Init.lifecycle_label(), "init");
    assert_eq!(DaemonPhase::Recover.lifecycle_label(), "recover");
    assert_eq!(DaemonPhase::Paused.lifecycle_label(), "paused");
    assert_eq!(DaemonPhase::Observe.lifecycle_label(), "observe");
    assert_eq!(DaemonPhase::Decide.lifecycle_label(), "decide");
    assert_eq!(DaemonPhase::Apply.lifecycle_label(), "apply");
    assert_eq!(DaemonPhase::Measure.lifecycle_label(), "measure");
    assert_eq!(DaemonPhase::Rollback.lifecycle_label(), "rollback");
    assert_eq!(DaemonPhase::Cooldown.lifecycle_label(), "cooldown");
    assert_eq!(DaemonPhase::Faulted.lifecycle_label(), "faulted");
    assert_eq!(DaemonPhase::Shutdown.lifecycle_label(), "shutdown");

    assert!(DaemonPhase::Disabled.is_terminal());
    assert!(DaemonPhase::Paused.is_terminal());
    assert!(DaemonPhase::Faulted.is_terminal());
    assert!(DaemonPhase::Shutdown.is_terminal());
    assert!(!DaemonPhase::Observe.is_terminal());
    assert!(!DaemonPhase::Measure.is_terminal());

    assert!(DaemonPhase::Faulted.is_faulted());
    assert!(!DaemonPhase::Shutdown.is_faulted());
}

#[test]
fn daemon_phase_preserves_existing_serialized_names_and_accepts_new_aliases() {
    let serialized_names = [
        (DaemonPhase::Disabled, "\"disabled\""),
        (DaemonPhase::Init, "\"init\""),
        (DaemonPhase::Recover, "\"recover\""),
        (DaemonPhase::Paused, "\"paused\""),
        (DaemonPhase::Observe, "\"observing\""),
        (DaemonPhase::Decide, "\"planning\""),
        (DaemonPhase::Apply, "\"applying\""),
        (DaemonPhase::Measure, "\"measuring\""),
        (DaemonPhase::Keep, "\"keeping\""),
        (DaemonPhase::Rollback, "\"reverting\""),
        (DaemonPhase::Cooldown, "\"cooldown\""),
        (DaemonPhase::Faulted, "\"faulted\""),
        (DaemonPhase::Shutdown, "\"shutdown\""),
    ];

    for (phase, expected_json) in serialized_names {
        assert_eq!(serde_json::to_string(&phase).unwrap(), expected_json);
    }

    let accepted_names = [
        ("\"disabled\"", DaemonPhase::Disabled),
        ("\"init\"", DaemonPhase::Init),
        ("\"recover\"", DaemonPhase::Recover),
        ("\"paused\"", DaemonPhase::Paused),
        ("\"observing\"", DaemonPhase::Observe),
        ("\"observe\"", DaemonPhase::Observe),
        ("\"planning\"", DaemonPhase::Decide),
        ("\"decide\"", DaemonPhase::Decide),
        ("\"applying\"", DaemonPhase::Apply),
        ("\"apply\"", DaemonPhase::Apply),
        ("\"measuring\"", DaemonPhase::Measure),
        ("\"measure\"", DaemonPhase::Measure),
        ("\"keeping\"", DaemonPhase::Keep),
        ("\"keep\"", DaemonPhase::Keep),
        ("\"reverting\"", DaemonPhase::Rollback),
        ("\"rollback\"", DaemonPhase::Rollback),
        ("\"cooldown\"", DaemonPhase::Cooldown),
        ("\"faulted\"", DaemonPhase::Faulted),
        ("\"shutdown\"", DaemonPhase::Shutdown),
    ];

    for (json, expected_phase) in accepted_names {
        assert_eq!(
            serde_json::from_str::<DaemonPhase>(json).unwrap(),
            expected_phase
        );
    }
}

#[test]
fn daemon_state_can_store_live_runtime_fields() {
    let state = DaemonState {
        mode: DaemonMode::ApplyLowRisk,
        phase: DaemonPhase::Measure,
        active_target: Some(DaemonTargetState {
            root_pid: Some(1234),
            active_targets: 12,
            comm: Some("game".to_owned()),
        }),
        active_experiment: Some(DaemonExperimentState {
            experiment_id: "experiment-1".to_owned(),
            action_id: "cpu-affinity-profile:game".to_owned(),
            candidate_name: Some("game".to_owned()),
            mode: DaemonMode::ApplyLowRisk,
            safety_class: SafetyClass::ReversibleLowRisk,
            started_unix_nanos: Some(100),
        }),
        active_rollback: Some(DaemonRollbackState {
            action_id: "cpu-affinity-profile:game".to_owned(),
            mode: DaemonMode::ApplyLowRisk,
            safety_class: SafetyClass::ReversibleLowRisk,
            rollback_available: true,
            token: None,
            manual_restore_command: Some("stutter autotune restore".to_owned()),
        }),
        last_decision: Some(DaemonDecisionState {
            decision: "candidate_applied".to_owned(),
            reason: "candidate passed gates".to_owned(),
            unix_nanos: Some(200),
            diagnostic_current_raw_score_total: Some(300),
            candidate_count: None,
            top_denied_reason: None,
            planner: None,
            situation: None,
            focus_kind: None,
        }),
        degraded: vec![DaemonDegradedStatus {
            category: "data_quality".to_owned(),
            message: "low scored samples".to_owned(),
        }],
        faulted: None,
        profile_memory: DaemonProfileMemory {
            profiles: vec![DaemonWorkloadProfile {
                workload_identity_hash: "workload-abc".to_owned(),
                workload_label: Some("game".to_owned()),
                candidate_name: "game-main".to_owned(),
                action_id: "cpu-affinity-profile:game-main".to_owned(),
                action_kind: "cpu_affinity_profile".to_owned(),
                safety_class: SafetyClass::ReversibleLowRisk,
                kept_unix_nanos: 300,
                last_validated_unix_nanos: Some(300),
                diagnostic_baseline_raw_score_total: Some(1000),
                diagnostic_candidate_raw_score_total: Some(850),
                score_delta: -150,
                confidence_milli: 900,
                environment: DaemonProfileEnvironment::default(),
                partition: DaemonProfilePartition {
                    power_source: Some("ac".to_owned()),
                    scheduler_label: Some("scx_lavd".to_owned()),
                    ..DaemonProfilePartition::default()
                },
            }],
        },
        ..DaemonState::default()
    };

    let decoded: DaemonState =
        serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();

    assert_eq!(decoded.mode, DaemonMode::ApplyLowRisk);
    assert_eq!(decoded.phase, DaemonPhase::Measure);
    assert_eq!(
        decoded
            .active_target
            .as_ref()
            .and_then(|target| target.root_pid),
        Some(1234)
    );
    assert!(decoded.active_rollback.unwrap().rollback_available);
    assert_eq!(decoded.degraded.len(), 1);
    assert_eq!(decoded.profile_memory.profiles.len(), 1);
    assert_eq!(
        decoded.profile_memory.profiles[0].workload_identity_hash,
        "workload-abc"
    );
}

#[test]
fn daemon_state_defaults_new_runtime_fields_when_loading_older_snapshots() {
    let json = r#"{
        "schema_version": 1,
        "mode": "observe",
        "phase": "disabled",
        "cooldown_until_unix_nanos": null,
        "active_target": null,
        "active_experiment": null,
        "active_rollback": null,
        "last_decision": null,
        "degraded": [],
        "faulted": null
    }"#;

    let decoded: DaemonState = serde_json::from_str(json).unwrap();

    assert_eq!(
        decoded.health.state,
        crate::daemon::health::SystemHealthState::Healthy
    );
    assert!(decoded.health.ok_for_apply);
    assert!(decoded.profile_memory.profiles.is_empty());
}

#[test]
fn profile_environment_hashes_kernel_scx_and_topology() {
    let metadata = SystemMetadata {
        kernel_osrelease: Some("6.12.0".to_owned()),
        cpu_online: Some("0-3".to_owned()),
        cpu_possible: Some("0-3".to_owned()),
        scx_state: Some("enabled".to_owned()),
        scx_ops: Some("scx_lavd".to_owned()),
        cpu_topology: vec![crate::metadata::CpuTopology {
            cpu: 0,
            thread_siblings_list: Some("0,2".to_owned()),
            core_id: Some("0".to_owned()),
            physical_package_id: Some("0".to_owned()),
        }],
        ..SystemMetadata::default()
    };

    let environment = DaemonProfileEnvironment::from_system_metadata(&metadata);
    let repeated = DaemonProfileEnvironment::from_system_metadata(&metadata);

    assert_eq!(environment, repeated);
    assert_eq!(environment.kernel_version.as_deref(), Some("6.12.0"));
    assert_eq!(environment.scheduler_label.as_deref(), Some("scx_lavd"));
    assert!(environment.hardware_fingerprint.is_some());
    assert!(environment.cpu_topology_hash.is_some());
}

#[test]
fn workload_profile_validation_detects_environment_change_and_age() {
    let stored_environment = DaemonProfileEnvironment {
        hardware_fingerprint: Some("hardware-a".to_owned()),
        kernel_version: Some("6.12.0".to_owned()),
        cpu_topology_hash: Some("topology-a".to_owned()),
        scx_ops: Some("scx_lavd".to_owned()),
        scheduler_label: Some("scx_lavd".to_owned()),
        ..DaemonProfileEnvironment::default()
    };
    let current_environment = DaemonProfileEnvironment {
        hardware_fingerprint: Some("hardware-a".to_owned()),
        kernel_version: Some("6.13.0".to_owned()),
        cpu_topology_hash: Some("topology-b".to_owned()),
        scx_ops: Some("scx_bpfland".to_owned()),
        scheduler_label: Some("scx_bpfland".to_owned()),
        ..DaemonProfileEnvironment::default()
    };
    let profile = DaemonWorkloadProfile {
        workload_identity_hash: "workload-abc".to_owned(),
        workload_label: Some("game".to_owned()),
        candidate_name: "game-main".to_owned(),
        action_id: "cpu-affinity-profile:game-main".to_owned(),
        action_kind: "cpu_affinity_profile".to_owned(),
        safety_class: SafetyClass::ReversibleLowRisk,
        kept_unix_nanos: 100,
        last_validated_unix_nanos: Some(100),
        diagnostic_baseline_raw_score_total: Some(1000),
        diagnostic_candidate_raw_score_total: Some(850),
        score_delta: -150,
        confidence_milli: 900,
        environment: stored_environment,
        partition: DaemonProfilePartition::default(),
    };

    let validation = profile.validation(
        &current_environment,
        100 + PROFILE_REVALIDATE_AFTER_NANOS + 1,
    );

    assert!(!validation.valid);
    assert!(
        validation
            .reason_codes
            .contains(&"kernel_changed".to_owned())
    );
    assert!(
        validation
            .reason_codes
            .contains(&"cpu_topology_changed".to_owned())
    );
    assert!(
        validation
            .reason_codes
            .contains(&"scx_ops_changed".to_owned())
    );
    assert!(
        validation
            .reason_codes
            .contains(&"revalidation_due".to_owned())
    );
    assert!(validation.confidence_milli < 900);
}

#[test]
fn profile_memory_forget_filters_by_workload_and_candidate() {
    let profile = |workload: &str, candidate: &str| DaemonWorkloadProfile {
        workload_identity_hash: workload.to_owned(),
        workload_label: Some(workload.to_owned()),
        candidate_name: candidate.to_owned(),
        action_id: format!("cpu-affinity-profile:{candidate}"),
        action_kind: "cpu_affinity_profile".to_owned(),
        safety_class: SafetyClass::ReversibleLowRisk,
        kept_unix_nanos: 1,
        last_validated_unix_nanos: Some(1),
        diagnostic_baseline_raw_score_total: None,
        diagnostic_candidate_raw_score_total: None,
        score_delta: 0,
        confidence_milli: 800,
        environment: DaemonProfileEnvironment::default(),
        partition: DaemonProfilePartition::default(),
    };
    let mut memory = DaemonProfileMemory {
        profiles: vec![
            profile("workload-a", "candidate-a"),
            profile("workload-a", "candidate-b"),
            profile("workload-b", "candidate-a"),
        ],
    };

    let removed = memory.forget_matching(Some("workload-a"), Some("candidate-a"), false);

    assert_eq!(removed.len(), 1);
    assert_eq!(memory.profiles.len(), 2);
    assert!(
        memory
            .profiles
            .iter()
            .all(|profile| profile.workload_identity_hash != "workload-a"
                || profile.candidate_name != "candidate-a")
    );
}

#[test]
fn daemon_state_snapshot_writer_atomically_writes_json_and_removes_temp_file() {
    let dir = temp_dir("snapshot-writer");
    let path = dir.join("daemon_state.json");
    let writer = DaemonStateSnapshotWriter::new(&path);
    let state = DaemonState {
        mode: DaemonMode::ApplyLowRisk,
        phase: DaemonPhase::Cooldown,
        cooldown_until_unix_nanos: Some(9_000),
        degraded: vec![DaemonDegradedStatus {
            category: "data_quality".to_owned(),
            message: "low data quality".to_owned(),
        }],
        ..DaemonState::default()
    };

    writer.write(&state).unwrap();

    let decoded = load_daemon_state(&path).unwrap();

    assert_eq!(writer.path(), path.as_path());
    assert_eq!(decoded.mode, DaemonMode::ApplyLowRisk);
    assert_eq!(decoded.phase, DaemonPhase::Cooldown);
    assert_eq!(decoded.cooldown_until_unix_nanos, Some(9_000));
    assert_eq!(decoded.degraded[0].category, "data_quality");
    assert!(temporary_files_in(&dir).is_empty());

    fs::remove_dir_all(dir).ok();
}

#[test]
fn daemon_state_snapshot_writer_does_not_use_fixed_temp_path() {
    let dir = temp_dir("fixed-temp-sentinel");
    let path = dir.join("daemon_state.json");
    let fixed_temp_path = dir.join("daemon_state.json.tmp");
    fs::write(&fixed_temp_path, "sentinel").unwrap();

    let writer = DaemonStateSnapshotWriter::new(&path);
    let state = DaemonState {
        mode: DaemonMode::ApplyLowRisk,
        phase: DaemonPhase::Cooldown,
        ..DaemonState::default()
    };

    writer.write(&state).unwrap();

    assert_eq!(fs::read_to_string(&fixed_temp_path).unwrap(), "sentinel");
    assert_eq!(
        load_daemon_state(&path).unwrap().phase,
        DaemonPhase::Cooldown
    );

    fs::remove_dir_all(dir).ok();
}

#[test]
fn load_daemon_state_rejects_unsupported_schema_version() {
    let dir = temp_dir("unsupported-schema");
    let path = dir.join("daemon_state.json");
    let state = DaemonState {
        schema_version: DAEMON_STATE_SCHEMA_VERSION + 1,
        ..DaemonState::default()
    };

    serde_json::to_writer_pretty(fs::File::create(&path).unwrap(), &state).unwrap();

    let err = load_daemon_state(&path).unwrap_err();

    assert!(
        err.to_string()
            .contains("unsupported daemon state snapshot schema_version")
    );

    fs::remove_dir_all(dir).ok();
}

#[test]
fn default_daemon_state_snapshot_path_matches_autotune_state_directory() {
    let path = default_daemon_state_snapshot_path();
    let rendered = path.to_string_lossy();

    assert!(rendered.ends_with(".local/state/stutter/autotune/daemon_state.json"));
}

#[test]
fn decision_state_accepts_legacy_diagnostic_score_total_name() {
    let json = r#"{
        "decision": "candidate_applied",
        "reason": "candidate passed gates",
        "unix_nanos": 200,
        "diagnostic_score_total": 300
    }"#;

    let decision: DaemonDecisionState = serde_json::from_str(json).unwrap();

    assert_eq!(decision.diagnostic_current_raw_score_total, Some(300));
}

#[test]
fn decision_state_serializes_current_raw_score_total_name() {
    let decision = DaemonDecisionState {
        decision: "candidate_applied".to_owned(),
        reason: "candidate passed gates".to_owned(),
        unix_nanos: Some(200),
        diagnostic_current_raw_score_total: Some(300),
        candidate_count: None,
        top_denied_reason: None,
        planner: None,
        situation: None,
        focus_kind: None,
    };

    let json = serde_json::to_string(&decision).unwrap();

    assert!(json.contains("diagnostic_current_raw_score_total"));
    assert!(!json.contains("diagnostic_score_total"));
}

#[test]
fn workload_profile_accepts_legacy_candidate_diagnostic_score_total_name() {
    let json = r#"{
        "workload_identity_hash": "workload-abc",
        "workload_label": "game",
        "candidate_name": "game-main",
        "action_id": "cpu-affinity-profile:game-main",
        "action_kind": "cpu_affinity_profile",
        "safety_class": "ReversibleLowRisk",
        "kept_unix_nanos": 300,
        "last_validated_unix_nanos": 300,
        "diagnostic_baseline_raw_score_total": 1000,
        "diagnostic_candidate_diagnostic_score_total": 850,
        "score_delta": -150,
        "confidence_milli": 900,
        "environment": {},
        "partition": {}
    }"#;

    let profile: DaemonWorkloadProfile = serde_json::from_str(json).unwrap();

    assert_eq!(profile.diagnostic_candidate_raw_score_total, Some(850));
}

#[test]
fn workload_profile_serializes_candidate_raw_score_total_name() {
    let profile = DaemonWorkloadProfile {
        workload_identity_hash: "workload-abc".to_owned(),
        workload_label: Some("game".to_owned()),
        candidate_name: "game-main".to_owned(),
        action_id: "cpu-affinity-profile:game-main".to_owned(),
        action_kind: "cpu_affinity_profile".to_owned(),
        safety_class: SafetyClass::ReversibleLowRisk,
        kept_unix_nanos: 300,
        last_validated_unix_nanos: Some(300),
        diagnostic_baseline_raw_score_total: Some(1000),
        diagnostic_candidate_raw_score_total: Some(850),
        score_delta: -150,
        confidence_milli: 900,
        environment: DaemonProfileEnvironment::default(),
        partition: DaemonProfilePartition::default(),
    };

    let json = serde_json::to_string(&profile).unwrap();

    assert!(json.contains("diagnostic_candidate_raw_score_total"));
    assert!(!json.contains("diagnostic_candidate_diagnostic_score_total"));
}
