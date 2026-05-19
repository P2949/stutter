//! Focus resolver tests extracted from `focus::mod`.
//!
//! Owns tests for this focus behavior area after extraction from `focus::mod`.
//! Does not own shared fixtures or production focus behavior.

#[cfg(test)]
mod tests {
    use crate::focus::{
        test_support::{
            required_focus_policy, required_test_group, required_test_process,
            required_test_snapshot, resolver_test_group, resolver_test_policy,
            resolver_test_process, resolver_test_snapshot,
        },
        *,
    };

    #[test]
    fn focus_resolver_requires_repeated_winner_before_initial_switch() {
        let mut resolver = FocusResolver::new(resolver_test_policy());
        let group = resolver_test_group(
            FocusGroupKind::Game,
            vec![10],
            vec![10],
            Some(10),
            0.80,
            0.80,
        );
        let process = resolver_test_process(10, "Game.exe", SystemTaskClass::Game);

        let first = resolver.decide_from_snapshot(resolver_test_snapshot(
            vec![group.clone()],
            vec![process.clone()],
            1000,
        ));

        match first {
            FocusDecision::NoTarget { reason } => {
                assert!(reason.contains("waiting for stable winner"));
            }
            other => panic!("expected NoTarget on first pending poll, got {other:?}"),
        }

        let second =
            resolver.decide_from_snapshot(resolver_test_snapshot(vec![group], vec![process], 2000));

        match second {
            FocusDecision::Switch { old, new } => {
                assert!(old.is_none());
                assert_eq!(new.group.kind, FocusGroupKind::Game);
                assert_eq!(new.group.root_pids, vec![10]);
                assert_eq!(new.selected_at_ms, 2000);
                assert_eq!(new.last_confirmed_ms, 2000);
                assert_eq!(
                    new.situation,
                    crate::autotune::state::SituationKind::GameFocused
                );
            }
            other => panic!("expected initial Switch after repeated winner, got {other:?}"),
        }
    }

    #[test]
    fn focus_resolver_keeps_current_when_alive_and_score_floor_met() {
        let mut policy = resolver_test_policy();
        policy.required_winner_polls = 1;
        let mut resolver = FocusResolver::new(policy);

        let first_group = resolver_test_group(
            FocusGroupKind::Browser,
            vec![20],
            vec![20],
            Some(20),
            0.90,
            0.90,
        );
        let first_process =
            resolver_test_process(20, "firefox", SystemTaskClass::BrowserForeground);

        let first = resolver.decide_from_snapshot(resolver_test_snapshot(
            vec![first_group],
            vec![first_process],
            0,
        ));

        match first {
            FocusDecision::Switch { new, .. } => {
                assert_eq!(new.group.kind, FocusGroupKind::Browser);
            }
            other => panic!("expected initial Switch, got {other:?}"),
        }

        let keep_group = resolver_test_group(
            FocusGroupKind::Browser,
            vec![20],
            vec![20],
            Some(20),
            0.45,
            0.70,
        );
        let keep_process = resolver_test_process(20, "firefox", SystemTaskClass::BrowserForeground);

        let keep = resolver.decide_from_snapshot(resolver_test_snapshot(
            vec![keep_group],
            vec![keep_process],
            1000,
        ));

        match keep {
            FocusDecision::Keep { focus } => {
                assert_eq!(focus.group.kind, FocusGroupKind::Browser);
                assert_eq!(focus.group.score, 0.45);
                assert_eq!(focus.selected_at_ms, 0);
                assert_eq!(focus.last_confirmed_ms, 1000);
            }
            other => {
                panic!("expected Keep for live current focus above score floor, got {other:?}")
            }
        }
    }

