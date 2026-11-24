use super::*;

fn frame(at: u64, ms: f64) -> GuardEvent {
    GuardEvent::Frame {
        at,
        event: FrameEvent {
            elapsed_ms: at * 1000,
            frametime_ms: ms,
        },
    }
}

#[test]
fn guard_facts_are_cut_to_the_stretch_not_the_whole_attempt() {
    // A slow tail early, a clean stretch later: only the clean stretch counts for a window over it.
    let mut events: Vec<GuardEvent> = (100..110).map(|at| frame(at, 40.0)).collect();
    events.extend((200..210).map(|at| frame(at, 8.0)));
    let whole = guards_over(&events, 1, 0, 1_000);
    let clean = guards_over(&events, 1, 200, 210);
    let (whole, clean) = (whole, clean);
    // Frame facts need the other signals too; when the builder reports none of them observed, nothing is cut at all.
    assert_eq!(whole.is_some(), clean.is_some());
    if let (Some(whole), Some(clean)) = (whole, clean) {
        assert!(clean.frame_p99_ms < whole.frame_p99_ms);
        assert_eq!(clean.frame_samples, 10);
        assert_eq!(whole.frame_samples, 20);
    }
}

#[test]
fn a_truncated_or_absent_entry_yields_no_events_for_the_replay() {
    let process = AttemptProcess {
        slot: 0,
        pid: stutter_core::ids::Pid::from(5),
        start_ticks: 9,
        uid: 0,
        started_unix: 0,
        completed_unix: 1,
    };
    let doc = vec![serde_json::json!({
        "attempt": 0, "pid": 5, "start_ticks": 9, "truncated": true, "events": []
    })];
    assert!(events_of(&doc, &process).is_none());
    assert!(events_of(&[], &process).is_none());
}
