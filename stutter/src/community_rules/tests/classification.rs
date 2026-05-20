//! Tests for community-rule classification, guarded names, and context matching.
//!
//! Owns classification regression tests and test-only identities. Does not own production rule
//! loading, rendering, import, or persistence.

use super::*;
use crate::process_tree::TaskClass;

fn identity<'a>(
    thread_comm: &'a str,
    process_comm: &'a str,
    cmdline: &'a str,
    exe_path: &'a str,
    cgroup_path: &'a str,
) -> CommunityProcessIdentity<'a> {
    CommunityProcessIdentity {
        thread_comm,
        process_comm,
        cmdline,
        exe_path,
        cgroup_path,
    }
}

#[test]
fn normalize_basename_is_case_insensitive_and_strips_deleted_suffix() {
    assert_eq!(
        normalize_process_name("/games/KingdomCome.EXE (deleted)").as_deref(),
        Some("kingdomcome.exe")
    );
    assert_eq!(
        normalize_process_name(r#"C:\Games\Build.EXE"#).as_deref(),
        Some("build.exe")
    );
}

#[test]
fn exact_exe_basename_match_classifies_game_with_context() {
    let hit = classify_process_identity(&identity(
        "KingdomCome.exe",
        "KingdomCome.exe",
        "/usr/bin/wine KingdomCome.exe",
        "/home/me/.steam/steamapps/common/KingdomCome/KingdomCome.exe",
        "/user.slice/app-steam-379430.scope",
    ))
    .unwrap();

    assert_eq!(hit.class, TaskClass::Game);
    assert!(hit.reason.contains("wine_proton_k.rules"));
}

#[test]
fn case_insensitive_cmdline_basename_match_works_when_comm_is_truncated() {
    let hit = classify_process_identity(&identity(
        "KingdomCome",
        "KingdomCome",
        "/home/me/.steam/steamapps/compatdata/379430/pfx/drive_c/KINGDOMCOME.EXE --arg",
        "/usr/bin/wine",
        "/user.slice/app-steam-379430.scope",
    ))
    .unwrap();

    assert_eq!(hit.class, TaskClass::Game);
    assert!(hit.confidence <= 0.88);
    assert!(hit.reason.contains("cmdline basename"));
}

#[test]
fn build_exe_game_rule_does_not_classify_outside_game_context() {
    let db = rules_db_with_rules(vec![rule("build.exe", "Game")]);

    let hit = db.classify(
        &identity(
            "build.exe",
            "build.exe",
            "/tmp/build.exe --project",
            "/tmp/build.exe",
            "/user.slice/build-tool.scope",
        ),
        true,
    );

    assert!(hit.is_none());
}

#[test]
fn build_exe_game_rule_classifies_inside_compatdata_context() {
    let db = rules_db_with_rules(vec![rule("build.exe", "Game")]);

    let hit = db
        .classify(
            &identity(
                "build.exe",
                "build.exe",
                "/mnt/games/compatdata/123/pfx/drive_c/build.exe",
                "/usr/bin/wine",
                "/user.slice/app-steam-123.scope",
            ),
            true,
        )
        .unwrap();

    assert_eq!(hit.class, TaskClass::Game);
    assert!(hit.reason.contains("context=compatdata"));
    assert_eq!(hit.confidence, 0.70);
}

#[test]
fn build_exe_game_rule_classifies_inside_steamapps_context() {
    let db = rules_db_with_rules(vec![rule("build.exe", "Game")]);

    let hit = db
        .classify(
            &identity(
                "build.exe",
                "build.exe",
                "/home/me/.steam/steamapps/common/TestGame/build.exe",
                "/home/me/.steam/steamapps/common/TestGame/build.exe",
                "/user.slice/app-steam-456.scope",
            ),
            true,
        )
        .unwrap();

    assert_eq!(hit.class, TaskClass::Game);
    assert!(hit.reason.contains("context=steamapps"));
    assert_eq!(hit.confidence, 0.70);
}

#[test]
fn native_linux_game_rule_classifies_without_wine_context() {
    let db = rules_db_with_rules(vec![rule("factorio", "Game")]);

    let hit = db
        .classify(
            &identity(
                "factorio",
                "factorio",
                "/home/me/games/factorio/bin/x64/factorio",
                "/home/me/games/factorio/bin/x64/factorio",
                "/user.slice/factorio.scope",
            ),
            true,
        )
        .unwrap();

    assert_eq!(hit.class, TaskClass::Game);
    assert!(hit.reason.contains("context=exact-name"));
}

#[test]
fn clear_non_game_rule_classifies_from_exe_path() {
    let db = rules_db_with_rules(vec![rule("rustc", "Compiler")]);

    let hit = db
        .classify(
            &identity(
                "rustc",
                "rustc",
                "rustc --crate-name stutter",
                "/usr/bin/rustc",
                "/user.slice/build.scope",
            ),
            true,
        )
        .unwrap();

    assert_eq!(hit.class, TaskClass::Compiler);
    assert!(hit.reason.contains("context=exact-exe"));
    assert_eq!(hit.confidence, 0.80);
}

#[test]
fn ambiguous_rule_without_context_does_not_match() {
    let hit = classify_process_identity(&identity(
        "build.exe",
        "build.exe",
        "/tmp/build.exe",
        "/tmp/build.exe",
        "/user.slice/app-builder.scope",
    ));

    assert!(hit.is_none());
}

fn rules_db_with_rules(rules: Vec<CommunityRule>) -> CommunityRulesDb {
    CommunityRulesDb::from_file(CommunityRulesFile {
        schema_version: 1,
        source: CommunityRulesSource {
            name: "test community rules".to_owned(),
            repo: None,
            commit: None,
            generated_at: "2026-05-09T00:00:00Z".to_owned(),
        },
        rules,
    })
    .unwrap()
}

fn rule(name: &str, stutter_class: &str) -> CommunityRule {
    CommunityRule {
        name: name.to_owned(),
        normalized_name: normalize_process_name(name).unwrap(),
        r#type: stutter_class.to_owned(),
        stutter_class: stutter_class.to_owned(),
        confidence: 0.90,
        source_path: "test.rules".to_owned(),
        context: Vec::new(),
        title: None,
        source_url: None,
        comment: None,
        ambiguous: false,
    }
}

#[test]
fn non_game_rule_can_classify_from_exe_basename() {
    let db = rules_db_with_rules(vec![rule("rustc", "Compiler")]);

    let hit = db
        .classify(
            &identity(
                "rustc",
                "rustc",
                "rustc --crate-name stutter",
                "/usr/bin/rustc",
                "/user.slice/build.scope",
            ),
            true,
        )
        .unwrap();

    assert_eq!(hit.class, TaskClass::Compiler);
    assert!(hit.reason.contains("context=exact-exe"));
}

#[test]
fn thread_comm_match_is_capped_lower_than_process_comm_match() {
    let db = rules_db_with_rules(vec![rule("worker-thread", "Game")]);

    let thread_hit = db
        .classify(
            &identity(
                "worker-thread",
                "parent-process",
                "",
                "",
                "/user.slice/build.scope",
            ),
            false,
        )
        .unwrap();

    assert_eq!(thread_hit.class, TaskClass::Game);
    assert_eq!(thread_hit.confidence, 0.65);

    let process_hit = db
        .classify(
            &identity(
                "other-thread",
                "worker-thread",
                "",
                "",
                "/user.slice/build.scope",
            ),
            false,
        )
        .unwrap();

    assert_eq!(process_hit.class, TaskClass::Game);
    assert_eq!(process_hit.confidence, 0.75);
}

#[test]
fn non_game_exact_match_is_capped_at_point_eight() {
    let db = rules_db_with_rules(vec![rule("rustc", "Compiler")]);

    let hit = db
        .classify(
            &identity(
                "rustc",
                "rustc",
                "rustc --crate-name stutter",
                "/usr/bin/rustc",
                "/user.slice/build.scope",
            ),
            true,
        )
        .unwrap();

    assert_eq!(hit.class, TaskClass::Compiler);
    assert_eq!(hit.confidence, 0.80);
}

#[test]
fn generic_service_rule_is_capped_at_point_six() {
    let mut service_rule = rule("helperd", "Service");
    service_rule.source_path = "misc.rules".to_owned();
    let db = rules_db_with_rules(vec![service_rule]);

    let hit = db
        .classify(
            &identity(
                "helperd",
                "helperd",
                "helperd",
                "/usr/bin/helperd",
                "/system.slice/helperd.service",
            ),
            true,
        )
        .unwrap();

    assert_eq!(hit.class, TaskClass::Service);
    assert_eq!(hit.confidence, 0.60);
}

#[test]
fn specific_service_rule_can_use_non_game_exact_cap() {
    let mut service_rule = rule("NetworkManager", "NetworkDaemon");
    service_rule.source_path = "00-default/Services/network/networkmanager.rules".to_owned();
    let db = rules_db_with_rules(vec![service_rule]);

    let hit = db
        .classify(
            &identity(
                "NetworkManager",
                "NetworkManager",
                "NetworkManager --no-daemon",
                "/usr/bin/NetworkManager",
                "/system.slice/NetworkManager.service",
            ),
            true,
        )
        .unwrap();

    assert_eq!(hit.class, TaskClass::NetworkDaemon);
    assert_eq!(hit.confidence, 0.80);
}

#[test]
fn ambiguous_game_rule_still_caps_at_point_seven_with_context() {
    let mut game_rule = rule("build.exe", "Game");
    game_rule.ambiguous = true;
    let db = rules_db_with_rules(vec![game_rule]);

    let hit = db
        .classify(
            &identity(
                "build.exe",
                "build.exe",
                "/home/me/.steam/steamapps/compatdata/123/pfx/drive_c/build.exe",
                "/usr/bin/wine",
                "/user.slice/app-steam-123.scope",
            ),
            true,
        )
        .unwrap();

    assert_eq!(hit.class, TaskClass::Game);
    assert_eq!(hit.confidence, 0.70);
}

#[test]
fn non_game_rule_does_not_classify_from_comm_only() {
    let db = rules_db_with_rules(vec![rule("rustc", "Compiler")]);

    let hit = db.classify(
        &identity("rustc", "rustc", "", "", "/user.slice/build.scope"),
        true,
    );

    assert!(hit.is_none());
}

#[test]
fn unknown_class_rule_is_skipped() {
    let db = rules_db_with_rules(vec![rule("mystery", "Unknown")]);

    let hit = db.classify(
        &identity(
            "mystery",
            "mystery",
            "mystery",
            "/usr/bin/mystery",
            "/user.slice/mystery.scope",
        ),
        true,
    );

    assert!(hit.is_none());
}

#[test]
fn gaming_runtime_rule_requires_runtime_context_when_strict() {
    let db = rules_db_with_rules(vec![rule("pv-adverb", "SteamRuntime")]);

    let without_context = db.classify(
        &identity(
            "pv-adverb",
            "pv-adverb",
            "/usr/lib/pressure-vessel/pv-adverb",
            "/usr/lib/pressure-vessel/pv-adverb",
            "/user.slice/app-steam-123.scope",
        ),
        true,
    );

    assert_eq!(
        without_context.map(|hit| hit.class),
        Some(TaskClass::SteamRuntime)
    );

    let no_runtime_context = db.classify(
        &identity(
            "pv-adverb",
            "pv-adverb",
            "/tmp/pv-adverb",
            "/tmp/pv-adverb",
            "/user.slice/plain.scope",
        ),
        true,
    );

    assert!(no_runtime_context.is_none());
}

#[test]
fn game_rule_still_requires_context_when_rule_requires_context() {
    let mut game_rule = rule("SomeGame.exe", "Game");
    game_rule.context = vec!["wine_or_proton_or_steam".to_owned()];
    let db = rules_db_with_rules(vec![game_rule]);

    let hit = db.classify(
        &identity(
            "SomeGame.exe",
            "SomeGame.exe",
            "/tmp/SomeGame.exe",
            "/tmp/SomeGame.exe",
            "/user.slice/plain.scope",
        ),
        true,
    );

    assert!(hit.is_none());
}

#[test]
fn merge_file_forces_guarded_non_game_name_to_ambiguous() {
    let db = CommunityRulesDb::from_file(CommunityRulesFile {
        schema_version: 2,
        source: CommunityRulesSource {
            name: "test guarded rules".to_owned(),
            repo: None,
            commit: None,
            generated_at: "2026-05-09T00:00:00Z".to_owned(),
        },
        rules: vec![CommunityRule {
            name: "python".to_owned(),
            normalized_name: "python".to_owned(),
            r#type: "Compiler".to_owned(),
            stutter_class: "Compiler".to_owned(),
            confidence: 0.90,
            source_path: "test.rules".to_owned(),
            context: Vec::new(),
            title: None,
            source_url: None,
            comment: None,
            ambiguous: false,
        }],
    })
    .unwrap();

    let hit = db.classify(
        &identity(
            "python",
            "python",
            "python build.py",
            "/usr/bin/python",
            "/user.slice/build.scope",
        ),
        true,
    );

    assert!(hit.is_none());
}

#[test]
fn ambiguous_rule_with_compatdata_context_can_match() {
    let hit = classify_process_identity(&identity(
        "build.exe",
        "build.exe",
        "/home/me/.steam/steamapps/compatdata/123/pfx/drive_c/build.exe",
        "/usr/bin/wine",
        "/user.slice/app-steam-123.scope",
    ))
    .unwrap();

    assert_eq!(hit.class, TaskClass::Game);
    assert!(hit.confidence <= 0.70);
}