    #[test]
    fn focus_resolver_enforces_switch_margin_and_cooldown() {
        let mut policy = resolver_test_policy();
        policy.required_winner_polls = 1;
        policy.switch_margin = 0.20;
        policy.switch_cooldown_ms = 5000;
        let mut resolver = FocusResolver::new(policy);

        let old_group = resolver_test_group(
            FocusGroupKind::Browser,
            vec![30],
            vec![30],
            Some(30),
            0.60,
            0.90,
        );
        let old_process = resolver_test_process(30, "firefox", SystemTaskClass::BrowserForeground);

        let initial = resolver.decide_from_snapshot(resolver_test_snapshot(
            vec![old_group.clone()],
            vec![old_process.clone()],
            0,
        ));

        match initial {
            FocusDecision::Switch { new, .. } => {
                assert_eq!(new.group.kind, FocusGroupKind::Browser);
            }
            other => panic!("expected initial Switch, got {other:?}"),
        }

        let weak_new_group = resolver_test_group(
            FocusGroupKind::Compile,
            vec![40],
            vec![40],
            Some(40),
            0.75,
            0.95,
        );
        let weak_new_process = resolver_test_process(40, "cargo", SystemTaskClass::BuildJob);

        let below_margin = resolver.decide_from_snapshot(resolver_test_snapshot(
            vec![old_group.clone(), weak_new_group],
            vec![old_process.clone(), weak_new_process],
            1000,
        ));

        match below_margin {
            FocusDecision::Keep { focus } => {
                assert_eq!(focus.group.kind, FocusGroupKind::Browser);
            }
            other => panic!("expected Keep when candidate fails switch margin, got {other:?}"),
        }

        let strong_new_group = resolver_test_group(
            FocusGroupKind::Compile,
            vec![41],
            vec![41],
            Some(41),
            0.90,
            0.95,
        );
        let strong_new_process = resolver_test_process(41, "cargo", SystemTaskClass::BuildJob);

        let during_cooldown = resolver.decide_from_snapshot(resolver_test_snapshot(
            vec![old_group.clone(), strong_new_group.clone()],
            vec![old_process.clone(), strong_new_process.clone()],
            2000,
        ));

        match during_cooldown {
            FocusDecision::Keep { focus } => {
                assert_eq!(focus.group.kind, FocusGroupKind::Browser);
            }
            other => panic!("expected Keep during switch cooldown, got {other:?}"),
        }

        let after_cooldown = resolver.decide_from_snapshot(resolver_test_snapshot(
            vec![old_group, strong_new_group],
            vec![old_process, strong_new_process],
            6000,
        ));

        match after_cooldown {
            FocusDecision::Switch { old, new } => {
                assert_eq!(old.unwrap().group.kind, FocusGroupKind::Browser);
                assert_eq!(new.group.kind, FocusGroupKind::Compile);
                assert_eq!(
                    new.situation,
                    crate::autotune::state::SituationKind::CompileLoad
                );
            }
            other => panic!("expected Switch after cooldown and margin, got {other:?}"),
        }
    }

    #[test]
    fn focus_resolver_clears_when_no_group_meets_confidence() {
        let mut policy = resolver_test_policy();
        policy.required_winner_polls = 1;
        let mut resolver = FocusResolver::new(policy);

        let high_conf_group = resolver_test_group(
            FocusGroupKind::Game,
            vec![50],
            vec![50],
            Some(50),
            0.90,
            0.90,
        );
        let process = resolver_test_process(50, "Game.exe", SystemTaskClass::Game);

        let initial = resolver.decide_from_snapshot(resolver_test_snapshot(
            vec![high_conf_group],
            vec![process.clone()],
            0,
        ));

        match initial {
            FocusDecision::Switch { new, .. } => {
                assert_eq!(new.group.kind, FocusGroupKind::Game);
            }
            other => panic!("expected initial Switch, got {other:?}"),
        }

        let low_conf_group = resolver_test_group(
            FocusGroupKind::Game,
            vec![50],
            vec![50],
            Some(50),
            0.90,
            0.40,
        );

        let clear = resolver.decide_from_snapshot(resolver_test_snapshot(
            vec![low_conf_group],
            vec![process],
            1000,
        ));

        match clear {
            FocusDecision::Clear { old, reason } => {
                assert!(old.is_some());
                assert!(reason.contains("min_confidence"));
            }
            other => panic!("expected Clear when no group meets confidence, got {other:?}"),
        }
    }

    #[test]
    fn focus_resolver_limits_selected_root_pids_to_policy_max_roots() {
        let mut policy = resolver_test_policy();
        policy.required_winner_polls = 1;
        policy.max_roots = 2;
        let mut resolver = FocusResolver::new(policy);

        let group = resolver_test_group(
            FocusGroupKind::Game,
            vec![60, 61, 62],
            vec![60, 61, 62],
            Some(60),
            0.90,
            0.90,
        );

        let processes = vec![
            resolver_test_process(60, "GameA.exe", SystemTaskClass::Game),
            resolver_test_process(61, "GameB.exe", SystemTaskClass::Game),
            resolver_test_process(62, "GameC.exe", SystemTaskClass::Game),
        ];

        let decision =
            resolver.decide_from_snapshot(resolver_test_snapshot(vec![group], processes, 0));

        match decision {
            FocusDecision::Switch { new, .. } => {
                assert_eq!(new.group.root_pids, vec![60, 61]);
                assert_eq!(new.group.member_pids, vec![60, 61, 62]);
            }
            other => panic!("expected Switch with truncated roots, got {other:?}"),
        }
    }

