//! Tests for community-rules command behavior and file loading.
//!
//! Owns command-facing community-rule regression tests. Does not own production rule models,
//! importers, rendering, or classification.

use std::fs;

use tempfile::tempdir;

use super::*;

struct EnvGuard {
    key: &'static str,
    old: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let old = std::env::var(key).ok();
        unsafe {
            std::env::set_var(key, value);
        }
        Self { key, old }
    }

    fn unset(key: &'static str) -> Self {
        let old = std::env::var(key).ok();
        unsafe {
            std::env::remove_var(key);
        }
        Self { key, old }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(old) = &self.old {
            unsafe {
                std::env::set_var(self.key, old);
            }
        } else {
            unsafe {
                std::env::remove_var(self.key);
            }
        }
    }
}

fn test_rules_json(name: &str) -> String {
    format!(
        r#"{{
"schema_version": 1,
"source": {{
"name": "test",
"repo": "https://example.test/repo.git",
"commit": "abc123",
"generated_at": "2026-05-09T00:00:00Z"
}},
"rules": [
{{
"name": "{name}",
"normalized_name": "{name}",
"type": "Game",
"stutter_class": "Game",
"confidence": 0.82,
"source_path": "00-default/Games/test.rules",
"context": ["wine_or_proton_or_steam"],
"title": null,
"ambiguous": false
}}
]
}}"#
    )
}

fn generated_rules_json_for_check() -> String {
    r#"{
"schema_version": 2,
"source": {
  "name": "test",
  "repo": null,
  "commit": null,
  "generated_at": "2026-05-09T00:00:00Z"
},
"rules": [
  {
    "name": "build.exe",
    "normalized_name": "build.exe",
    "type": "Game",
    "stutter_class": "Game",
    "confidence": 0.70,
    "source_path": "00-default/Games/wine_proton/test.rules",
    "context": ["wine_or_proton_or_steam"],
    "title": "Build Game",
    "source_url": null,
    "comment": null,
    "ambiguous": true
  },
  {
    "name": "rustc",
    "normalized_name": "rustc",
    "type": "Compiler",
    "stutter_class": "Compiler",
    "confidence": 0.80,
    "source_path": "00-default/Development/compiler.rules",
    "context": [],
    "title": null,
    "source_url": null,
    "comment": null,
    "ambiguous": false
  },
  {
    "name": "rustc-copy",
    "normalized_name": "rustc",
    "type": "Compiler",
    "stutter_class": "Compiler",
    "confidence": 0.80,
    "source_path": "00-default/Development/compiler-copy.rules",
    "context": [],
    "title": null,
    "source_url": null,
    "comment": null,
    "ambiguous": false
  },
  {
    "name": "mystery",
    "normalized_name": "mystery",
    "type": "Unknown",
    "stutter_class": "Unknown",
    "confidence": 0.50,
    "source_path": "00-default/Other/unknown.rules",
    "context": [],
    "title": null,
    "source_url": null,
    "comment": null,
    "ambiguous": false
  }
]
}"#
    .to_owned()
}

#[test]
fn rules_check_generated_reports_duplicates_ambiguous_and_unknown() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("ananicy.generated.json");
    fs::write(&path, generated_rules_json_for_check()).unwrap();

    let report = rules_check_generated_command(&path).unwrap();

    assert_eq!(report.source_files, 1);
    assert_eq!(report.json_objects, 4);
    assert_eq!(report.imported_rules, 4);
    assert_eq!(report.duplicates, 1);
    assert_eq!(report.ambiguous_names, 1);
    assert_eq!(report.context_required_game_rules, 1);
    assert_eq!(report.exact_only_non_game_rules, 2);
    assert_eq!(report.rules_mapped_to_unknown, 1);
    assert_eq!(report.unknown_mapped_classes, 0);
    assert_eq!(
        report.largest_duplicate_groups,
        vec![("rustc".to_owned(), 2)]
    );
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.contains("build.exe requires Steam/Proton context"))
    );
}

#[test]
fn rules_check_source_reports_import_skips_and_classes() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("source");
    fs::create_dir_all(source.join("00-default/Mixed")).unwrap();
    fs::write(
        source.join("00-default/Mixed/mixed.rules"),
        r#"
{"name":"build.exe","type":"Game"}
{"name":"rustc","type":"Compiler"}
{"type":"Game"}
{"name":"mystery","type":"MysteryCategory"}
"#,
    )
    .unwrap();

    let report = rules_check_source_command(&source).unwrap();

    assert_eq!(report.source_files, 1);
    assert_eq!(report.json_objects, 4);
    assert_eq!(report.imported_rules, 2);
    assert_eq!(report.skipped_no_name, 1);
    assert_eq!(report.skipped_unknown_class, 1);
    assert_eq!(report.skipped_objects(), 2);
    assert_eq!(report.ambiguous_names, 1);
    assert_eq!(report.context_required_game_rules, 1);
    assert_eq!(report.exact_only_non_game_rules, 1);
    assert_eq!(report.classes.get("Game"), Some(&1));
    assert_eq!(report.classes.get("Compiler"), Some(&1));
}

#[test]
fn import_report_text_matches_snapshot() {
    let mut report = ImportReport {
        scanned_files: 2,
        parsed_objects: 3,
        imported_rules: 1,
        skipped_no_name: 1,
        skipped_unknown_class: 1,
        ambiguous_rules: 1,
        context_required_game_rules: 1,
        ..ImportReport::default()
    };
    report.classes.insert("Game".to_owned(), 1);

    assert_eq!(
        render_import_report(&report),
        include_str!("../../../tests/snapshots/community_rules_import_report.txt")
    );
}

