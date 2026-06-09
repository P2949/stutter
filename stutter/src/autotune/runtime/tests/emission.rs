use super::*;

#[test]
fn dry_run_all_safe_writes_plan_files_without_starting_experiment() {
    let plan_dir = temp_runtime_plan_dir("suggest-mode");
    let current_pid = std::process::id();
    let mut config = AutotuneRuntimeConfig::suggest(None, Some(current_pid), None)
        .with_profiles(vec![low_risk_profile()])
        .with_dry_run_all_safe(true)
        .with_dry_run_plan_dir(plan_dir.clone());
    config.history_log = None;
    let mut runtime = AutotuneRuntime::new(config);
    let mut observation = high_quality_game_observation_with_focus_confidence(0.95);
    observation.target_root_pid = Some(current_pid);
    observation.active_target_count = 1;
    observation.active_tasks = vec![active_task_snapshot(
        current_pid,
        current_pid,
        "game",
        TaskClass::Game,
    )];

    let candidate = runtime
        .select_candidate_for_observation(&observation)
        .unwrap();
    let decision = decide_autotune_transition(
        &runtime.controller.policy,
        &runtime.controller.state,
        &observation,
        candidate,
    );

    runtime
        .apply_decision_side_effects(&observation, &decision, "dry-run-all-safe test")
        .unwrap();

    assert!(!runtime.has_active_experiment());
    assert!(runtime.controller.state.active_experiment.is_none());
    assert!(runtime.pending_history_context.is_none());
    assert!(
        !runtime.last_dry_run_plan_files.is_empty(),
        "expected at least one candidate plan file from dry-run planner results"
    );
    for plan in &runtime.last_dry_run_plan_files {
        assert!(plan.path.starts_with(&plan_dir));
        assert!(
            plan.path.exists(),
            "missing plan file {}",
            plan.path.display()
        );
        assert_eq!(plan.safety_class, SafetyClass::ReversibleLowRisk);
        let decoded: crate::autotune::planning::plan_io::CandidatePlanFile =
            serde_json::from_slice(&std::fs::read(&plan.path).unwrap()).unwrap();
        assert_eq!(
            decoded.policy_intent,
            crate::daemon_policy::PolicyIntent::Suggest
        );
        assert!(
            decoded.apply_command.is_none(),
            "suggest-mode plan files must not advertise direct apply"
        );
        assert!(decoded.policy_explanation.final_reason.contains("allowed"));
    }
}

#[test]
fn runtime_reports_active_experiment_state() {
    let mut runtime = runtime();

    assert!(!runtime.has_active_experiment());

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

    runtime
        .live_experiments
        .set_current_for_tests(LiveExperiment {
            experiment_id: ExperimentId::new("experiment-active"),
            safety_class: candidate.safety_class(),
            mode: DaemonMode::ApplyLowRisk,
            candidate,
            baseline_score,
            baseline_signals: ObjectiveSignals::default(),
            baseline_active_config: None,
            applied_unix_nanos: 1_000,
            washout_until_unix_nanos: 2_000,
            measure_until_unix_nanos: 3_000,
            rollback: RollbackToken::CpuAffinityRestoreFile {
                path: PathBuf::from("/tmp/stutter-active-restore.json"),
                affected_tasks: 1,
            },
        });

    assert!(runtime.has_active_experiment());
}
