use super::*;

#[test]
fn status_from_daemon_state_reports_snapshot_fields() {
    let state = DaemonState {
        mode: DaemonMode::ApplyLowRisk,
        phase: DaemonPhase::Measure,
        cooldown_until_unix_nanos: Some(
            crate::audit::unix_nanos_now().saturating_add(30_000_000_000),
        ),
        active_target: Some(DaemonTargetState {
            root_pid: Some(1234),
            active_targets: 2,
            comm: Some("KingdomCome.exe".to_owned()),
        }),
        active_experiment: Some(DaemonExperimentState {
            experiment_id: "experiment-1".to_owned(),
            action_id: "cpu-affinity-profile:game-main".to_owned(),
            candidate_name: Some("game-main".to_owned()),
            mode: DaemonMode::ApplyLowRisk,
            safety_class: SafetyClass::ReversibleLowRisk,
            started_unix_nanos: Some(100),
        }),
        active_rollback: Some(DaemonRollbackState {
            action_id: "cpu-affinity-profile:game-main".to_owned(),
            mode: DaemonMode::ApplyLowRisk,
            safety_class: SafetyClass::ReversibleLowRisk,
            rollback_available: true,
            token: Some(RollbackToken::CpuAffinityRestoreFile {
                path: PathBuf::from("/tmp/stutter-restore.json"),
                affected_tasks: 31,
            }),
            manual_restore_command: Some("stutter restore".to_owned()),
        }),
        last_decision: Some(DaemonDecisionState {
            decision: "candidate_applied".to_owned(),
            reason: "candidate is being measured".to_owned(),
            unix_nanos: Some(200),
            score_total: Some(818),
            candidate_count: None,
            top_denied_reason: None,
            planner: None,
            situation: None,
            focus_kind: None,
        }),
        degraded: vec![DaemonDegradedStatus {
            category: "data_quality".to_owned(),
            message: "Low: low scored samples".to_owned(),
        }],
        faulted: None,
        ..DaemonState::default()
    };

    let status = status_from_daemon_state(PathBuf::from("/tmp/daemon_state.json"), &state);

    assert_eq!(status.phase, "measure");
    assert_eq!(status.mode, "ApplyLowRisk");
    assert_eq!(
        status.target,
        Some(StatusTarget {
            comm: "KingdomCome.exe".to_owned(),
            pid: 1234,
        })
    );
    assert_eq!(status.active_profile, None);
    assert_eq!(status.active_candidate.as_deref(), Some("game-main"));
    assert_eq!(
        status.last_decision,
        "candidate_applied: candidate is being measured"
    );
    assert!(status.rollback_available);
    assert_eq!(
        status.last_rollback_path.as_deref(),
        Some("/tmp/stutter-restore.json")
    );
    assert!(status.cooldown_remaining_seconds.unwrap_or(0) > 0);
    assert_eq!(status.current_score, Some(818));
    assert_eq!(
        status.data_quality.as_deref(),
        Some("Low: low scored samples")
    );
    assert_eq!(status.manual_restore_command, "stutter restore");
}

#[test]
fn status_from_daemon_state_lists_all_profile_memory_kept_actions() {
    let profile =
        |candidate_name: &str, action_id: &str| crate::daemon::state::DaemonWorkloadProfile {
            workload_identity_hash: "workload-a".to_owned(),
            workload_label: Some("KingdomCome.exe".to_owned()),
            candidate_name: candidate_name.to_owned(),
            action_id: action_id.to_owned(),
            action_kind: action_id
                .split_once(':')
                .map(|(kind, _)| kind.replace('-', "_"))
                .unwrap_or_else(|| "unknown".to_owned()),
            safety_class: SafetyClass::ReversibleLowRisk,
            kept_unix_nanos: 100,
            last_validated_unix_nanos: Some(100),
            baseline_score_total: Some(1_000),
            candidate_score_total: Some(800),
            score_delta: -200,
            confidence_milli: 900,
            environment: crate::daemon::state::DaemonProfileEnvironment::default(),
            partition: crate::daemon::state::DaemonProfilePartition::default(),
        };
    let state = DaemonState {
        profile_memory: crate::daemon::state::DaemonProfileMemory {
            profiles: vec![
                profile("game-main", "cpu-affinity-profile:game-main"),
                profile("io-priority", "ionice:io-priority"),
            ],
        },
        ..DaemonState::default()
    };

    let status = status_from_daemon_state(PathBuf::from("/tmp/daemon_state.json"), &state);

    assert_eq!(status.kept_actions.len(), 2);
    assert!(
        status
            .kept_actions
            .iter()
            .any(|action| action.action_id == "cpu-affinity-profile:game-main")
    );
    assert!(
        status
            .kept_actions
            .iter()
            .any(|action| action.action_id == "ionice:io-priority")
    );
}

