use super::*;

#[test]
fn foreground_pid_boosts_matching_game_group() {
    let foreground = foreground_snapshot(Some(11));
    let mut snapshot = foreground_scoring_snapshot(
        Some(foreground),
        vec![
            test_group(
                FocusGroupKind::Game,
                vec![10],
                vec![10, 11],
                Some(10),
                "game",
                0.30,
            ),
            test_group(
                FocusGroupKind::Browser,
                vec![20],
                vec![20, 21],
                Some(20),
                "browser",
                0.70,
            ),
        ],
    );

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Hybrid);

    let game = snapshot
        .groups
        .iter()
        .find(|group| group.display_name == "game")
        .unwrap();
    assert!(game.score_breakdown.foreground_score > 0.0);
    assert!(game.score > 0.30);
    assert!(
        game.reasons
            .iter()
            .any(|reason| reason.contains("foreground-window score"))
    );
}

#[test]
fn foreground_source_rejects_non_foreground_heuristic_winner() {
    let foreground = foreground_snapshot(Some(11));
    let mut snapshot = foreground_scoring_snapshot(
        Some(foreground),
        vec![
            test_group(
                FocusGroupKind::Game,
                vec![10],
                vec![10, 11],
                Some(10),
                "game",
                0.30,
            ),
            test_group(
                FocusGroupKind::Browser,
                vec![20],
                vec![20, 21],
                Some(20),
                "browser",
                0.95,
            ),
        ],
    );

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

    assert_eq!(snapshot.groups.len(), 1);
    assert_eq!(snapshot.groups[0].display_name, "game");
    assert!(snapshot.groups[0].score_breakdown.foreground_score > 0.0);
}

#[test]
fn hybrid_source_falls_back_when_provider_unavailable() {
    let mut unavailable = foreground_snapshot(Some(11));
    unavailable.status = crate::foreground::ForegroundProviderStatus::Unavailable;
    unavailable.decision.confidence = 0.0;
    let mut snapshot = foreground_scoring_snapshot(
        Some(unavailable),
        vec![
            test_group(
                FocusGroupKind::Game,
                vec![10],
                vec![10, 11],
                Some(10),
                "game",
                0.30,
            ),
            test_group(
                FocusGroupKind::Browser,
                vec![20],
                vec![20, 21],
                Some(20),
                "browser",
                0.95,
            ),
        ],
    );

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Hybrid);

    assert_eq!(snapshot.groups.len(), 2);
    assert_eq!(snapshot.groups[0].display_name, "game");
    assert_eq!(snapshot.groups[0].score, 0.30);
    assert_eq!(snapshot.groups[1].display_name, "browser");
    assert_eq!(snapshot.groups[1].score, 0.95);
}

#[test]
fn foreground_pid_does_not_select_system_service_root() {
    let mut snapshot = foreground_scoring_snapshot(Some(foreground_snapshot(Some(30))), Vec::new());
    let process = snapshot.processes.get_mut(&30).unwrap();
    process.comm = "systemd".to_owned();
    process.cmdline = "/usr/lib/systemd/systemd --user".to_owned();
    process.classification.class = SystemTaskClass::Service;

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

    assert!(snapshot.groups.is_empty());
}

#[test]
fn foreground_browser_maps_to_browser_foreground_band() {
    let foreground = foreground_snapshot(Some(20));
    let mut browser = test_group(
        FocusGroupKind::Browser,
        vec![20],
        vec![20, 21],
        Some(20),
        "browser",
        0.20,
    );
    browser.priority_band = PriorityBand::ForegroundLatency;
    browser.confidence = 0.80;

    let mut snapshot = foreground_scoring_snapshot(Some(foreground), vec![browser]);
    snapshot
        .processes
        .get_mut(&20)
        .unwrap()
        .classification
        .priority_band = PriorityBand::ForegroundLatency;

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

    assert_eq!(snapshot.groups.len(), 1);
    assert_eq!(snapshot.groups[0].kind, FocusGroupKind::Browser);
    assert_eq!(
        snapshot.groups[0].priority_band,
        PriorityBand::ForegroundLatency
    );
    assert_eq!(snapshot.groups[0].root_pids, vec![20]);
    assert!(snapshot.groups[0].score_breakdown.foreground_score > 0.0);
}

#[test]
fn foreground_source_preserves_existing_group_roots_when_group_contains_foreground_pid() {
    let foreground = foreground_snapshot(Some(11));
    let mut snapshot = foreground_scoring_snapshot(
        Some(foreground),
        vec![test_group(
            FocusGroupKind::Game,
            vec![10],
            vec![10, 11],
            Some(10),
            "game-with-helper-root",
            0.40,
        )],
    );

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

    assert_eq!(snapshot.groups.len(), 1);
    assert_eq!(snapshot.groups[0].display_name, "game-with-helper-root");
    assert_eq!(snapshot.groups[0].root_pids, vec![10]);
    assert_eq!(snapshot.groups[0].member_pids, vec![10, 11]);
    assert!(
        snapshot.groups[0]
            .reasons
            .iter()
            .all(|reason| reason
                != "foreground window PID selected but no known focus group matched")
    );
}

#[test]
fn foreground_source_adds_conservative_unknown_fallback_when_no_group_contains_foreground_pid() {
    let foreground = foreground_snapshot(Some(30));
    let mut snapshot = foreground_scoring_snapshot(
        Some(foreground),
        vec![test_group(
            FocusGroupKind::Browser,
            vec![20],
            vec![20, 21],
            Some(20),
            "browser",
            0.90,
        )],
    );
    // Break family match between browser (PPID 1) and compiler (PID 30)
    snapshot.processes.get_mut(&30).unwrap().ppid = 99;

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

    assert_eq!(snapshot.groups.len(), 1);
    assert_eq!(snapshot.groups[0].kind, FocusGroupKind::Unknown);
    assert_eq!(snapshot.groups[0].root_pids, vec![30]);
    assert_eq!(snapshot.groups[0].member_pids, Vec::<u32>::new());
    assert_eq!(snapshot.groups[0].primary_pid, Some(30));
    assert_eq!(snapshot.groups[0].display_name, "foreground:process-30");
    assert!((snapshot.groups[0].confidence - (0.95 * 0.75)).abs() < f32::EPSILON);
    assert!(
        snapshot.groups[0]
            .reasons
            .iter()
            .any(|reason| reason
                == "foreground window PID selected but no known focus group matched")
    );
    assert!(
        snapshot.groups[0]
            .reasons
            .iter()
            .any(|reason| reason == "conservative foreground fallback root pid=30")
    );
    assert!(snapshot.groups[0].score_breakdown.foreground_score > 0.0);
}

#[test]
fn foreground_source_fallback_member_pids_are_descendants_not_the_root_pid() {
    let foreground = foreground_snapshot(Some(10));
    let mut snapshot = foreground_scoring_snapshot(Some(foreground), Vec::new());

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

    assert_eq!(snapshot.groups.len(), 1);
    assert_eq!(snapshot.groups[0].root_pids, vec![10]);
    assert_eq!(snapshot.groups[0].member_pids, vec![11]);
    assert!(!snapshot.groups[0].member_pids.contains(&10));
}
