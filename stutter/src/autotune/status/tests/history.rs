use super::{support::kept_event, *};

#[test]
fn status_from_history_lists_multiple_kept_actions() {
    let mut second = kept_event();
    second.unix_nanos = 2;
    second.decision.candidate_name = Some("io-priority".to_owned());
    second.decision.action_kind = Some("ionice".to_owned());
    second.action_id = Some(crate::actions::ActionId::new("ionice:io-priority"));

    let status =
        status_from_history_events(PathBuf::from("/tmp/history.jsonl"), &[kept_event(), second]);

    assert_eq!(status.kept_actions.len(), 2);
    assert!(
        status
            .kept_actions
            .iter()
            .any(|action| action.action_id.as_str() == "cpu-affinity-profile:game-main-suggested")
    );
    assert!(
        status
            .kept_actions
            .iter()
            .any(|action| action.action_id.as_str() == "ionice:io-priority")
    );
}

#[test]
fn empty_history_reports_disabled_status() {
    let status = status_from_history_events(PathBuf::from("/tmp/history.jsonl"), &[]);

    assert_eq!(status.phase, "Disabled");
    assert_eq!(status.mode, "Observe");
    assert_eq!(status.target, None);
    assert_eq!(status.active_profile, None);
    assert_eq!(status.last_decision, "no autotune history found");
    assert!(!status.rollback_available);
    assert_eq!(status.last_rollback_path, None);
}

#[test]
fn rollback_event_clears_active_profile_and_rollback_available() {
    let mut rolled_back = kept_event();
    rolled_back.unix_nanos = 2;
    rolled_back.decision.decision = "Revert".to_owned();
    rolled_back.rollback_performed = true;
    rolled_back.reason = "regressed; rollback performed".to_owned();

    let status = status_from_history_events(
        PathBuf::from("/tmp/history.jsonl"),
        &[kept_event(), rolled_back],
    );

    assert_eq!(status.active_profile, None);
    assert!(!status.rollback_available);
    assert_eq!(status.last_rollback_path, None);
    assert_eq!(
        status.last_decision,
        "candidate_reverted, rollback performed"
    );
}

#[test]
fn status_reports_cooldown_remaining_from_history_policy_metadata() {
    let mut event = kept_event();
    event.decision.decision = "cooldown_entered".to_owned();
    event.decision.rollback_policy = format!(
        "rollback-on-restore;cooldown_until_unix_nanos={};manual_restore_command=stutter_autotune_restore",
        crate::audit::unix_nanos_now().saturating_add(60_000_000_000)
    );

    let status = status_from_history_events(PathBuf::from("/tmp/history.jsonl"), &[event]);

    assert!(status.cooldown_remaining_seconds.unwrap_or(0) > 0);
    assert!(status.cooldown_remaining_seconds.unwrap_or(0) <= 60);
}

#[test]
fn restored_event_clears_rollback_active_profile_and_last_fault() {
    let mut faulted = kept_event();
    faulted.unix_nanos = 2;
    faulted.phase = ControllerPhase::Faulted;
    faulted.decision.decision = "faulted".to_owned();
    faulted.reason = "rollback failed".to_owned();

    let mut restored = kept_event();
    restored.unix_nanos = 3;
    restored.decision.decision = "restored".to_owned();
    restored.rollback_performed = true;
    restored.reason = "manual restore succeeded".to_owned();

    let status = status_from_history_events(
        PathBuf::from("/tmp/history.jsonl"),
        &[kept_event(), faulted, restored],
    );

    assert_eq!(status.active_profile, None);
    assert_eq!(status.active_candidate, None);
    assert!(!status.rollback_available);
    assert_eq!(status.last_rollback_path, None);
    assert_eq!(status.last_fault, None);
    assert_eq!(status.last_decision, "restored, rollback performed");
}

#[test]
fn candidate_applied_event_sets_active_candidate_and_rollback_available() {
    let mut event = kept_event();
    event.phase = ControllerPhase::Measuring;
    event.decision.decision = "candidate_applied".to_owned();
    event.decision.rollback_policy =
        "rollback-on-restore;manual_restore_command=stutter_autotune_restore".to_owned();

    let status = status_from_history_events(PathBuf::from("/tmp/history.jsonl"), &[event]);

    assert_eq!(
        status.active_candidate.as_deref(),
        Some("game-main-suggested")
    );
    assert!(status.rollback_available);
    assert!(status.last_rollback_path.is_some());
}
