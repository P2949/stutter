use super::*;

#[test]
fn stale_foreground_snapshot_does_not_score_focus_groups() {
    let mut foreground = foreground_snapshot(Some(11));
    foreground.stale_ms = Some(500);

    let mut snapshot = foreground_scoring_snapshot(
        Some(foreground),
        vec![test_group(
            FocusGroupKind::Game,
            vec![10],
            vec![10, 11],
            Some(10),
            "game",
            0.30,
        )],
    );

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Hybrid);

    assert_eq!(snapshot.groups.len(), 1);
    assert_eq!(snapshot.groups[0].display_name, "game");
    assert_eq!(snapshot.groups[0].score_breakdown.foreground_score, 0.0);
}

#[test]
fn foreground_source_clears_groups_when_provider_unavailable() {
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
                0.90,
            ),
        ],
    );

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

    assert!(snapshot.groups.is_empty());
}

#[test]
fn hybrid_source_falls_back_to_heuristic_when_foreground_is_stale() {
    let mut stale = foreground_snapshot(Some(11));
    stale.stale_ms = Some(1_000);
    let mut snapshot = foreground_scoring_snapshot(
        Some(stale),
        vec![test_group(
            FocusGroupKind::Browser,
            vec![20],
            vec![20, 21],
            Some(20),
            "browser",
            0.90,
        )],
    );

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Hybrid);

    assert_eq!(snapshot.groups.len(), 1);
    assert_eq!(snapshot.groups[0].display_name, "browser");
    assert_eq!(snapshot.groups[0].score, 0.90);
    assert_eq!(snapshot.groups[0].score_breakdown.foreground_score, 0.0);
}
