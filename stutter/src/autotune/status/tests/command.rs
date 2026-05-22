use super::{
    support::{kept_event, temp_dir},
    *,
};

#[test]
fn load_autotune_status_prefers_daemon_state_snapshot_when_present() {
    let dir = temp_dir("prefers-daemon-state");
    let history_path = dir.join("history.jsonl");
    let event = kept_event();
    let mut file = fs::File::create(&history_path).unwrap();
    serde_json::to_writer(&mut file, &event).unwrap();
    file.write_all(b"\n").unwrap();

    let daemon_state_path = dir.join("daemon_state.json");
    let state = DaemonState {
        mode: DaemonMode::ApplyLowRisk,
        phase: DaemonPhase::Faulted,
        last_decision: Some(DaemonDecisionState {
            decision: "faulted".to_owned(),
            reason: "snapshot fault wins over history".to_owned(),
            unix_nanos: Some(300),
            diagnostic_score_total: Some(42),
            candidate_count: None,
            top_denied_reason: None,
            planner: None,
            situation: None,
            focus_kind: None,
        }),
        faulted: Some(DaemonFaultState {
            reason: "snapshot fault wins over history".to_owned(),
            manual_restore_command: Some("stutter restore".to_owned()),
        }),
        ..DaemonState::default()
    };
    DaemonStateSnapshotWriter::new(&daemon_state_path)
        .write(&state)
        .unwrap();

    let status = load_autotune_status(&history_path).unwrap();

    assert_eq!(status.phase, "faulted");
    assert_eq!(status.mode, "ApplyLowRisk");
    assert_eq!(status.current_score, Some(42));
    assert_eq!(
        status.last_fault.as_deref(),
        Some("snapshot fault wins over history")
    );
    assert_eq!(status.active_profile, None);
    assert_eq!(status.history_path, daemon_state_path);

    fs::remove_dir_all(dir).ok();
}

#[test]
fn command_reads_history_file_and_renders_text() {
    let dir = temp_dir("command-text");
    let path = dir.join("history.jsonl");
    let event = kept_event();
    let mut file = fs::File::create(&path).unwrap();
    serde_json::to_writer(&mut file, &event).unwrap();
    file.write_all(b"\n").unwrap();

    let status = load_autotune_status(&path).unwrap();

    assert_eq!(status.phase, "Cooldown");
    assert_eq!(
        status.active_profile.as_deref(),
        Some("game-main-suggested")
    );

    fs::remove_dir_all(dir).ok();
}
