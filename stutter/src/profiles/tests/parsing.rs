use std::{fs, path::Path};

use super::super::*;

#[test]
fn parses_minimal_profile() {
    let profiles = parse_profiles(
        r#"
        [[profile]]
        name = "kcd # not a comment"

        [[profile.rules]]
        affinity = "0-3"
        match_class = ["Game"]
        match_comm = ["RenderThread", "Main"]
        "#,
    )
    .unwrap();

    assert_eq!(profiles.len(), 1);
    assert_eq!(profiles[0].name, "kcd # not a comment");
    let rule = &profiles[0].rules[0];
    assert_eq!(rule.affinity.as_ref().unwrap().to_range_string(), "0-3");
    assert_eq!(rule.match_class, vec![TaskClass::Game]);
    let comm_patterns = rule
        .match_comm
        .iter()
        .map(CompiledPattern::raw)
        .collect::<Vec<_>>();
    assert_eq!(comm_patterns, vec!["RenderThread", "Main"]);
}

#[test]
fn render_profiles_toml_outputs_profile_rules() {
    let profile = Profile {
        name: "generated \"profile\"".to_owned(),
        rules: vec![ProfileRule {
            affinity: Some(CpuMask::parse("0-1").unwrap()),
            nice: Some(5),
            ionice: Some(IoPrioValue::idle()),
            match_class: vec![TaskClass::Game, TaskClass::GameRenderThread],
            match_comm: vec![
                CompiledPattern::new("RenderThread".to_owned()).unwrap(),
                CompiledPattern::new("Main".to_owned()).unwrap(),
            ],
        }],
    };

    let toml = render_profiles_toml(&[profile]);

    assert!(toml.contains("[[profile]]"));
    assert!(toml.contains("name = \"generated \\\"profile\\\"\""));
    assert!(toml.contains("[[profile.rules]]"));
    assert!(toml.contains("affinity = \"0-1\""));
    assert!(toml.contains("nice = 5"));
    assert!(toml.contains("ionice = \"idle\""));
    assert!(toml.contains("match_class = [\"Game\", \"GameRenderThread\"]"));
    assert!(toml.contains("match_comm = [\"RenderThread\", \"Main\"]"));
}

#[test]
fn profile_parser_accepts_online_affinity() {
    let profiles = parse_profiles(
        r#"
        [[profile]]
        name = "baseline-online"

        [[profile.rules]]
        affinity = "online"
        match_class = ["Game"]
        "#,
    )
    .unwrap();

    assert_eq!(profiles.len(), 1);
    assert!(!profiles[0].rules[0].affinity.as_ref().unwrap().is_empty());
}

#[test]
fn profile_parser_accepts_nice_only_rule() {
    let profiles = parse_profiles(
        r#"
        [[profile]]
        name = "background"

        [[profile.rules]]
        match_class = ["Indexer"]
        nice = 10
        "#,
    )
    .unwrap();

    let rule = &profiles[0].rules[0];
    assert!(rule.affinity.is_none());
    assert_eq!(rule.nice, Some(10));
    assert_eq!(rule.ionice, None);
}

#[test]
fn profile_parser_accepts_ionice_only_rule() {
    let profiles = parse_profiles(
        r#"
        [[profile]]
        name = "background"

        [[profile.rules]]
        match_class = ["PackageManager"]
        ionice = "idle"
        "#,
    )
    .unwrap();

    let rule = &profiles[0].rules[0];
    assert!(rule.affinity.is_none());
    assert_eq!(rule.nice, None);
    assert_eq!(rule.ionice, Some(IoPrioValue::idle()));
}

#[test]
fn profile_parser_accepts_combined_affinity_nice_ionice_rule() {
    let profiles = parse_profiles(
        r#"
        [[profile]]
        name = "game-latency"

        [[profile.rules]]
        match_class = ["Game", "GameRenderThread"]
        affinity = "0-3"
        nice = -5
        ionice = "be:2"
        "#,
    )
    .unwrap();

    let rule = &profiles[0].rules[0];
    assert_eq!(rule.affinity.as_ref().unwrap().to_range_string(), "0-3");
    assert_eq!(rule.nice, Some(-5));
    assert_eq!(rule.ionice, Some(IoPrioValue::best_effort(2)));
}

#[test]
fn profile_parser_rejects_invalid_nice_range() {
    let err = parse_profiles(
        r#"
        [[profile]]
        name = "bad"

        [[profile.rules]]
        nice = 20
        "#,
    )
    .unwrap_err();

    assert!(format!("{err:#}").contains("outside Linux range"));
}

#[test]
fn profile_parser_rejects_invalid_ionice_strings() {
    for ionice in ["best-effort", "realtime", "be:8", "rt:9", "idle:4"] {
        let err = parse_profiles(&format!(
            r#"
            [[profile]]
            name = "bad"

            [[profile.rules]]
            ionice = "{ionice}"
            "#
        ))
        .unwrap_err();

        assert!(
            format!("{err:#}").contains("ionice") || format!("{err:#}").contains("I/O priority")
        );
    }
}

#[test]
fn profile_parser_rejects_rule_with_no_action_fields() {
    let err = parse_profiles(
        r#"
        [[profile]]
        name = "bad"

        [[profile.rules]]
        match_class = ["Game"]
        "#,
    )
    .unwrap_err();

    assert!(format!("{err:#}").contains("at least one action field"));
}

#[test]
fn invalid_symbolic_affinity_fails_clearly() {
    let err = parse_profiles(
        r#"
        [[profile]]
        name = "bad"

        [[profile.rules]]
        affinity = "all"
        match_class = ["Game"]
        "#,
    )
    .unwrap_err();

    assert!(err.to_string().contains("invalid CPU id"));
}

#[test]
fn examples_profile_file_parses() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let path = manifest_dir
        .parent()
        .unwrap()
        .join("examples/profiles/common-game-layouts.toml");
    let profiles = load_profiles(&path).unwrap();

    assert!(!profiles.is_empty());
    assert!(
        profiles
            .iter()
            .any(|profile| profile.name == "baseline-online")
    );
}

fn write_two_profile_file() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("profiles.toml");
    fs::write(
        &path,
        r#"
        [[profile]]
        name = "baseline-online"

        [[profile.rules]]
        affinity = "online"

        [[profile]]
        name = "tuned"

        [[profile.rules]]
        affinity = "1-3"
        match_comm = ["Main"]
        "#,
    )
    .unwrap();
    (dir, path)
}

#[test]
fn load_selected_profile_defaults_to_first_profile() {
    let (_dir, path) = write_two_profile_file();

    let profile = load_selected_profile(&path, None).unwrap();

    assert_eq!(profile.name, "baseline-online");
}

#[test]
fn load_selected_profile_selects_named_profile() {
    let (_dir, path) = write_two_profile_file();

    let profile = load_selected_profile(&path, Some("tuned")).unwrap();

    assert_eq!(profile.name, "tuned");
    assert_eq!(
        profile.rules[0]
            .affinity
            .as_ref()
            .unwrap()
            .to_range_string(),
        "1-3"
    );
}

#[test]
fn load_selected_profile_rejects_missing_name_and_lists_available_profiles() {
    let (_dir, path) = write_two_profile_file();

    let err = load_selected_profile(&path, Some("missing")).unwrap_err();
    let message = err.to_string();

    assert!(message.contains("profile 'missing' not found"));
    assert!(message.contains("baseline-online"));
    assert!(message.contains("tuned"));
}
