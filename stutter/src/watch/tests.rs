use super::{
    process_match::{ProcessMatchReason, find_process_match_by_pattern_at_with_cache},
    *,
};
use crate::{
    actions::ioprio::IoPrioValue,
    affinity::CpuMask,
    process_tree::TaskClass,
    profiles::{Profile, ProfileRule},
};

#[test]
fn force_for_watch_apply_only_uses_user_force_on_initial_apply() {
    assert!(force_for_watch_apply(true, true));
    assert!(!force_for_watch_apply(false, true));
    assert!(!force_for_watch_apply(true, false));
    assert!(!force_for_watch_apply(false, false));
}

#[test]
fn apply_profile_policy_allows_dry_run_without_medium_flag() {
    let profile = priority_profile();

    validate_apply_profile_policy(
        &profile,
        1234,
        false,
        true,
        false,
        false,
        crate::daemon_policy::ActionSource::Test,
    )
    .unwrap();
}

#[test]
fn watch_apply_profile_uses_policy_for_medium_risk_profiles() {
    let profile = priority_profile();

    let err = validate_apply_profile_policy(
        &profile,
        1234,
        false,
        false,
        false,
        false,
        crate::daemon_policy::ActionSource::Test,
    )
    .unwrap_err();

    assert!(err.to_string().contains("rejected by daemon policy"));
    assert!(
        err.to_string()
            .contains("safety class ReversibleMediumRisk")
    );

    validate_apply_profile_policy(
        &profile,
        1234,
        false,
        false,
        true,
        false,
        crate::daemon_policy::ActionSource::Test,
    )
    .unwrap();
}

#[test]
fn apply_profile_policy_allows_affinity_only_profile_without_medium_flag() {
    let profile = Profile {
        name: "affinity".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0").unwrap()),
            nice: None,
            ionice: None,
            match_class: vec![TaskClass::Game],
            match_comm: Vec::new(),
        }],
    };

    validate_apply_profile_policy(
        &profile,
        1234,
        false,
        false,
        false,
        false,
        crate::daemon_policy::ActionSource::Test,
    )
    .unwrap();
}

#[test]
fn apply_profile_policy_allows_persistent_effect_only_when_requested() {
    let profile = Profile {
        name: "affinity".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0").unwrap()),
            nice: None,
            ionice: None,
            match_class: vec![TaskClass::Game],
            match_comm: Vec::new(),
        }],
    };

    let action = crate::actions::cpu_affinity::CpuAffinityProfileAction {
        tree_pid: 1234,
        profile: profile.clone(),
        force_restore_overwrite: false,
    };
    let desc = action.descriptor_with_persistent_effect(true);

    let policy = profile_apply_policy(
        false,
        false,
        false,
        crate::daemon_policy::ActionSource::Test,
    );
    let err = policy
        .check_action(crate::daemon_policy::PolicyIntent::Apply, &desc)
        .unwrap_err();
    assert!(err.to_string().contains("persistent effect"));

    let policy = profile_apply_policy(false, false, true, crate::daemon_policy::ActionSource::Test);
    policy
        .check_action(crate::daemon_policy::PolicyIntent::Apply, &desc)
        .unwrap();
}

#[test]
fn apply_profile_mode_allows_one_shot_dry_run() {
    validate_apply_profile_mode(true, false).unwrap();
}

#[test]
fn apply_profile_mode_rejects_dry_run_watch() {
    let err = validate_apply_profile_mode(true, true).unwrap_err();

    assert!(
        err.to_string()
            .contains("apply-profile --dry-run cannot be combined with --watch")
    );
}

#[test]
fn apply_profile_mode_allows_real_watch() {
    validate_apply_profile_mode(false, true).unwrap();
}

#[test]
fn test_watch_process_should_poll() {
    assert!(WatchProcessState::None.should_poll());
    assert!(WatchProcessState::Waiting.should_poll());
    assert!(!WatchProcessState::Running(123).should_poll());
}

#[test]
fn test_process_match_score() {
    let p = "my-game";
    let pl = "my-game";

    assert_eq!(process_match_score(p, pl, "my-game", ""), Some(5));
    assert_eq!(process_match_score(p, pl, "MY-GAME", ""), Some(4));
    assert_eq!(
        process_match_score(p, pl, "other", "/usr/bin/my-game"),
        Some(3)
    );
    assert_eq!(
        process_match_score(p, pl, "other", "C:\\Games\\my-game"),
        Some(3)
    );
    assert_eq!(process_match_score(p, pl, "super-my-game-pro", ""), Some(2));
    assert_eq!(
        process_match_score(p, pl, "other", "--game=my-game"),
        Some(1)
    );
    assert_eq!(process_match_score(p, pl, "other", "--foo"), None);
}

#[test]
fn test_find_process_selection_priority() {
    let dir = std::env::temp_dir().join(format!("stutter-watch-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let pid100 = dir.join("100");
    std::fs::create_dir_all(&pid100).unwrap();
    std::fs::write(pid100.join("status"), "Name:\tmy-game-helper\nPPid:\t1\n").unwrap();
    std::fs::write(pid100.join("cmdline"), "helper\0").unwrap();
    std::fs::write(
        pid100.join("stat"),
        "100 (my-game-helper) S 1 100 0 0 -1 0 0 0 0 0 0 0 0 20 0 1 0 1000 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
    )
    .unwrap();

    let pid200 = dir.join("200");
    std::fs::create_dir_all(&pid200).unwrap();
    std::fs::write(pid200.join("status"), "Name:\tother\nPPid:\t1\n").unwrap();
    std::fs::write(pid200.join("cmdline"), "other\0--match=my-game\0").unwrap();
    std::fs::write(
        pid200.join("stat"),
        "200 (other) S 1 200 0 0 -1 0 0 0 0 0 0 0 0 20 0 1 0 1000 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
    )
    .unwrap();

    let mut cache = crate::process_tree::ProcessCache::default();
    let selected = find_process_by_pattern_at_with_cache(&dir, "my-game", &mut cache);
    assert_eq!(selected, Some(100));

    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn process_match_decision_explains_selected_process() {
    let dir =
        std::env::temp_dir().join(format!("stutter-watch-explain-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let pid100 = dir.join("100");
    std::fs::create_dir_all(&pid100).unwrap();
    std::fs::write(pid100.join("status"), "Name:\tother\nPPid:\t1\n").unwrap();
    std::fs::write(pid100.join("cmdline"), "/opt/Game.exe\0").unwrap();
    std::fs::write(
        pid100.join("stat"),
        "100 (other) S 1 100 0 0 -1 0 0 0 0 0 0 0 0 20 0 1 0 1000 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n",
    )
    .unwrap();

    let mut cache = crate::process_tree::ProcessCache::default();
    let decision = find_process_match_by_pattern_at_with_cache(&dir, "Game.exe", &mut cache)
        .expect("expected process match decision");

    assert_eq!(decision.pid.as_u32(), 100);
    assert_eq!(decision.score, 3);
    assert_eq!(
        decision.reasons,
        vec![ProcessMatchReason::ExecutableBasename]
    );
    assert_eq!(decision.reason_labels(), vec!["executable_basename"]);

    std::fs::remove_dir_all(dir).ok();
}

fn priority_profile() -> Profile {
    Profile {
        name: "priority".to_owned(),
        rules: vec![ProfileRule {
            affinity: None,
            nice: Some(10),
            ionice: Some(IoPrioValue::idle()),
            match_class: vec![TaskClass::Indexer],
            match_comm: Vec::new(),
        }],
    }
}