#[test]
fn status_from_daemon_state_renders_daemon_lifecycle_labels_and_candidates() {
    let base_state = DaemonState {
        mode: DaemonMode::ApplyLowRisk,
        active_experiment: Some(DaemonExperimentState {
            experiment_id: "experiment-1".to_owned(),
            action_id: "cpu-affinity-profile:game-main".to_owned(),
            candidate_name: Some("game-main".to_owned()),
            mode: DaemonMode::ApplyLowRisk,
            safety_class: SafetyClass::ReversibleLowRisk,
            started_unix_nanos: Some(100),
        }),
        ..DaemonState::default()
    };

    let cases = [
        (DaemonPhase::Disabled, "disabled", None, None),
        (DaemonPhase::Init, "init", None, None),
        (DaemonPhase::Recover, "recover", None, None),
        (DaemonPhase::Paused, "paused", None, None),
        (DaemonPhase::Observe, "observe", None, None),
        (DaemonPhase::Decide, "decide", None, Some("game-main")),
        (DaemonPhase::Apply, "apply", None, Some("game-main")),
        (DaemonPhase::Measure, "measure", None, Some("game-main")),
        (DaemonPhase::Keep, "keep", Some("game-main"), None),
        (DaemonPhase::Rollback, "rollback", None, None),
        (DaemonPhase::Cooldown, "cooldown", Some("game-main"), None),
        (DaemonPhase::Faulted, "faulted", None, None),
        (DaemonPhase::Shutdown, "shutdown", None, None),
    ];

    for (phase, expected_phase, expected_profile, expected_candidate) in cases {
        let mut state = base_state.clone();
        state.phase = phase;

        let status = status_from_daemon_state(PathBuf::from("/tmp/daemon_state.json"), &state);

        assert_eq!(status.phase, expected_phase);
        assert_eq!(status.active_profile.as_deref(), expected_profile);
        assert_eq!(status.active_candidate.as_deref(), expected_candidate);
    }
}

#[test]
fn status_from_daemon_state_includes_top_denied_reason() {
    let state = DaemonState {
        mode: DaemonMode::Suggest,
        phase: DaemonPhase::Decide,
        last_decision: Some(DaemonDecisionState {
            decision: "observed".to_owned(),
            reason: "no candidate selected".to_owned(),
            unix_nanos: Some(200),
            score_total: Some(818),
            candidate_count: Some(1),
            top_denied_reason: Some("NoEffectiveChange".to_owned()),
            planner: None,
            situation: Some("CompileCpuBound".to_owned()),
            focus_kind: Some("Compile".to_owned()),
        }),
        ..DaemonState::default()
    };

    let status = status_from_daemon_state(PathBuf::from("/tmp/daemon_state.json"), &state);

    assert_eq!(
        status.last_decision,
        "observed: no candidate selected; top_denied_reason=NoEffectiveChange"
    );

    let rendered = render_autotune_status_text(&status);
    assert!(rendered.contains("top_denied_reason=NoEffectiveChange"));

    let json = serde_json::to_string(&status).unwrap();
    assert!(json.contains("top_denied_reason=NoEffectiveChange"));
}

