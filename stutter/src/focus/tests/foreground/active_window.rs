use super::*;

#[test]
fn foreground_missing_process_pid_does_not_boost_or_create_focus_group_in_hybrid_mode() {
    let mut foreground = foreground_snapshot(Some(9999));
    if let Some(target) = foreground.decision.target.as_mut() {
        target.window_id = Some("163".to_owned());
        target.workspace = Some("5".to_owned());
    }

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

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Hybrid);

    assert_eq!(snapshot.groups.len(), 1);
    assert_eq!(snapshot.groups[0].display_name, "browser");
    assert_eq!(snapshot.groups[0].score_breakdown.foreground_score, 0.0);
}

#[test]
fn foreground_missing_process_pid_clears_unrelated_groups_in_foreground_only_mode() {
    let foreground = foreground_snapshot(Some(9999));

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

    apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

    assert!(snapshot.groups.is_empty());
}

#[test]
fn focus_snapshot_stores_foreground_snapshot() {
    let proc = crate::test_support::FakeProc::new("focus-stores-foreground");
    proc.write_process(
        crate::test_support::FakeProcess::new(4242, "steam_app_379430", 12_345)
            .cmdline(vec!["steam_app_379430".to_owned()])
            .cgroup("/user.slice/app.slice"),
    )
    .unwrap();

    let foreground = foreground_snapshot(Some(4242));
    let mut cache = FocusCache::default();

    let snapshot = focus_snapshot_at(proc.path(), &mut cache, 1_000, Some(&foreground));

    assert_eq!(
        snapshot.foreground.as_ref().and_then(|fg| fg
            .decision
            .target
            .as_ref()
            .and_then(|target| target.pid)),
        Some(4242)
    );
    assert_eq!(
        snapshot.foreground.as_ref().and_then(|fg| {
            fg.decision
                .target
                .as_ref()
                .and_then(|target| target.app_id.as_deref())
        }),
        Some("steam")
    );
}

#[test]
fn focus_snapshot_marks_matching_foreground_process() {
    let proc = crate::test_support::FakeProc::new("focus-marks-matching-foreground");
    proc.write_process(
        crate::test_support::FakeProcess::new(4242, "steam_app_379430", 12_345)
            .cmdline(vec!["steam_app_379430".to_owned()])
            .cgroup("/user.slice/app.slice"),
    )
    .unwrap();
    proc.write_process(
        crate::test_support::FakeProcess::new(9000, "firefox", 22_222)
            .cmdline(vec!["firefox".to_owned()])
            .cgroup("/user.slice/app.slice"),
    )
    .unwrap();

    let foreground = foreground_snapshot(Some(4242));
    let mut cache = FocusCache::default();

    let snapshot = focus_snapshot_at(proc.path(), &mut cache, 1_000, Some(&foreground));

    assert!(
        snapshot
            .processes
            .get(&4242)
            .unwrap()
            .is_foreground_window_process
    );
    assert!(
        !snapshot
            .processes
            .get(&9000)
            .unwrap()
            .is_foreground_window_process
    );
}

#[test]
fn focus_snapshot_does_not_mark_any_process_without_foreground_pid() {
    let proc = crate::test_support::FakeProc::new("focus-no-foreground-pid");
    proc.write_process(
        crate::test_support::FakeProcess::new(4242, "steam_app_379430", 12_345)
            .cmdline(vec!["steam_app_379430".to_owned()])
            .cgroup("/user.slice/app.slice"),
    )
    .unwrap();

    let foreground = foreground_snapshot(None);
    let mut cache = FocusCache::default();

    let snapshot = focus_snapshot_at(proc.path(), &mut cache, 1_000, Some(&foreground));

    assert!(
        !snapshot
            .processes
            .get(&4242)
            .unwrap()
            .is_foreground_window_process
    );
}

#[test]
fn focus_resolver_sample_accepts_foreground_and_source_mode() {
    let proc = crate::test_support::FakeProc::new("focus-resolver-foreground-source-mode");
    proc.write_process(
        crate::test_support::FakeProcess::new(4242, "steam_app_379430", 12_345)
            .cmdline(vec!["steam_app_379430".to_owned()])
            .cgroup("/user.slice/app.slice"),
    )
    .unwrap();

    let foreground = foreground_snapshot(Some(4242));
    let mut resolver = FocusResolver::new(FocusPolicy {
        min_confidence: 0.0,
        required_winner_polls: 1,
        ..FocusPolicy::default()
    });

    let decision = resolver.sample(
        proc.path(),
        1_000,
        Some(&foreground),
        FocusSource::Foreground,
    );

    match decision {
        FocusDecision::Keep { .. }
        | FocusDecision::Switch { .. }
        | FocusDecision::Clear { .. }
        | FocusDecision::NoTarget { .. } => {}
    }
}
