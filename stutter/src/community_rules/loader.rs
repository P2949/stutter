use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;

use super::{CommunityRulesDb, CommunityRulesFile};

#[cfg(test)]
const TEST_FIXTURE_RULES_JSON: &str =
    include_str!("../../assets/community-rules/ananicy.fixture.generated.json");

#[derive(Debug, Clone)]
pub struct LoadCommunityRulesInput {
    pub enabled: bool,
    pub load_test_fixture: bool,
    pub user_rules_dir: Option<PathBuf>,
    pub explicit_rules_files: Vec<PathBuf>,
    pub system_rules_dirs: Vec<PathBuf>,
}

impl Default for LoadCommunityRulesInput {
    fn default() -> Self {
        Self {
            enabled: true,
            load_test_fixture: cfg!(test),
            user_rules_dir: super::paths::default_user_rules_dir(),
            explicit_rules_files: Vec::new(),
            system_rules_dirs: super::paths::default_system_rules_dirs(),
        }
    }
}

pub fn load_rules_file(path: &Path) -> anyhow::Result<CommunityRulesFile> {
    #[cfg(test)]
    if path == Path::new("__stutter_test_fixture__") {
        return parse_rules_json(
            TEST_FIXTURE_RULES_JSON,
            "embedded community rules test fixture",
        );
    }

    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read community rules file {}", path.display()))?;
    parse_rules_json(&data, &path.display().to_string())
}

pub fn load_rules_dir(path: &Path) -> anyhow::Result<Vec<CommunityRulesFile>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    anyhow::ensure!(
        path.is_dir(),
        "community rules path is not a directory: {}",
        path.display()
    );

    let mut files = Vec::new();
    let mut paths = Vec::new();

    for entry in fs::read_dir(path).with_context(|| {
        format!(
            "failed to read community rules directory {}",
            path.display()
        )
    })? {
        let entry =
            entry.with_context(|| format!("failed to read entry under {}", path.display()))?;
        let path = entry.path();

        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        if file_name == "enabled.generated.json" {
            continue;
        }

        if file_name.ends_with(".generated.json") {
            paths.push(path);
        }
    }

    paths.sort();

    for path in paths {
        files.push(load_rules_file(&path)?);
    }

    Ok(files)
}

pub fn load_rules_db(input: LoadCommunityRulesInput) -> anyhow::Result<CommunityRulesDb> {
    if !input.enabled {
        return Ok(CommunityRulesDb::empty());
    }

    let mut files = Vec::new();

    for path in input.explicit_rules_files {
        files.push(load_rules_file(&path)?);
    }

    if let Some(user_rules_dir) = input.user_rules_dir {
        files.extend(load_rules_dir(&user_rules_dir)?);
    }

    for system_rules_dir in input.system_rules_dirs {
        files.extend(load_rules_dir(&system_rules_dir)?);
    }

    if input.load_test_fixture {
        #[cfg(test)]
        {
            files.push(load_rules_file(Path::new("__stutter_test_fixture__"))?);
        }

        #[cfg(not(test))]
        {
            log::warn!(
                "ignoring load_test_fixture=true because the community rules fixture is test-only"
            );
        }
    }

    CommunityRulesDb::from_files(files)
}

