use super::{support::kept_event, *};

#[test]
fn status_from_history_matches_text_example() {
    let status = status_from_history_events(PathBuf::from("/tmp/history.jsonl"), &[kept_event()]);

    assert_eq!(status.phase, "Cooldown");
    assert_eq!(status.mode, "ApplyLowRisk");
    assert_eq!(
        status.target,
        Some(StatusTarget {
            comm: "KingdomCome.exe".to_owned(),
            pid: 1234,
        })
    );
    assert_eq!(
        status.active_profile.as_deref(),
        Some("game-main-suggested")
    );
    assert_eq!(status.kept_actions.len(), 1);
    assert_eq!(
        status.kept_actions[0].action_id.as_str(),
        "cpu-affinity-profile:game-main-suggested"
    );
    assert_eq!(status.last_decision, "candidate_kept, improvement=18.2%");
    assert!(status.rollback_available);
    assert!(status.last_rollback_path.is_some());

    let rendered = render_autotune_status_text(&status);
    assert!(rendered.contains("phase: Cooldown"));
    assert!(rendered.contains("mode: ApplyLowRisk"));
    assert!(rendered.contains("target: KingdomCome.exe pid=1234"));
    assert!(rendered.contains("active_profile: game-main-suggested"));
    assert!(rendered.contains("kept_actions: game-main-suggested"));
    assert!(rendered.contains("last_decision: candidate_kept, improvement=18.2%"));
    assert!(rendered.contains("rollback_available: yes"));
    assert!(rendered.contains("last_rollback_path: "));
}

#[test]
fn json_status_serializes() {
    let status = status_from_history_events(PathBuf::from("/tmp/history.jsonl"), &[kept_event()]);

    let json = serde_json::to_string_pretty(&status).unwrap();

    assert!(json.contains("\"phase\": \"Cooldown\""));
    assert!(json.contains("\"mode\": \"ApplyLowRisk\""));
    assert!(json.contains("\"active_profile\": \"game-main-suggested\""));
    assert!(json.contains("\"kept_actions\""));
    assert!(json.contains("\"rollback_available\": true"));
}
