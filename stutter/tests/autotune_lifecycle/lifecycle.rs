use super::*;

#[tokio::test]
async fn apply_low_risk_fake_candidate_lifecycle_keeps_and_cleans_journal() -> anyhow::Result<()> {
    let dir = temp_dir();
    let history_path = dir.join("autotune-history.jsonl");
    let audit_path = dir.join("audit.jsonl");
    let journal_path = dir.join("controller-journal.json");

    let candidate = CandidateAction::fake(
        ActionId::new("test-fake".to_owned()),
        SafetyClass::ReversibleLowRisk,
    );
    let mut config = AutotuneRuntimeConfig::apply_low_risk(None, Some(1234), None)
        .with_simulated_candidates(vec![candidate])
        .with_simulated_action_effects()
        .with_candidate_window_seconds(1)
        .with_washout(0, 1);
    config.history_log = Some(history_path.clone());
    config.controller_journal_path = Some(journal_path.clone());

    let mut runtime = AutotuneRuntime::new(config);
    let mut active_targets = BTreeMap::new();
    active_targets.insert(1234.into(), game_task(1234));

    runtime.on_event(MonitorEvent::TargetSnapshot {
        elapsed_ms: 0,
        active_targets,
        removed_targets: Vec::new(),
    })?;
    runtime.on_event(MonitorEvent::FocusChanged {
        elapsed_ms: 0,
        old_kind: None,
        new_kind: FocusGroupKind::Game,
        root_pids: vec![1234],
        member_pids: vec![1234],
        confidence: 0.95,
        score: 1.0,
        situation: SituationKind::GameCpuSchedulerPressure,
        reasons: vec!["lifecycle test game focus".to_owned()],
    })?;

    runtime.on_event(interval_event(records(1_000, 4, 25, 5, 5, 2, 8_000_000)))?;
    assert_eq!(runtime.controller_state().phase, ControllerPhase::Observing);

    let started = runtime
        .on_event(interval_event(records(5_000, 1, 25, 5, 5, 2, 8_000_000)))?
        .expect("baseline window should start a fake experiment");
    assert_eq!(started.decision, "candidate_started");
    assert_eq!(runtime.controller_state().phase, ControllerPhase::Measuring);
    assert!(runtime.has_active_experiment());

    let kept = runtime
        .on_event(interval_event(records(6_000, 5, 25, 0, 0, 0, 500_000)))?
        .expect("candidate measurement should keep the improved fake action");
    assert_eq!(kept.decision, "candidate_kept");
    assert_eq!(runtime.controller_state().phase, ControllerPhase::Cooldown);
    assert_eq!(runtime.active_profile_state().kept_action_count(), 1);

    for tick in 0..53 {
        runtime.on_event(interval_event(records(
            12_000 + (tick * 1_000),
            1,
            25,
            0,
            0,
            0,
            500_000,
        )))?;
    }

    assert_ne!(runtime.controller_state().phase, ControllerPhase::Faulted);
    assert!(history_path.exists());
    assert!(!fs::read_to_string(&history_path)?.trim().is_empty());

    let restore = restore_known_autotune_actions(AutotuneRestoreCommandInput {
        journal_path: Some(journal_path.clone()),
        audit_path: Some(audit_path),
        history_path: Some(history_path),
        dry_run: false,
    })?;
    assert_eq!(restore.status, AutotuneRestoreStatus::Restored);
    assert!(read_controller_journal(&journal_path)?.is_clean());

    Ok(())
}
