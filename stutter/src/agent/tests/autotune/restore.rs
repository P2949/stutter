use super::*;

#[tokio::test]
async fn autotune_restore_returns_true_noop_when_nothing_is_active() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));

    let response = autotune_restore_handler(State(state.clone()), HeaderMap::new())
        .await
        .into_response();

    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let restore: crate::remote::AutotuneRestoreResponse = serde_json::from_slice(&body).unwrap();

    assert_eq!(status, StatusCode::OK);
    assert_eq!(restore.status, "nothing_to_restore");
    assert_eq!(restore.restored_actions, Some(0));
    assert_eq!(restore.skipped_actions, Some(0));
    assert_eq!(restore.failed_actions, Some(0));
    assert_eq!(restore.restored_records, Some(0));
    assert_eq!(restore.skipped_missing, Some(0));
    assert_eq!(restore.skipped_identity_mismatch, Some(0));
    assert_eq!(restore.failed_records, Some(0));

    let daemon_state = state.daemon_state.lock().await.clone();
    assert_eq!(
        daemon_state
            .last_decision
            .as_ref()
            .map(|decision| decision.decision.as_str()),
        Some("remote_autotune_nothing_to_restore")
    );
}

#[tokio::test]
async fn autotune_restore_without_auth_is_rejected() {
    let state = Arc::new(test_agent_state(
        "127.0.0.1:0".parse().unwrap(),
        Some("secret"),
    ));

    let response = autotune_restore_handler(State(state), HeaderMap::new())
        .await
        .into_response();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn autotune_restore_active_rollback_invokes_restore() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    let dir = agent_autotune_temp_dir("remote-restore-active");
    let journal_path = dir.join("controller_journal.json");
    let audit_path = dir.join("audit.jsonl");
    let history_path = dir.join("history.jsonl");
    let target = dir.join("sysfs-knob");
    std::fs::write(&target, "changed").unwrap();

    crate::autotune::controller_journal::write_controller_journal_applied(
        &journal_path,
        crate::autotune::experiment::ExperimentId::try_new("experiment-remote").unwrap(),
        crate::actions::ActionId::try_new("sysfs-restore:remote").unwrap(),
        crate::actions::RollbackToken::SysfsRestore {
            path: target.clone(),
            original_value: "original".to_owned(),
        },
    )
    .unwrap();

    let response = autotune_restore_authorized(
        state,
        AutotuneRestoreCommandInput {
            journal_path: Some(journal_path.clone()),
            audit_path: Some(audit_path),
            history_path: Some(history_path),
            dry_run: false,
        },
    )
    .await;
    let (status, restore) = decode_restore_response(response).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(restore.status, "restored");
    assert_eq!(restore.restored_actions, Some(1));
    assert_eq!(restore.restored_records, Some(1));
    assert_eq!(restore.failed_records, Some(0));
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
    assert!(
        crate::autotune::controller_journal::read_controller_journal(&journal_path)
            .unwrap()
            .is_clean()
    );
    std::fs::remove_dir_all(dir).ok();
}

#[tokio::test]
async fn autotune_restore_failure_returns_conflict_response() {
    let state = Arc::new(test_agent_state("127.0.0.1:0".parse().unwrap(), None));
    let dir = agent_autotune_temp_dir("remote-restore-failure");
    let journal_path = dir.join("controller_journal.json");
    let audit_path = dir.join("audit.jsonl");
    let history_path = dir.join("history.jsonl");
    let target = dir.join("missing").join("sysfs-knob");

    crate::autotune::controller_journal::write_controller_journal_applied(
        &journal_path,
        crate::autotune::experiment::ExperimentId::try_new("experiment-remote").unwrap(),
        crate::actions::ActionId::try_new("sysfs-restore:remote").unwrap(),
        crate::actions::RollbackToken::SysfsRestore {
            path: target,
            original_value: "original".to_owned(),
        },
    )
    .unwrap();

    let response = autotune_restore_authorized(
        state,
        AutotuneRestoreCommandInput {
            journal_path: Some(journal_path.clone()),
            audit_path: Some(audit_path),
            history_path: Some(history_path),
            dry_run: false,
        },
    )
    .await;
    let (status, restore) = decode_restore_response(response).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(restore.status, "restore_failed");
    assert_eq!(restore.failed_actions, Some(1));
    assert_eq!(restore.failed_records, Some(1));
    assert!(
        !crate::autotune::controller_journal::read_controller_journal(&journal_path)
            .unwrap()
            .is_clean()
    );
    std::fs::remove_dir_all(dir).ok();
}