    #[test]
    fn hysteresis_keeps_current_focus() {
        let mut policy = required_focus_policy();
        policy.required_winner_polls = 1;
        policy.switch_margin = 0.20;
        policy.switch_cooldown_ms = 0;
        let mut resolver = FocusResolver::new(policy);

        let browser_group = required_test_group(
            FocusGroupKind::Browser,
            vec![600],
            vec![600],
            Some(600),
            0.70,
            0.90,
        );
        let browser_process = required_test_process(
            600,
            1,
            "firefox",
            SystemTaskClass::BrowserForeground,
            PriorityBand::ForegroundLatency,
            30,
        );

        let initial = resolver.decide_from_snapshot(required_test_snapshot(
            vec![browser_group.clone()],
            vec![browser_process.clone()],
            0,
        ));

        assert!(matches!(initial, FocusDecision::Switch { .. }));

        let compile_group = required_test_group(
            FocusGroupKind::Compile,
            vec![700],
            vec![700],
            Some(700),
            0.75,
            0.95,
        );
        let compile_process = required_test_process(
            700,
            1,
            "cargo",
            SystemTaskClass::BuildJob,
            PriorityBand::Throughput,
            200,
        );

        let decision = resolver.decide_from_snapshot(required_test_snapshot(
            vec![browser_group, compile_group],
            vec![browser_process, compile_process],
            1000,
        ));

        match decision {
            FocusDecision::Keep { focus } => {
                assert_eq!(focus.group.kind, FocusGroupKind::Browser);
                assert_eq!(focus.group.score, 0.70);
            }
            other => panic!("expected browser focus to be kept, got {other:?}"),
        }
    }

    #[test]
    fn sustained_winner_switches() {
        let mut policy = required_focus_policy();
        policy.required_winner_polls = 2;
        policy.switch_margin = 0.20;
        policy.switch_cooldown_ms = 0;
        let mut resolver = FocusResolver::new(policy);

        let browser_group = required_test_group(
            FocusGroupKind::Browser,
            vec![800],
            vec![800],
            Some(800),
            0.60,
            0.90,
        );
        let browser_process = required_test_process(
            800,
            1,
            "firefox",
            SystemTaskClass::BrowserForeground,
            PriorityBand::ForegroundLatency,
            30,
        );

        let first = resolver.decide_from_snapshot(required_test_snapshot(
            vec![browser_group.clone()],
            vec![browser_process.clone()],
            0,
        ));
        assert!(matches!(first, FocusDecision::NoTarget { .. }));

        let second = resolver.decide_from_snapshot(required_test_snapshot(
            vec![browser_group.clone()],
            vec![browser_process.clone()],
            1000,
        ));
        assert!(matches!(second, FocusDecision::Switch { old: None, .. }));

        let compile_group = required_test_group(
            FocusGroupKind::Compile,
            vec![900],
            vec![900],
            Some(900),
            0.95,
            0.95,
        );
        let compile_process = required_test_process(
            900,
            1,
            "cargo",
            SystemTaskClass::BuildJob,
            PriorityBand::Throughput,
            300,
        );

        let pending = resolver.decide_from_snapshot(required_test_snapshot(
            vec![browser_group, compile_group.clone()],
            vec![browser_process, compile_process.clone()],
            2000,
        ));

        match pending {
            FocusDecision::Keep { focus } => {
                assert_eq!(focus.group.kind, FocusGroupKind::Browser);
            }
            other => panic!("expected keep while compile winner is pending, got {other:?}"),
        }

        let switched = resolver.decide_from_snapshot(required_test_snapshot(
            vec![compile_group],
            vec![compile_process],
            3000,
        ));

        match switched {
            FocusDecision::Switch { old, new } => {
                assert_eq!(old.unwrap().group.kind, FocusGroupKind::Browser);
                assert_eq!(new.group.kind, FocusGroupKind::Compile);
                assert_eq!(new.group.score, 0.95);
            }
            other => panic!("expected sustained compile winner to switch, got {other:?}"),
        }
    }