#[test]
fn status_from_daemon_state_includes_structured_planner_summary() {
    use crate::{
        actions::SafetyClass,
        autotune::{
            objective::ObjectiveKind,
            planner::{PlannerEvaluationSummary, PlannerSummary},
        },
        daemon_policy::ActionEffectScope,
    };

    let state = DaemonState {
        last_decision: Some(DaemonDecisionState {
            decision: "no_action".to_owned(),
            reason: "no eligible candidates".to_owned(),
            unix_nanos: Some(200),
            candidate_count: None,
            top_denied_reason: None,
            situation: None,
            focus_kind: None,
            planner: Some(PlannerSummary {
                selected: None,
                eligible_candidates: Vec::new(),
                eligible_proposals: 0,
                total_proposals: 1,
                grouped_denials: Vec::new(),
                missing_capabilities: Vec::new(),
                workload_blocked: Vec::new(),
                manual_only_suggestions: Vec::new(),
                top_denied_candidates: vec![PlannerEvaluationSummary {
                    candidate_name: "test-candidate".to_owned(),
                    action_kind: "test-kind".to_owned(),
                    provider: "test-provider".to_owned(),
                    objective: ObjectiveKind::StutterScore,
                    safety_class: SafetyClass::ReversibleLowRisk,
                    effect_scope: ActionEffectScope::LocalProcessTree,
                    confidence: 0.8,
                    eligible: false,
                    rank: Some(10),
                    deny_reasons: vec![
                        crate::autotune::planner::CandidateDenyReason::NoEffectiveChange,
                    ],
                    deny_reason_codes: vec!["no_effective_change".to_owned()],
                    deny_messages: vec!["no change".to_owned()],
                    dry_run_affected_tasks: None,
                    manual_only_reason: None,
                    evidence: vec!["test_signal=1.0 weight=1.00".to_owned()],
                }],
                no_action: Some(crate::autotune::planner::PlannerNoActionSummary {
                    reason: "all candidates denied".to_owned(),
                    total_proposals: 1,
                    eligible_proposals: 0,
                    grouped_denials: Vec::new(),
                    top_denied_candidates: vec![PlannerEvaluationSummary {
                        candidate_name: "test-candidate".to_owned(),
                        action_kind: "test-kind".to_owned(),
                        provider: "test-provider".to_owned(),
                        objective: ObjectiveKind::StutterScore,
                        safety_class: SafetyClass::ReversibleLowRisk,
                        effect_scope: ActionEffectScope::LocalProcessTree,
                        confidence: 0.8,
                        eligible: false,
                        rank: Some(10),
                        deny_reasons: vec![
                            crate::autotune::planner::CandidateDenyReason::NoEffectiveChange,
                        ],
                        deny_reason_codes: vec!["no_effective_change".to_owned()],
                        deny_messages: vec!["no change".to_owned()],
                        dry_run_affected_tasks: None,
                        manual_only_reason: None,
                        evidence: vec!["test_signal=1.0 weight=1.00".to_owned()],
                    }],
                    missing_capabilities: Vec::new(),
                    workload_blocked: Vec::new(),
                    manual_only_suggestions: Vec::new(),
                }),
            }),
            score_total: None,
        }),
        ..DaemonState::default()
    };

    let status = status_from_daemon_state(PathBuf::from("/tmp/daemon_state.json"), &state);
    let rendered = render_autotune_status_text(&status);
    let json = serde_json::to_string(&status).unwrap();

    assert!(rendered.contains("planner: total=1 eligible=0"));
    assert!(rendered.contains("planner_denied: candidate=test-candidate"));
    assert!(rendered.contains("objective=StutterScore"));
    assert!(rendered.contains("confidence=0.800"));
    assert!(rendered.contains("evidence=test_signal=1.0 weight=1.00"));
    assert!(json.contains("\"planner\""));
    assert!(json.contains("test_signal=1.0 weight=1.00"));

    let planner = status
        .planner
        .as_ref()
        .expect("planner summary should be present");
    let no_action = planner
        .no_action
        .as_ref()
        .expect("no_action summary should be present");
    assert_eq!(no_action.top_denied_candidates.len(), 1);
    assert_eq!(
        no_action.top_denied_candidates[0].candidate_name,
        "test-candidate"
    );
    assert_eq!(
        no_action.top_denied_candidates[0].evidence[0],
        "test_signal=1.0 weight=1.00"
    );
}