fn parse_rules_json(data: &str, source_label: &str) -> anyhow::Result<CommunityRulesFile> {
    let file: CommunityRulesFile = serde_json::from_str(data)
        .with_context(|| format!("failed to parse community rules JSON from {source_label}"))?;
    anyhow::ensure!(
        matches!(file.schema_version, 1 | 2),
        "unsupported community rules schema version {}",
        file.schema_version
    );
    Ok(file)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

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

    #[test]
    fn generated_schema_v2_roundtrips_through_loader() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("ananicy.generated.json");
        fs::write(
            &path,
            r#"{
  "schema_version": 2,
  "source": {
    "name": "test",
    "repo": "https://example.test/repo.git",
    "commit": "abc123",
    "generated_at": "2026-05-09T00:00:00Z"
  },
  "rules": [
    {
      "name": "KingdomCome.exe",
      "normalized_name": "kingdomcome.exe",
      "type": "Game",
      "stutter_class": "Game",
      "confidence": 0.82,
      "source_path": "00-default/Games/wine_proton/wine_proton_k.rules",
      "context": ["wine_or_proton_or_steam"],
      "title": "Kingdom Come: Deliverance",
      "source_url": "https://store.steampowered.com/app/379430/Kingdom_Come_Deliverance/",
      "comment": "Kingdom Come: Deliverance https://store.steampowered.com/app/379430/Kingdom_Come_Deliverance/",
      "ambiguous": false
    }
  ]
}"#,
        )
        .unwrap();

        let file = load_rules_file(&path).unwrap();

        assert_eq!(file.schema_version, 2);
        assert_eq!(file.rules.len(), 1);
        assert_eq!(file.rules[0].normalized_name, "kingdomcome.exe");
        assert_eq!(
            file.rules[0].title.as_deref(),
            Some("Kingdom Come: Deliverance")
        );
        assert_eq!(
            file.rules[0].source_url.as_deref(),
            Some("https://store.steampowered.com/app/379430/Kingdom_Come_Deliverance/")
        );
        assert_eq!(
            file.rules[0].comment.as_deref(),
            Some(
                "Kingdom Come: Deliverance https://store.steampowered.com/app/379430/Kingdom_Come_Deliverance/"
            )
        );

        let db = CommunityRulesDb::from_file(file).unwrap();
        assert_eq!(db.rule_count(), 1);
    }

    #[test]
    fn load_rules_file_rejects_bad_schema() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("bad.generated.json");
        fs::write(
            &path,
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

        let err = load_rules_file(&path).unwrap_err().to_string();
        assert!(err.contains("unsupported community rules schema version 999"));
    }

    #[test]
    fn load_rules_dir_ignores_missing_directory() {
        let dir = tempdir().unwrap();
        let files = load_rules_dir(&dir.path().join("missing")).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn load_rules_dir_loads_generated_json_files_in_sorted_order() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("b.generated.json"),
            test_rules_json("b-game.exe"),
        )
        .unwrap();
        fs::write(
            dir.path().join("a.generated.json"),
            test_rules_json("a-game.exe"),
        )
        .unwrap();
        fs::write(dir.path().join("a.metadata.json"), "{}").unwrap();
        fs::write(
            dir.path().join("enabled.generated.json"),
            test_rules_json("enabled.exe"),
        )
        .unwrap();

        let files = load_rules_dir(dir.path()).unwrap();

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].rules[0].name, "a-game.exe");
        assert_eq!(files[1].rules[0].name, "b-game.exe");
    }

    #[test]
    fn load_rules_db_loads_package_style_system_rules_and_ignores_metadata() {
        let dir = tempdir().unwrap();
        let system_dir = dir
            .path()
            .join("usr")
            .join("share")
            .join("stutter")
            .join("community-rules");
        fs::create_dir_all(&system_dir).unwrap();

        fs::write(
            system_dir.join("ananicy.generated.json"),
            test_rules_json("system-packaged-game.exe"),
        )
        .unwrap();
        fs::write(
            system_dir.join("ananicy.metadata.json"),
            r#"{"schema_version":1,"name":"ananicy","license":"GPL-3.0-only","source_repo":"https://github.com/CachyOS/ananicy-rules","source_commit":"abc123","generated_at":"2026-05-09T00:00:00Z","generated_by":"stutter rules import","rule_file":"ananicy.generated.json"}"#,
        )
        .unwrap();

        let db = load_rules_db(LoadCommunityRulesInput {
            enabled: true,
            load_test_fixture: false,
            user_rules_dir: None,
            explicit_rules_files: Vec::new(),
            system_rules_dirs: vec![system_dir],
        })
        .unwrap();

        assert_eq!(db.rule_count(), 1);
    }

    #[test]
    fn load_rules_db_prioritizes_explicit_then_user_then_system_then_fixture() {
        let dir = tempdir().unwrap();
        let explicit = dir.path().join("explicit.generated.json");
        let user_dir = dir.path().join("user");
        let system_dir = dir.path().join("system");
        fs::create_dir_all(&user_dir).unwrap();
        fs::create_dir_all(&system_dir).unwrap();

        fs::write(&explicit, test_rules_json("explicit-game.exe")).unwrap();
        fs::write(
            user_dir.join("user.generated.json"),
            test_rules_json("user-game.exe"),
        )
        .unwrap();
        fs::write(
            system_dir.join("system.generated.json"),
            test_rules_json("system-game.exe"),
        )
        .unwrap();

        let db = load_rules_db(LoadCommunityRulesInput {
            enabled: true,
            load_test_fixture: true,
            user_rules_dir: Some(user_dir),
            explicit_rules_files: vec![explicit],
            system_rules_dirs: vec![system_dir],
        })
        .unwrap();

        assert_eq!(db.rule_count(), 6);
    }

    #[test]
    fn load_rules_db_disabled_returns_empty_db() {
        let db = load_rules_db(LoadCommunityRulesInput {
            enabled: false,
            load_test_fixture: true,
            user_rules_dir: None,
            explicit_rules_files: Vec::new(),
            system_rules_dirs: Vec::new(),
        })
        .unwrap();

        assert_eq!(db.rule_count(), 0);
    }
}