#[test]
fn rules_import_dry_run_does_not_write() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("source");
    let out = dir.path().join("out").join("ananicy.generated.json");
    let metadata = dir.path().join("out").join("ananicy.metadata.json");
    fs::create_dir_all(source.join("00-default/Games")).unwrap();
    fs::write(
        source.join("00-default/Games/example.rules"),
        r#"{"name":"example-game.exe","type":"Game","nice":-20}"#,
    )
    .unwrap();

    rules_import_command(crate::cli::RulesImportCommandInput {
        source,
        name: "ananicy".to_owned(),
        license: "GPL-3.0-only".to_owned(),
        source_repo: Some("https://example.test/ananicy-rules.git".to_owned()),
        source_commit: Some("abc123".to_owned()),
        out: Some(out.clone()),
        dry_run: true,
    })
    .unwrap();

    assert!(!out.exists());
    assert!(!metadata.exists());
}

#[test]
fn rules_import_writes_metadata_next_to_output_file() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("source");
    let out = dir.path().join("out").join("ananicy.generated.json");
    let metadata = dir.path().join("out").join("ananicy.metadata.json");
    fs::create_dir_all(source.join("00-default/Games")).unwrap();
    fs::write(
        source.join("00-default/Games/example.rules"),
        r#"{"name":"example-game.exe","type":"Game","nice":-20}"#,
    )
    .unwrap();

    rules_import_command(crate::cli::RulesImportCommandInput {
        source,
        name: "ananicy".to_owned(),
        license: "GPL-3.0-only".to_owned(),
        source_repo: Some("https://github.com/CachyOS/ananicy-rules".to_owned()),
        source_commit: Some("abc123".to_owned()),
        out: Some(out.clone()),
        dry_run: false,
    })
    .unwrap();

    assert!(out.exists());
    assert!(metadata.exists());

    let metadata: CommunityRulesMetadataFile =
        serde_json::from_str(&fs::read_to_string(metadata).unwrap()).unwrap();
    assert_eq!(metadata.schema_version, 1);
    assert_eq!(metadata.name, "ananicy");
    assert_eq!(metadata.license, "GPL-3.0-only");
    assert_eq!(
        metadata.source_repo.as_deref(),
        Some("https://github.com/CachyOS/ananicy-rules")
    );
    assert_eq!(metadata.source_commit.as_deref(), Some("abc123"));
    assert_eq!(metadata.generated_by, "stutter rules import");
    assert_eq!(metadata.rule_file, "ananicy.generated.json");
}

#[test]
fn default_community_rules_dir_uses_xdg_data_home() {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    let dir = tempdir().unwrap();
    let _xdg = EnvGuard::set("XDG_DATA_HOME", dir.path().to_str().unwrap());
    let _home = EnvGuard::set("HOME", "/tmp/ignored-home");

    assert_eq!(
        default_community_rules_dir().unwrap(),
        dir.path().join("stutter").join("community-rules")
    );
}

#[test]
fn default_community_rules_dir_falls_back_to_home() {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    let dir = tempdir().unwrap();
    let _xdg = EnvGuard::unset("XDG_DATA_HOME");
    let _home = EnvGuard::set("HOME", dir.path().to_str().unwrap());

    assert_eq!(
        default_community_rules_dir().unwrap(),
        dir.path()
            .join(".local")
            .join("share")
            .join("stutter")
            .join("community-rules")
    );
}

#[test]
fn load_user_rules_ignores_missing_directory() {
    let dir = tempdir().unwrap();
    let config = CommunityRulesConfig {
        enabled: true,
        load_builtin_fixture: false,
        user_rules_dir: Some(dir.path().join("missing")),
        explicit_rules_files: Vec::new(),
    };

    let db = load_community_rules(&config).unwrap();
    assert_eq!(db.rule_count(), 0);
}

#[test]
fn load_user_rules_rejects_bad_schema() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("bad.generated.json"),
        r#"{
"schema_version": 999,
"source": {
"name": "bad",
"repo": "https://example.test/repo.git",
"commit": "bad",
"generated_at": "2026-05-09T00:00:00Z"
},
"rules": []
}"#,
    )
    .unwrap();

    let config = CommunityRulesConfig {
        enabled: true,
        load_builtin_fixture: false,
        user_rules_dir: Some(dir.path().to_path_buf()),
        explicit_rules_files: Vec::new(),
    };

    let err = load_community_rules(&config).unwrap_err().to_string();
    assert!(err.contains("unsupported community rules schema version 999"));
}

#[test]
fn load_user_rules_combines_multiple_json_files() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("one.generated.json"),
        test_rules_json("one-game.exe"),
    )
    .unwrap();
    fs::write(
        dir.path().join("two.generated.json"),
        test_rules_json("two-game.exe"),
    )
    .unwrap();
    fs::write(
            dir.path().join("one.metadata.json"),
            r#"{"schema_version":1,"name":"one","license":"GPL-3.0-only","source_repo":null,"source_commit":null,"generated_at":"2026-05-09T00:00:00Z","generated_by":"stutter rules import","rule_file":"one.generated.json"}"#,
        )
        .unwrap();

    let config = CommunityRulesConfig {
        enabled: true,
        load_builtin_fixture: false,
        user_rules_dir: Some(dir.path().to_path_buf()),
        explicit_rules_files: Vec::new(),
    };

    let db = load_community_rules(&config).unwrap();
    assert_eq!(db.rule_count(), 2);
}
