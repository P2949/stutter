//! Foreground-aware focus tests extracted from `focus::mod`.
//!
//! Owns foreground-source, foreground-scoring, and foreground resolver sample coverage.
//! Does not own shared fixtures or production focus behavior.

#[cfg(test)]
mod tests {
    use crate::focus::{
        test_support::{
            foreground_scoring_snapshot, foreground_snapshot, foreground_test_group as test_group,
            foreground_test_process as test_process,
        },
        *,
    };

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
        let mut snapshot =
            foreground_scoring_snapshot(Some(foreground_snapshot(Some(30))), Vec::new());
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
            snapshot.groups[0].reasons.iter().all(|reason| reason
                != "foreground window PID selected but no known focus group matched")
        );
    }

    #[test]
    fn foreground_source_adds_conservative_unknown_fallback_when_no_group_contains_foreground_pid()
    {
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
            snapshot.groups[0].reasons.iter().any(|reason| reason
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

    #[test]
    fn foreground_source_does_not_auto_target_pid_one() {
        let mut snapshot = foreground_scoring_snapshot(
            Some(foreground_snapshot(Some(1))),
            vec![test_group(
                FocusGroupKind::Browser,
                vec![20],
                vec![20, 21],
                Some(20),
                "browser",
                0.90,
            )],
        );
        snapshot
            .processes
            .insert(1, test_process(1, 0, SystemTaskClass::Service));

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

        assert!(snapshot.groups.is_empty());
    }

    #[test]
    fn foreground_source_does_not_auto_target_systemd() {
        let mut snapshot =
            foreground_scoring_snapshot(Some(foreground_snapshot(Some(30))), Vec::new());
        let process = snapshot.processes.get_mut(&30).unwrap();
        process.comm = "systemd".to_owned();
        process.cmdline = "/usr/lib/systemd/systemd --user".to_owned();

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

        assert!(snapshot.groups.is_empty());
    }

    #[test]
    fn foreground_source_does_not_auto_target_compositor() {
        let mut snapshot =
            foreground_scoring_snapshot(Some(foreground_snapshot(Some(30))), Vec::new());
        let process = snapshot.processes.get_mut(&30).unwrap();
        process.classification.class = SystemTaskClass::Compositor;
        process.classification.priority_band = PriorityBand::ForegroundLatency;

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

        assert!(snapshot.groups.is_empty());
    }

    #[test]
    fn foreground_source_does_not_auto_target_realtime_audio_or_input() {
        let mut audio_snapshot =
            foreground_scoring_snapshot(Some(foreground_snapshot(Some(30))), Vec::new());
        let audio_process = audio_snapshot.processes.get_mut(&30).unwrap();
        audio_process.classification.class = SystemTaskClass::AudioRealtime;
        audio_process.classification.priority_band = PriorityBand::CriticalRealtime;

        apply_foreground_source_mode_to_snapshot(&mut audio_snapshot, FocusSource::Foreground);

        assert!(audio_snapshot.groups.is_empty());

        let mut input_snapshot =
            foreground_scoring_snapshot(Some(foreground_snapshot(Some(30))), Vec::new());
        let input_process = input_snapshot.processes.get_mut(&30).unwrap();
        input_process.classification.class = SystemTaskClass::Input;
        input_process.classification.priority_band = PriorityBand::CriticalRealtime;

        apply_foreground_source_mode_to_snapshot(&mut input_snapshot, FocusSource::Foreground);

        assert!(input_snapshot.groups.is_empty());
    }

    #[test]
    fn foreground_source_does_not_auto_target_xwayland() {
        let mut snapshot =
            foreground_scoring_snapshot(Some(foreground_snapshot(Some(30))), Vec::new());
        let process = snapshot.processes.get_mut(&30).unwrap();
        process.comm = "Xwayland".to_owned();
        process.cmdline = "/usr/bin/Xwayland :0 -rootless".to_owned();

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Foreground);

        assert!(snapshot.groups.is_empty());
    }

    #[test]
    fn hybrid_source_unsafe_foreground_falls_back_to_existing_heuristic_groups() {
        let mut snapshot = foreground_scoring_snapshot(
            Some(foreground_snapshot(Some(30))),
            vec![test_group(
                FocusGroupKind::Browser,
                vec![20],
                vec![20, 21],
                Some(20),
                "browser",
                0.90,
            )],
        );
        let process = snapshot.processes.get_mut(&30).unwrap();
        process.comm = "Xwayland".to_owned();
        process.cmdline = "/usr/bin/Xwayland :0 -rootless".to_owned();

        apply_foreground_source_mode_to_snapshot(&mut snapshot, FocusSource::Hybrid);

        assert_eq!(snapshot.groups.len(), 1);
        assert_eq!(snapshot.groups[0].display_name, "browser");
        assert_eq!(snapshot.groups[0].root_pids, vec![20]);
        assert_eq!(snapshot.groups[0].score, 0.90);
        assert_eq!(snapshot.groups[0].score_breakdown.foreground_score, 0.0);
    }

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
}