    #[test]
    fn idle_has_no_target() {
        let mut policy = required_focus_policy();
        policy.min_confidence = 0.80;
        let mut resolver = FocusResolver::new(policy);

        let idle_group = required_test_group(
            FocusGroupKind::Idle,
            vec![1000],
            vec![1000],
            Some(1000),
            0.10,
            0.30,
        );
        let idle_process = required_test_process(
            1000,
            1,
            "systemd",
            SystemTaskClass::Service,
            PriorityBand::Background,
            0,
        );

        let decision = resolver.decide_from_snapshot(required_test_snapshot(
            vec![idle_group],
            vec![idle_process],
            1000,
        ));

        match decision {
            FocusDecision::NoTarget { reason } => {
                assert!(reason.contains("min_confidence"));
            }
            other => panic!("expected NoTarget for idle below threshold, got {other:?}"),
        }
    }

    #[test]
    fn max_roots_is_enforced() {
        let mut policy = required_focus_policy();
        policy.required_winner_polls = 1;
        policy.max_roots = 2;
        let mut resolver = FocusResolver::new(policy);

        let group = required_test_group(
            FocusGroupKind::Compile,
            vec![1100, 1101, 1102, 1103, 1104],
            vec![1100, 1101, 1102, 1103, 1104],
            Some(1100),
            0.95,
            0.95,
        );
        let processes = vec![
            required_test_process(
                1100,
                1,
                "cargo-a",
                SystemTaskClass::BuildJob,
                PriorityBand::Throughput,
                100,
            ),
            required_test_process(
                1101,
                1,
                "cargo-b",
                SystemTaskClass::BuildJob,
                PriorityBand::Throughput,
                100,
            ),
            required_test_process(
                1102,
                1,
                "cargo-c",
                SystemTaskClass::BuildJob,
                PriorityBand::Throughput,
                100,
            ),
            required_test_process(
                1103,
                1,
                "cargo-d",
                SystemTaskClass::BuildJob,
                PriorityBand::Throughput,
                100,
            ),
            required_test_process(
                1104,
                1,
                "cargo-e",
                SystemTaskClass::BuildJob,
                PriorityBand::Throughput,
                100,
            ),
        ];

        let decision =
            resolver.decide_from_snapshot(required_test_snapshot(vec![group], processes, 1000));

        match decision {
            FocusDecision::Switch { new, .. } => {
                assert_eq!(new.group.root_pids, vec![1100, 1101]);
                assert_eq!(new.group.member_pids, vec![1100, 1101, 1102, 1103, 1104]);
            }
            other => panic!("expected Switch with capped roots, got {other:?}"),
        }
    }

    #[test]
    fn dead_root_clears_or_switches() {
        let mut policy = required_focus_policy();
        policy.required_winner_polls = 1;
        policy.switch_margin = 0.20;
        policy.switch_cooldown_ms = 0;
        let mut resolver = FocusResolver::new(policy);

        let browser_group = required_test_group(
            FocusGroupKind::Browser,
            vec![1200],
            vec![1200],
            Some(1200),
            0.80,
            0.90,
        );
        let browser_process = required_test_process(
            1200,
            1,
            "firefox",
            SystemTaskClass::BrowserForeground,
            PriorityBand::ForegroundLatency,
            100,
        );

        let initial = resolver.decide_from_snapshot(required_test_snapshot(
            vec![browser_group],
            vec![browser_process],
            0,
        ));
        assert!(matches!(initial, FocusDecision::Switch { .. }));

        let compile_group = required_test_group(
            FocusGroupKind::Compile,
            vec![1300],
            vec![1300],
            Some(1300),
            0.95,
            0.95,
        );
        let compile_process = required_test_process(
            1300,
            1,
            "cargo",
            SystemTaskClass::BuildJob,
            PriorityBand::Throughput,
            200,
        );

        let switch_decision = resolver.decide_from_snapshot(required_test_snapshot(
            vec![compile_group],
            vec![compile_process],
            1000,
        ));

        match switch_decision {
            FocusDecision::Switch { old, new } => {
                assert_eq!(old.unwrap().group.kind, FocusGroupKind::Browser);
                assert_eq!(new.group.kind, FocusGroupKind::Compile);
            }
            FocusDecision::Clear { old, reason } => {
                assert_eq!(old.unwrap().group.kind, FocusGroupKind::Browser);
                assert!(reason.contains("current focus root disappeared"));
            }
            other => panic!("expected dead root to switch or clear cleanly, got {other:?}"),
        }
    }
}
