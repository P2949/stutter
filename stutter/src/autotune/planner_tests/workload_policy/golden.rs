use super::support::*;

#[test]
fn planner_golden_cases() {
    let fixture_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("testdata/autotune/planner");
    let mut paths = std::fs::read_dir(&fixture_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect::<Vec<_>>();
    paths.sort();

    let expected_names = vec![
        "browser_cpu_pressure.json",
        "browser_focused.json",
        "browser_gpu_video.json",
        "browser_io_pressure.json",
        "browser_memory_pressure.json",
        "compile_cpu_bound.json",
        "compile_linker_pressure.json",
        "compositor_pressure.json",
        "cooldown_active.json",
        "critical_realtime_present.json",
        "external_mutation_detected.json",
        "game_cpu_scheduler_pressure.json",
        "game_gpu_bound.json",
        "game_gpu_power_limited.json",
        "game_gpu_profile_switch_medium_risk.json",
        "game_idle_suppressed.json",
        "game_irq_gpu_medium_risk.json",
        "game_irq_pressure_signals_present.json",
        "io_pressure.json",
        "irq_pressure.json",
        "kept_action_conflict.json",
        "low_data_quality.json",
        "media_playback.json",
        "memory_pressure_swappiness_medium_risk.json",
        "recording_active.json",
        "thermal_degraded.json",
        "virtual_machine_load.json",
    ];
    let actual_names = paths
        .iter()
        .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert_eq!(actual_names, expected_names);

    for path in paths {
        let text = std::fs::read_to_string(&path).unwrap();
        let case: PlannerGoldenCase = serde_json::from_str(&text)
            .unwrap_or_else(|err| panic!("failed to parse {}: {err}", path.display()));
        run_planner_golden_case(&path, case);
    }
}

fn run_planner_golden_case(path: &std::path::Path, case: PlannerGoldenCase) {
    let mut observation = build_fixture_observation(&case);
    let policy = build_fixture_policy(&case.policy, observation.target_root_pid);
    let profiles = fixture_profiles(&case);
    let workload_policy = fixture_workload_policy(&case);
    let planner = CandidatePlanner::default_for_policy(&policy);

    let mut controller_state = ControllerRuntimeState::default();
    let state_candidate =
        state_candidate_for_action_kind(first_fixture_action_kind(&case), &profiles);

    if case.cooldown_active {
        controller_state.record_candidate_result(
            crate::autotune::controller::ControllerCandidateResultInput {
                candidate: &state_candidate,
                observation: &observation,
                cpu_topology_signature: None,
                result: CandidateMemoryResult::Reverted,
                diagnostic_baseline_raw_score_total: Some(100),
                diagnostic_current_raw_score_total: Some(120),
                rollback_reason: Some("fixture cooldown".to_owned()),
                cooldown_expires_unix_nanos: Some(observation.now_unix_nanos + 10_000),
            },
        );
    }

    if case.external_mutation {
        controller_state.active_experiment = Some(ActiveExperiment {
            experiment_id: ExperimentId::new("fixture-external-mutation"),
            candidate: state_candidate.clone(),
            baseline_score: window_score(100),
        });
        observation.active_config_snapshot =
            Some(active_nice_snapshot_for_tasks(&observation.active_tasks, 0));
    }

    if case.kept_conflict {
        observation.active_config_snapshot = None;
    }

    let active_profile_state = case.kept_conflict.then(|| {
        active_profile_state_with_kept(KeptCandidateState::new(
            ExperimentId::new("fixture-kept-conflict"),
            state_candidate.clone(),
            window_score(100),
            window_score(90),
            rollback_token(),
            observation.now_unix_nanos,
            "fixture kept conflict",
        ))
    });

    let mut dry_runner = CountingDryRunner::default();
    let result = planner.plan_with_dry_runner(
        PlannerInput {
            observation: &observation,
            daemon_policy: &policy,
            capabilities: &observation.capabilities,
            system_health: &observation.system_health,
            controller_state: &controller_state,
            active_profile_state: active_profile_state.as_ref(),
            workload_policy: &workload_policy,
            profiles: &profiles,
        },
        &mut dry_runner,
    );

    let selected_action_kind = result
        .selected
        .as_ref()
        .map(|candidate| candidate.action_kind().to_owned());
    assert_eq!(
        selected_action_kind,
        case.expected_selected_action_kind,
        "fixture {} selected action mismatch; evaluations={:#?}",
        path.display(),
        result.evaluations
    );

    if let Some(selected) = result.selected.as_ref() {
        assert!(
            !selected.is_high_risk_system_adjacent(),
            "fixture {} selected high-risk/system-adjacent candidate {}",
            path.display(),
            selected.candidate_name()
        );
    }

    assert_eq!(
        result.evaluations.len(),
        case.expected_total_proposals,
        "fixture {} total proposal mismatch; evaluations={:#?}",
        path.display(),
        result.evaluations
    );

    assert_eq!(
        result
            .evaluations
            .iter()
            .filter(|evaluation| evaluation.eligible)
            .count(),
        case.expected_eligible_proposals,
        "fixture {} eligible proposal mismatch; evaluations={:#?}",
        path.display(),
        result.evaluations
    );

    let actual_action_kinds = result
        .evaluations
        .iter()
        .map(|evaluation| evaluation.action_kind.clone())
        .collect::<Vec<_>>();
    let expected_action_kinds = case
        .expected_evaluations
        .iter()
        .map(|evaluation| evaluation.action_kind.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        actual_action_kinds,
        expected_action_kinds,
        "fixture {} action-kind list changed; evaluations={:#?}",
        path.display(),
        result.evaluations
    );

    for expected in &case.expected_evaluations {
        let evaluation = result
            .evaluations
            .iter()
            .find(|evaluation| evaluation.action_kind == expected.action_kind)
            .unwrap_or_else(|| {
                panic!(
                    "fixture {} missing evaluation for {}",
                    path.display(),
                    expected.action_kind
                )
            });

        let expected_objective =
            crate::autotune::workload_policy::parse_objective_kind(&expected.objective)
                .unwrap_or_else(|err| {
                    panic!(
                        "fixture {} has invalid objective {}: {err}",
                        path.display(),
                        expected.objective
                    )
                });
        assert_eq!(
            evaluation.objective,
            expected_objective,
            "fixture {} objective changed for {}",
            path.display(),
            expected.action_kind
        );

        assert_eq!(
            evaluation.eligible,
            expected.eligible,
            "fixture {} eligibility changed for {}; evaluation={:#?}",
            path.display(),
            expected.action_kind,
            evaluation
        );

        assert!(
            evaluation.confidence >= expected.min_confidence
                && evaluation.confidence <= expected.max_confidence,
            "fixture {} confidence {:.3} outside expected range [{:.3}, {:.3}] for {}",
            path.display(),
            evaluation.confidence,
            expected.min_confidence,
            expected.max_confidence,
            expected.action_kind
        );

        let actual_dry_run_affected_tasks = evaluation
            .dry_run
            .as_ref()
            .map(|state| state.affected_tasks);
        assert_eq!(
            actual_dry_run_affected_tasks,
            expected.dry_run_affected_tasks,
            "fixture {} dry-run behavior changed for {}; evaluation={:#?}",
            path.display(),
            expected.action_kind,
            evaluation
        );

        assert_eq!(
            evaluation.candidate.manual_only_reason().is_some(),
            expected.manual_only,
            "fixture {} manual-only flag changed for {}",
            path.display(),
            expected.action_kind
        );

        let mut actual_reason_codes = evaluation
            .deny_reasons
            .iter()
            .map(CandidateDenyReason::reason_code)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        actual_reason_codes.sort();

        let mut expected_reason_codes = expected.deny_reason_codes.clone();
        expected_reason_codes.sort();

        assert_eq!(
            actual_reason_codes,
            expected_reason_codes,
            "fixture {} deny reasons changed for {}; evaluation={:#?}",
            path.display(),
            expected.action_kind,
            evaluation
        );
    }

    let summary = result.summary();
    assert_eq!(summary.total_proposals, case.expected_total_proposals);
    assert_eq!(summary.eligible_proposals, case.expected_eligible_proposals);
}
