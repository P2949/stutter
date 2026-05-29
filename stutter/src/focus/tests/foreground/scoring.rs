use super::*;

#[test]
fn foreground_score_is_full_confidence_for_member_pid() {
    let foreground = foreground_snapshot(Some(11));
    let snapshot = foreground_scoring_snapshot(
        Some(foreground),
        vec![test_group(
            FocusGroupKind::Game,
            vec![10],
            vec![10, 11],
            Some(10),
            "game",
            0.40,
        )],
    );

    let score = foreground_score_for_group(&snapshot.groups[0], &snapshot);

    assert!((score - 0.95).abs() < f32::EPSILON);
}

#[test]
fn foreground_score_is_partial_for_same_process_family() {
    let foreground = foreground_snapshot(Some(11));
    let snapshot = foreground_scoring_snapshot(
        Some(foreground),
        vec![test_group(
            FocusGroupKind::Game,
            vec![10],
            vec![10],
            Some(10),
            "game",
            0.40,
        )],
    );

    let score = foreground_score_for_group(&snapshot.groups[0], &snapshot);

    assert!((score - (0.75 * 0.95)).abs() < f32::EPSILON);
}

#[test]
fn foreground_source_keeps_only_foreground_scoring_groups() {
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
                0.90,
            ),
        ],
    );

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

    assert_eq!(snapshot.groups.len(), 1);
    assert_eq!(snapshot.groups[0].display_name, "game");
    assert!(snapshot.groups[0].score_breakdown.foreground_score > 0.0);
    assert!(snapshot.groups[0].score > 0.30);
}

#[test]
fn hybrid_source_boosts_foreground_group_but_keeps_fallback_groups() {
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
                0.90,
            ),
        ],
    );

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Hybrid);

    assert_eq!(snapshot.groups.len(), 2);
    assert!(
        snapshot
            .groups
            .iter()
            .any(|group| group.display_name == "game")
    );
    assert!(
        snapshot
            .groups
            .iter()
            .any(|group| group.display_name == "browser")
    );
    let game = snapshot
        .groups
        .iter()
        .find(|group| group.display_name == "game")
        .unwrap();
    assert!(game.score_breakdown.foreground_score > 0.0);
    assert!(
        game.reasons
            .iter()
            .any(|reason| reason.contains("foreground-window score"))
    );
}

#[test]
fn heuristic_source_preserves_scores_exactly() {
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
                0.90,
            ),
        ],
    );
    let before = snapshot
        .groups
        .iter()
        .map(|group| {
            (
                group.display_name.clone(),
                group.score,
                group.score_breakdown.foreground_score,
            )
        })
        .collect::<Vec<_>>();

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Heuristic);

    let after = snapshot
        .groups
        .iter()
        .map(|group| {
            (
                group.display_name.clone(),
                group.score,
                group.score_breakdown.foreground_score,
            )
        })
        .collect::<Vec<_>>();

    assert_eq!(before, after);
}

#[test]
fn foreground_breakdown_deserializes_old_json_with_default_foreground_score() {
    let json = r#"{
            "cpu_score": 0.1,
            "io_score": 0.2,
            "interactivity_score": 0.3,
            "class_priority_score": 0.4,
            "stability_score": 0.5,
            "penalty": 0.6
        }"#;

    let breakdown: FocusScoreBreakdown = serde_json::from_str(json).unwrap();

    assert_eq!(breakdown.foreground_score, 0.0);
}
