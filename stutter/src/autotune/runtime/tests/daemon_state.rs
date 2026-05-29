use super::*;

#[test]
fn daemon_phase_from_controller_phase_maps_all_controller_phases() {
    let cases = [
        (ControllerPhase::Disabled, DaemonPhase::Disabled),
        (ControllerPhase::Observing, DaemonPhase::Observe),
        (ControllerPhase::Planning, DaemonPhase::Decide),
        (ControllerPhase::Applying, DaemonPhase::Apply),
        (ControllerPhase::Measuring, DaemonPhase::Measure),
        (ControllerPhase::Keeping, DaemonPhase::Keep),
        (ControllerPhase::Reverting, DaemonPhase::Rollback),
        (ControllerPhase::Cooldown, DaemonPhase::Cooldown),
        (ControllerPhase::Faulted, DaemonPhase::Faulted),
    ];

    for (controller_phase, expected_daemon_phase) in cases {
        assert_eq!(
            daemon_phase_from_controller_phase(controller_phase),
            expected_daemon_phase
        );
    }
}

#[test]
fn daemon_state_snapshot_serializes_live_runtime_state() {
    let mut runtime = AutotuneRuntime::new(
        AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), None)
            .with_profiles(vec![low_risk_profile()]),
    );
    let candidate = CandidateAction::cpu_affinity_profile(low_risk_profile(), 1234);
    let baseline_score = WindowScore {
        started_unix_nanos: 100,
        finished_unix_nanos: 200,
        interval_count: 1,
        scored_samples: 100,
        scored_task_count: 1,
        score: StutterScore {
            total: 500,
            over_1ms: 10,
            over_2ms: 5,
            over_5ms: 1,
            ..StutterScore::default()
        },
    };
    let rollback = RollbackToken::CpuAffinityRestoreFile {
        path: PathBuf::from("/tmp/stutter-restore.json"),
        affected_tasks: 2,
    };

    runtime.target_state = RuntimeTargetState {
        root_pid: Some(1234),
        active_targets: 2,
        target_comm: Some("game".to_owned()),
        active_tasks: BTreeMap::new(),
    };
    runtime.latest_focus = Some(AutotuneObservationFocus {
        kind: FocusGroupKind::Game,
        root_pids: vec![1234],
        member_pids: vec![1234, 1235],
        confidence: 0.95,
        situation: SituationKind::GameCpuSchedulerPressure,
        reasons: vec!["game focus selected".to_owned()],
    });
    runtime.latest_drop_counters = DropCountersSnapshot {
        ringbuf_reserve_failed: 7,
        ..DropCountersSnapshot::default()
    };
    runtime.recent_diagnoses.push_back(LiveDiagnosisEntry {
        elapsed_ms: 12_345,
        cause: StutterCause::GpuBoundCandidate,
        confidence: Confidence::High,
        anchor_class: TaskClass::Game,
        anchor_comm: "game".to_owned(),
        evidence: vec!["gpu busy".to_owned()],
    });
    runtime
        .live_experiments
        .set_current_for_tests(LiveExperiment {
            experiment_id: ExperimentId::new("experiment-1"),
            safety_class: candidate.safety_class(),
            mode: DaemonMode::ApplyLowRisk,
            candidate,
            baseline_score,
            baseline_signals: ObjectiveSignals::default(),
            baseline_active_config: None,
            applied_unix_nanos: 1_000,
            washout_until_unix_nanos: 2_000,
            measure_until_unix_nanos: 3_000,
            rollback,
        });
    runtime.controller.state.phase = ControllerPhase::Faulted;
    runtime.controller.state.cooldown_until_unix_nanos = Some(9_000);

    let mut observation = high_quality_game_observation_with_focus_confidence(0.95);
    observation.data_quality = OnlineDataQuality::Low {
        reasons: vec!["low scored samples".to_owned()],
    };
    observation.drop_counter_total = 7;
    observation.score.total = 999;
    runtime.last_observation = observation;
    runtime.last_decision = Some(AutotuneDecisionStreamEntry {
        unix_nanos: 8_000,
        phase: "Faulted".to_owned(),
        mode: "ApplyLowRisk".to_owned(),
        focus_kind: Some("Game".to_owned()),
        focus_confidence: 0.95,
        target_root_pid: Some(1234),
        active_target_count: 2,
        situation: "GameCpuSchedulerPressure".to_owned(),
        situation_confidence: 0.95,
        situation_evidence: Vec::new(),
        situation_blockers: Vec::new(),
        protected_tasks_count: 0,
        candidate_count: 0,
        top_denied_reason: None,
        planner: None,
        dry_run_plan_files: Vec::new(),
        diagnostic_raw_score_total: 999,
        data_quality: "Low: low scored samples".to_owned(),
        data_quality_reason_codes: vec!["measurement_uncertain".to_owned()],
        decision: "faulted".to_owned(),
        reason: "rollback failed".to_owned(),
    });
    let kept_candidate = CandidateAction::cpu_affinity_profile(low_risk_profile(), 1234);
    runtime.controller.state.record_candidate_result(
        crate::autotune::controller::ControllerCandidateResultInput {
            candidate: &kept_candidate,
            observation: &runtime.last_observation,
            cpu_topology_signature: None,
            result: CandidateMemoryResult::Kept,
            diagnostic_baseline_raw_score_total: Some(500),
            diagnostic_current_raw_score_total: Some(400),
            rollback_reason: None,
            cooldown_expires_unix_nanos: None,
        },
    );

    let snapshot = runtime.daemon_state_snapshot();
    let value = serde_json::to_value(&snapshot).unwrap();

    assert_eq!(snapshot.mode, DaemonMode::ApplyLowRisk);
    assert_eq!(snapshot.phase, DaemonPhase::Faulted);
    assert_eq!(snapshot.cooldown_until_unix_nanos, Some(9_000));
    assert_eq!(
        snapshot
            .active_target
            .as_ref()
            .and_then(|target| target.root_pid),
        Some(1234)
    );
    assert_eq!(
        snapshot
            .active_experiment
            .as_ref()
            .map(|experiment| experiment.experiment_id.as_str()),
        Some("experiment-1")
    );
    assert_eq!(
        snapshot
            .active_rollback
            .as_ref()
            .map(|rollback| rollback.rollback_available),
        Some(true)
    );
    assert_eq!(
        snapshot
            .last_decision
            .as_ref()
            .map(|decision| decision.decision.as_str()),
        Some("faulted")
    );
    assert_eq!(
        snapshot.faulted.as_ref().map(|fault| fault.reason.as_str()),
        Some("rollback failed")
    );
    assert!(
        snapshot
            .degraded
            .iter()
            .any(|status| status.category == "data_quality")
    );
    assert!(snapshot.degraded.iter().any(|status| {
        status.category == "data_quality"
            && status
                .message
                .contains("reason_codes=measurement_uncertain")
    }));
    assert!(
        snapshot
            .degraded
            .iter()
            .any(|status| status.category == "drop_counters")
    );
    assert!(!snapshot.health.ok_for_apply);
    assert_eq!(
        snapshot.health.reason_code.as_deref(),
        Some("drop_counters_high")
    );
    assert!(
        snapshot
            .degraded
            .iter()
            .any(|status| status.category == "cooldown")
    );
    assert!(
        snapshot
            .degraded
            .iter()
            .any(|status| status.category == "recent_diagnosis")
    );

    assert_eq!(value["schema_version"], DAEMON_STATE_SCHEMA_VERSION);
    assert_eq!(value["mode"], "apply-low-risk");
    assert_eq!(value["phase"], "faulted");
    assert_eq!(value["cooldown_until_unix_nanos"].as_u64(), Some(9_000));
    assert_eq!(value["active_target"]["comm"], "game");
    assert_eq!(
        value["active_experiment"]["action_id"],
        "cpu-affinity-profile:game-low-risk"
    );
    assert_eq!(
        value["active_rollback"]["token"]["kind"],
        "cpu-affinity-restore-file"
    );
    assert_eq!(
        value["last_decision"]["diagnostic_current_raw_score_total"].as_u64(),
        Some(999)
    );
    assert_eq!(
        value["active_rollback"]["manual_restore_command"],
        "stutter daemon emergency-restore"
    );
    assert_eq!(
        value["faulted"]["manual_restore_command"],
        "stutter daemon emergency-restore"
    );
    assert_eq!(snapshot.profile_memory.profiles.len(), 1);
    assert_eq!(
        snapshot.profile_memory.profiles[0].candidate_name,
        "game-low-risk"
    );
    assert_eq!(
        snapshot.profile_memory.profiles[0]
            .workload_label
            .as_deref(),
        Some("game")
    );
    assert_eq!(
        value["profile_memory"]["profiles"][0]["action_kind"],
        "cpu_affinity_profile"
    );
}
