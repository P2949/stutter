use tempfile::tempdir;

use super::*;
use crate::community_rules::{CommunityProcessIdentity, CommunityRulesDb};

fn import_input(source_dir: &Path) -> ImportInput {
    ImportInput {
        source_dir: source_dir.to_path_buf(),
        source_name: "test ananicy import".to_owned(),
        source_repo: Some("https://example.test/ananicy-rules.git".to_owned()),
        source_commit: Some("abc123".to_owned()),
        generated_at: "2026-05-09T00:00:00Z".to_owned(),
    }
}

fn write_rule_file(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn importer_imports_multi_object_rules_files() {
    let dir = tempdir().unwrap();
    write_rule_file(
        &dir.path()
            .join("00-default/Games/linux-native/linux_native_m.rules"),
        r#"
{"name":"first-game","type":"Game"}
{"name":"second-game","type":"Game"}
{"name":"third-game","type":"Game"}
"#,
    );

    let imported = import_ananicy_rules(import_input(dir.path())).unwrap();

    assert_eq!(imported.report.scanned_files, 1);
    assert_eq!(imported.report.parsed_objects, 3);
    assert_eq!(imported.report.imported_rules, 3);
    assert_eq!(imported.file.rules.len(), 3);
    assert_eq!(imported.file.rules[0].normalized_name, "first-game");
    assert_eq!(imported.file.rules[1].normalized_name, "second-game");
    assert_eq!(imported.file.rules[2].normalized_name, "third-game");
}

#[test]
fn importer_ignores_scheduling_policy() {
    let dir = tempdir().unwrap();
    write_rule_file(
        &dir.path().join("00-default/Games/policy.rules"),
        r#"
{"name":"policy-game.exe","type":"Game","nice":-20,"ionice":"realtime","sched":"fifo","sched_policy":"SCHED_FIFO","cpu_affinity":"0-3","systemd":"high-priority.slice"}
"#,
    );

    let imported = import_ananicy_rules(import_input(dir.path())).unwrap();
    assert_eq!(imported.file.rules.len(), 1);

    let serialized = serde_json::to_string(&imported.file).unwrap();
    assert!(!serialized.contains("nice"));
    assert!(!serialized.contains("ionice"));
    assert!(!serialized.contains("SCHED_FIFO"));
    assert!(!serialized.contains("cpu_affinity"));
    assert!(!serialized.contains("systemd"));
}

#[test]
fn importer_extracts_process_names() {
    let dir = tempdir().unwrap();
    write_rule_file(
        &dir.path()
            .join("00-default/Games/wine_proton/wine_proton_k.rules"),
        r#"
{"name":"C:\\Games\\KINGDOMCOME.EXE","type":"Game","title":"Kingdom Come"}
"#,
    );
    write_rule_file(
        &dir.path().join("00-default/Games/ignored.txt"),
        r#"
{"name":"ignored.exe","type":"Game"}
"#,
    );

    let imported = import_ananicy_rules(import_input(dir.path())).unwrap();

    assert_eq!(imported.file.rules.len(), 1);
    assert_eq!(imported.file.rules[0].name, r#"C:\Games\KINGDOMCOME.EXE"#);
    assert_eq!(imported.file.rules[0].normalized_name, "kingdomcome.exe");
    assert_eq!(imported.file.rules[0].stutter_class, "Game");
    assert_eq!(
        imported.file.rules[0].source_path,
        "00-default/Games/wine_proton/wine_proton_k.rules"
    );
}

#[test]
fn importer_deduplicates_by_normalized_name_and_source_path_only() {
    let dir = tempdir().unwrap();
    write_rule_file(
        &dir.path().join("00-default/Games/a.rules"),
        r#"
{"name":"SameGame.exe","type":"Game"}
{"name":"samegame.exe","type":"Game"}
"#,
    );
    write_rule_file(
        &dir.path().join("00-default/Games/subdir/a.rules"),
        r#"
{"name":"samegame.exe","type":"Game"}
"#,
    );

    let imported = import_ananicy_rules(import_input(dir.path())).unwrap();

    assert_eq!(imported.report.scanned_files, 2);
    assert_eq!(imported.report.parsed_objects, 3);
    assert_eq!(imported.report.duplicate_rules, 1);
    assert_eq!(imported.report.imported_rules, 2);
    assert_eq!(imported.file.rules.len(), 2);
    assert_eq!(imported.file.rules[0].normalized_name, "samegame.exe");
    assert_eq!(
        imported.file.rules[0].source_path,
        "00-default/Games/a.rules"
    );
    assert_eq!(imported.file.rules[1].normalized_name, "samegame.exe");
    assert_eq!(
        imported.file.rules[1].source_path,
        "00-default/Games/subdir/a.rules"
    );
}

#[test]
fn importer_marks_ambiguous_exe_names() {
    let dir = tempdir().unwrap();
    write_rule_file(
        &dir.path()
            .join("00-default/Games/wine_proton/wine_proton_b.rules"),
        r#"
{"name":"build.exe","type":"Game"}
{"name":"specificgame.exe","type":"Game"}
"#,
    );

    let imported = import_ananicy_rules(import_input(dir.path())).unwrap();

    let build = imported
        .file
        .rules
        .iter()
        .find(|rule| rule.normalized_name == "build.exe")
        .unwrap();
    let specific = imported
        .file
        .rules
        .iter()
        .find(|rule| rule.normalized_name == "specificgame.exe")
        .unwrap();

    assert!(build.ambiguous);
    assert!(!specific.ambiguous);
    assert!(build.confidence <= 0.70);
    assert_eq!(imported.report.ambiguous_rules, 1);
    assert_eq!(imported.report.context_required_game_rules, 2);
}

#[test]
fn importer_marks_ambiguous_names_context_required() {
    let dir = tempdir().unwrap();
    write_rule_file(
        &dir.path()
            .join("00-default/Games/wine_proton/wine_proton_l.rules"),
        r#"
{"name":"launcher.exe","type":"Game"}
{"name":"game.exe","type":"Game"}
{"name":"build.exe","type":"Game"}
"#,
    );

    let imported = import_ananicy_rules(import_input(dir.path())).unwrap();

    assert_eq!(imported.report.imported_rules, 3);
    assert_eq!(imported.report.ambiguous_rules, 3);
    assert_eq!(imported.report.context_required_game_rules, 3);
    assert!(
        imported
            .file
            .rules
            .iter()
            .all(|rule| rule.ambiguous
                && rule.context.contains(&"wine_or_proton_or_steam".to_owned()))
    );
}

#[test]
fn importer_preserves_source_metadata() {
    let dir = tempdir().unwrap();
    write_rule_file(
        &dir.path()
            .join("00-default/Games/linux-native/linux_native_f.rules"),
        r#"
{"name":"factorio","type":"Game","title":"Factorio"}
"#,
    );

    let imported = import_ananicy_rules(import_input(dir.path())).unwrap();

    assert_eq!(imported.file.schema_version, 2);
    assert_eq!(imported.file.source.name, "test ananicy import");
    assert_eq!(
        imported.file.source.repo,
        Some("https://example.test/ananicy-rules.git".to_owned())
    );
    assert_eq!(imported.file.source.commit, Some("abc123".to_owned()));
    assert_eq!(imported.file.source.generated_at, "2026-05-09T00:00:00Z");
}

#[test]
fn importer_keeps_native_linux_games_without_wine_context_hint() {
    let dir = tempdir().unwrap();
    write_rule_file(
        &dir.path()
            .join("00-default/Games/linux-native/linux_native_f.rules"),
        r#"
{"name":"factorio","type":"Game","title":"Factorio"}
"#,
    );

    let imported = import_ananicy_rules(import_input(dir.path())).unwrap();

    assert_eq!(imported.report.imported_rules, 1);
    assert_eq!(imported.report.context_required_game_rules, 0);
    assert_eq!(imported.file.rules[0].normalized_name, "factorio");
    assert_eq!(imported.file.rules[0].stutter_class, "Game");
    assert!(!imported.file.rules[0].ambiguous);
    assert_eq!(
        imported.file.rules[0].context,
        vec!["linux_native".to_owned()]
    );
}

#[test]
fn importer_maps_clear_non_game_categories_and_paths() {
    let dir = tempdir().unwrap();
    write_rule_file(
        &dir.path().join("00-default/Development/tools.rules"),
        r#"
{"name":"rustc","type":"Compiler"}
{"name":"ld.lld","type":"Linker"}
"#,
    );
    write_rule_file(
        &dir.path().join("00-default/Desktop/browser-gpu.rules"),
        r#"
{"name":"browser-gpu","type":"BrowserGpu"}
"#,
    );
    write_rule_file(
        &dir.path().join("00-default/System/package-manager.rules"),
        r#"
{"name":"emerge","type":"PackageManager"}
"#,
    );

    let imported = import_ananicy_rules(import_input(dir.path())).unwrap();

    assert_eq!(imported.report.imported_rules, 4);
    assert_eq!(imported.report.exact_only_non_game_rules, 4);
    assert_eq!(imported.report.classes.get("Compiler"), Some(&1));
    assert_eq!(imported.report.classes.get("Linker"), Some(&1));
    assert_eq!(imported.report.classes.get("BrowserGpu"), Some(&1));
    assert_eq!(imported.report.classes.get("PackageManager"), Some(&1));
    assert!(
        imported
            .file
            .rules
            .iter()
            .all(|rule| rule.stutter_class != "Unknown")
    );
}

#[test]
fn importer_output_roundtrips_through_community_rules_db() {
    let dir = tempdir().unwrap();
    write_rule_file(
        &dir.path()
            .join("00-default/Games/wine_proton/wine_proton_k.rules"),
        r#"
{"name":"KingdomCome.exe","type":"Game","title":"Kingdom Come"}
"#,
    );

    let imported = import_ananicy_rules(import_input(dir.path())).unwrap();
    let serialized = serde_json::to_string(&imported.file).unwrap();
    let db = CommunityRulesDb::from_json(&serialized).unwrap();

    let hit = db
        .classify(
            &CommunityProcessIdentity {
                thread_comm: "KingdomCome.exe",
                process_comm: "KingdomCome.exe",
                cmdline: "/usr/bin/wine KingdomCome.exe",
                exe_path: "/home/me/.steam/steamapps/common/KingdomCome/KingdomCome.exe",
                cgroup_path: "/user.slice/app-steam-379430.scope",
            },
            true,
        )
        .unwrap();

    assert_eq!(hit.class, TaskClass::Game);
    assert!(hit.source_path.contains("wine_proton_k.rules"));
}

#[test]
fn importer_extracts_title_source_url_and_comment_from_preceding_comment() {
    let dir = tempdir().unwrap();
    write_rule_file(
        &dir.path()
            .join("00-default/Games/wine_proton/wine_proton_k.rules"),
        r#"
# Kingdom Come: Deliverance https://store.steampowered.com/app/379430/Kingdom_Come_Deliverance/
{"name":"KingdomCome.exe","type":"Game"}
"#,
    );

    let imported = import_ananicy_rules(import_input(dir.path())).unwrap();

    assert_eq!(imported.file.schema_version, 2);
    assert_eq!(imported.file.rules.len(), 1);
    assert_eq!(
        imported.file.rules[0].title.as_deref(),
        Some("Kingdom Come: Deliverance")
    );
    assert_eq!(
        imported.file.rules[0].source_url.as_deref(),
        Some("https://store.steampowered.com/app/379430/Kingdom_Come_Deliverance/")
    );
    assert_eq!(
        imported.file.rules[0].comment.as_deref(),
        Some(
            "Kingdom Come: Deliverance https://store.steampowered.com/app/379430/Kingdom_Come_Deliverance/"
        )
    );
}

#[test]
fn importer_json_title_overrides_comment_title_but_preserves_comment() {
    let dir = tempdir().unwrap();
    write_rule_file(
        &dir.path()
            .join("00-default/Games/wine_proton/wine_proton_j.rules"),
        r#"
# Comment Title https://example.test/comment
{"name":"json-title-game.exe","type":"Game","title":"JSON Title","source_url":"https://example.test/json"}
"#,
    );

    let imported = import_ananicy_rules(import_input(dir.path())).unwrap();

    assert_eq!(imported.file.rules.len(), 1);
    assert_eq!(imported.file.rules[0].title.as_deref(), Some("JSON Title"));
    assert_eq!(
        imported.file.rules[0].source_url.as_deref(),
        Some("https://example.test/json")
    );
    assert_eq!(
        imported.file.rules[0].comment.as_deref(),
        Some("Comment Title https://example.test/comment")
    );
}

#[test]
fn importer_blank_line_breaks_comment_association() {
    let dir = tempdir().unwrap();
    write_rule_file(
        &dir.path()
            .join("00-default/Games/wine_proton/wine_proton_b.rules"),
        r#"
# Detached Title https://example.test/detached

{"name":"blank-break-game.exe","type":"Game"}
"#,
    );

    let imported = import_ananicy_rules(import_input(dir.path())).unwrap();

    assert_eq!(imported.file.rules.len(), 1);
    assert_eq!(imported.file.rules[0].title, None);
    assert_eq!(imported.file.rules[0].source_url, None);
    assert_eq!(imported.file.rules[0].comment, None);
}

#[test]
fn importer_report_counts_denylisted_ambiguous_context_and_exact_only_rules() {
    let dir = tempdir().unwrap();
    write_rule_file(
        &dir.path().join("00-default/Mixed/mixed.rules"),
        r#"
{"name":"python","type":"Compiler"}
{"name":"build.exe","type":"Game"}
{"name":"rustc","type":"Compiler"}
{"name":"node","type":"BrowserRenderer"}
"#,
    );

    let imported = import_ananicy_rules(import_input(dir.path())).unwrap();

    assert_eq!(imported.report.scanned_files, 1);
    assert_eq!(imported.report.parsed_objects, 4);
    assert_eq!(imported.report.imported_rules, 4);
    assert_eq!(imported.report.ambiguous_rules, 3);
    assert_eq!(imported.report.context_required_game_rules, 1);
    assert_eq!(imported.report.exact_only_non_game_rules, 1);
    assert_eq!(imported.report.classes.get("Game"), Some(&1));
    assert_eq!(imported.report.classes.get("Compiler"), Some(&2));
    assert_eq!(imported.report.classes.get("BrowserRenderer"), Some(&1));

    let python = imported
        .file
        .rules
        .iter()
        .find(|rule| rule.normalized_name == "python")
        .unwrap();
    let node = imported
        .file
        .rules
        .iter()
        .find(|rule| rule.normalized_name == "node")
        .unwrap();
    let rustc = imported
        .file
        .rules
        .iter()
        .find(|rule| rule.normalized_name == "rustc")
        .unwrap();

    assert!(python.ambiguous);
    assert!(node.ambiguous);
    assert!(!rustc.ambiguous);
}

#[test]
fn importer_combined_risky_fixture_reports_expected_counts() {
    let dir = tempdir().unwrap();
    write_rule_file(
        &dir.path().join("00-default/Mixed/risky.rules"),
        r#"
# Kingdom Come: Deliverance https://store.steampowered.com/app/379430/Kingdom_Come_Deliverance/
{"name":"KingdomCome.exe","type":"Game","nice":-20,"ionice":"realtime","sched":"fifo","cpu_affinity":"0-3","systemd":"high-priority.slice"}
{"name":"KingdomCome.exe","type":"Game"}
{"name":"build.exe","type":"Game"}
{"name":"factorio","type":"Game"}
{"name":"rustc","type":"Compiler"}
{"name":"mystery","type":"MysteryCategory"}
{"type":"Game"}
"#,
    );

    let imported = import_ananicy_rules(import_input(dir.path())).unwrap();

    assert_eq!(imported.report.scanned_files, 1);
    assert_eq!(imported.report.parsed_objects, 7);
    assert_eq!(imported.report.imported_rules, 4);
    assert_eq!(imported.report.duplicate_rules, 1);
    assert_eq!(imported.report.skipped_no_name, 1);
    assert_eq!(imported.report.skipped_unknown_class, 1);
    assert_eq!(imported.report.ambiguous_rules, 1);
    assert_eq!(imported.report.context_required_game_rules, 2);
    assert_eq!(imported.report.exact_only_non_game_rules, 1);
    assert_eq!(imported.report.classes.get("Game"), Some(&3));
    assert_eq!(imported.report.classes.get("Compiler"), Some(&1));

    let serialized = serde_json::to_string(&imported.file).unwrap();
    assert!(!serialized.contains("nice"));
    assert!(!serialized.contains("ionice"));
    assert!(!serialized.contains("SCHED_FIFO"));
    assert!(!serialized.contains("cpu_affinity"));
    assert!(!serialized.contains("systemd"));

    let kingdom_come = imported
        .file
        .rules
        .iter()
        .find(|rule| rule.normalized_name == "kingdomcome.exe")
        .unwrap();
    assert_eq!(
        kingdom_come.title.as_deref(),
        Some("Kingdom Come: Deliverance")
    );
    assert_eq!(
        kingdom_come.source_url.as_deref(),
        Some("https://store.steampowered.com/app/379430/Kingdom_Come_Deliverance/")
    );
}

#[test]
fn importer_skips_unknown_class_rules_instead_of_importing_unknown() {
    let dir = tempdir().unwrap();
    write_rule_file(
        &dir.path().join("00-default/Other/unknown.rules"),
        r#"
{"name":"mystery-process","type":"MysteryCategory"}
"#,
    );

    let imported = import_ananicy_rules(import_input(dir.path())).unwrap();

    assert_eq!(imported.report.parsed_objects, 1);
    assert_eq!(imported.report.skipped_unknown_class, 1);
    assert_eq!(imported.report.imported_rules, 0);
    assert!(imported.file.rules.is_empty());
}
