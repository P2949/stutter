use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde_json::{Map, Value};

use super::{CommunityRule, CommunityRulesFile, CommunityRulesSource, normalize_process_name};
use crate::process_tree::TaskClass;

#[derive(Debug, Clone)]
pub struct ImportInput {
    pub source_dir: PathBuf,
    pub source_name: String,
    pub source_repo: Option<String>,
    pub source_commit: Option<String>,
    pub generated_at: String,
}

pub fn import_ananicy_rules(input: ImportInput) -> anyhow::Result<CommunityRulesFile> {
    anyhow::ensure!(
        input.source_dir.is_dir(),
        "Ananicy rules source_dir is not a directory: {}",
        input.source_dir.display()
    );

    let mut rule_files = Vec::new();
    collect_rule_files(&input.source_dir, &mut rule_files)?;
    rule_files.sort();

    let mut rules = Vec::new();
    let mut seen = HashSet::new();

    for path in rule_files {
        let source_path = relative_source_path(&input.source_dir, &path);
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read Ananicy rule file {}", path.display()))?;
        let objects = extract_json_objects(&content)
            .with_context(|| format!("failed to parse JSON objects in {}", path.display()))?;

        for object in objects {
            let Some(rule) = community_rule_from_json_object(&object, &source_path)? else {
                continue;
            };

            let identity_key = (rule.normalized_name.clone(), rule.source_path.clone());
            if seen.insert(identity_key) {
                rules.push(rule);
            }
        }
    }

    Ok(CommunityRulesFile {
        schema_version: 1,
        source: CommunityRulesSource {
            name: input.source_name,
            repo: input.source_repo,
            commit: input.source_commit,
            generated_at: input.generated_at,
        },
        rules,
    })
}

fn collect_rule_files(dir: &Path, out: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in
        fs::read_dir(dir).with_context(|| format!("failed to read directory {}", dir.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry under {}", dir.display()))?;
        let path = entry.path();

        if path.is_dir() {
            collect_rule_files(&path, out)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rules") {
            out.push(path);
        }
    }

    Ok(())
}

fn relative_source_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn extract_json_objects(content: &str) -> anyhow::Result<Vec<Value>> {
    let mut objects = Vec::new();
    let mut buffer = String::new();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for line in content.lines() {
        let trimmed = line.trim_start();
        if depth == 0
            && (trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//"))
        {
            continue;
        }

        for ch in line.chars() {
            if depth == 0 {
                if ch == '{' {
                    buffer.clear();
                    buffer.push(ch);
                    depth = 1;
                }
                continue;
            }

            buffer.push(ch);

            if in_string {
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    in_string = false;
                }
                continue;
            }

            match ch {
                '"' => in_string = true,
                '{' => depth += 1,
                '}' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        let value: Value = serde_json::from_str(&buffer)?;
                        objects.push(value);
                        buffer.clear();
                    }
                }
                _ => {}
            }
        }

        if depth > 0 {
            buffer.push('\n');
        }
    }

    anyhow::ensure!(depth == 0, "unterminated JSON object in Ananicy rules file");
    Ok(objects)
}

fn community_rule_from_json_object(
    value: &Value,
    source_path: &str,
) -> anyhow::Result<Option<CommunityRule>> {
    let Some(object) = value.as_object() else {
        return Ok(None);
    };

    let Some(name) = string_field(object, &["name", "comm", "process", "exe"]) else {
        return Ok(None);
    };

    let Some(normalized_name) = normalize_process_name(name) else {
        return Ok(None);
    };

    let source_type = string_field(object, &["type", "category"])
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| broad_type_from_path(source_path).to_owned());

    let stutter_class = map_ananicy_category_to_task_class(&source_type, source_path);
    let ambiguous = is_ambiguous_rule_name(&normalized_name);
    let context = context_hints_for_rule(&normalized_name, source_path);
    let title = string_field(object, &["title", "description", "desc"]).map(ToOwned::to_owned);

    Ok(Some(CommunityRule {
        name: name.to_owned(),
        normalized_name,
        r#type: source_type,
        stutter_class: stutter_class.as_str().to_owned(),
        confidence: imported_confidence(stutter_class, ambiguous),
        source_path: source_path.to_owned(),
        context,
        title,
        ambiguous,
    }))
}

fn string_field<'a>(object: &'a Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    for key in keys {
        let Some(value) = object.get(*key).and_then(Value::as_str) else {
            continue;
        };
        let value = value.trim();
        if !value.is_empty() {
            return Some(value);
        }
    }

    None
}

fn broad_type_from_path(source_path: &str) -> &'static str {
    let lower = source_path.to_ascii_lowercase();

    if lower.contains("gamescope") {
        "GameScope"
    } else if lower.contains("wine") && lower.contains("server") {
        "WineServer"
    } else if lower.contains("game") || lower.contains("games") || lower.contains("wine_proton") {
        "Game"
    } else if lower.contains("browser") {
        "BrowserForeground"
    } else if lower.contains("compile") || lower.contains("compiler") {
        "Compiler"
    } else if lower.contains("build") {
        "BuildJob"
    } else if lower.contains("index") {
        "Indexer"
    } else if lower.contains("media") || lower.contains("video") || lower.contains("audio") {
        "Media"
    } else if lower.contains("terminal") || lower.contains("shell") {
        "Terminal"
    } else if lower.contains("service") || lower.contains("daemon") {
        "Service"
    } else {
        "Unknown"
    }
}

fn map_ananicy_category_to_task_class(category: &str, source_path: &str) -> TaskClass {
    let combined = format!("{category} {source_path}").to_ascii_lowercase();

    if combined.contains("gamescope") {
        TaskClass::GameScope
    } else if combined.contains("wine") && combined.contains("server") {
        TaskClass::WineServer
    } else if combined.contains("steam") && combined.contains("runtime") {
        TaskClass::SteamRuntime
    } else if combined.contains("game") || combined.contains("wine_proton") {
        TaskClass::Game
    } else if combined.contains("launcher") {
        TaskClass::Launcher
    } else if combined.contains("browser") && combined.contains("gpu") {
        TaskClass::BrowserGpu
    } else if combined.contains("browser") && combined.contains("renderer") {
        TaskClass::BrowserRenderer
    } else if combined.contains("browser") && combined.contains("network") {
        TaskClass::BrowserNetwork
    } else if combined.contains("browser") {
        TaskClass::BrowserForeground
    } else if combined.contains("compiler") || combined.contains("compile") {
        TaskClass::Compiler
    } else if combined.contains("linker") || combined.contains("link") {
        TaskClass::Linker
    } else if combined.contains("build") {
        TaskClass::BuildJob
    } else if combined.contains("index") {
        TaskClass::Indexer
    } else if combined.contains("package") {
        TaskClass::PackageManager
    } else if combined.contains("editor") {
        TaskClass::Editor
    } else if combined.contains("terminal") {
        TaskClass::Terminal
    } else if combined.contains("shell") {
        TaskClass::Shell
    } else if combined.contains("media") || combined.contains("video") || combined.contains("audio")
    {
        TaskClass::Media
    } else if combined.contains("record") {
        TaskClass::Recorder
    } else if combined.contains("virtual") || combined.contains("vm") {
        TaskClass::VirtualMachine
    } else if combined.contains("network") {
        TaskClass::NetworkDaemon
    } else if combined.contains("storage") {
        TaskClass::StorageDaemon
    } else if combined.contains("service") || combined.contains("daemon") {
        TaskClass::Service
    } else {
        TaskClass::Unknown
    }
}

fn context_hints_for_rule(normalized_name: &str, source_path: &str) -> Vec<String> {
    let mut context = Vec::new();
    let lower_path = source_path.to_ascii_lowercase();

    if lower_path.contains("wine")
        || lower_path.contains("proton")
        || lower_path.contains("steam")
        || lower_path.contains("compatdata")
        || normalized_name.ends_with(".exe")
    {
        context.push("wine_or_proton_or_steam".to_owned());
    }

    if lower_path.contains("linux") || lower_path.contains("native") {
        context.push("linux_native".to_owned());
    }

    context
}

fn is_ambiguous_rule_name(normalized_name: &str) -> bool {
    const AMBIGUOUS_EXE_NAMES: &[&str] = &[
        "build.exe",
        "launcher.exe",
        "game.exe",
        "start.exe",
        "setup.exe",
        "client.exe",
        "server.exe",
        "main.exe",
        "run.exe",
        "app.exe",
    ];

    if AMBIGUOUS_EXE_NAMES.contains(&normalized_name) {
        return true;
    }

    let stem = normalized_name
        .strip_suffix(".exe")
        .unwrap_or(normalized_name);

    stem.len() <= 3
        || matches!(
            stem,
            "app" | "run" | "main" | "start" | "setup" | "client" | "server"
        )
}

fn imported_confidence(class: TaskClass, ambiguous: bool) -> f32 {
    if ambiguous {
        0.70
    } else if class == TaskClass::Game {
        0.82
    } else if class == TaskClass::Unknown {
        0.50
    } else {
        0.68
    }
}

#[cfg(test)]
mod tests {
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
    fn importer_ignores_scheduling_policy() {
        let dir = tempdir().unwrap();
        write_rule_file(
            &dir.path().join("00-default/Games/policy.rules"),
            r#"
{"name":"policy-game.exe","type":"Game","nice":-20,"ionice":"realtime","sched":"fifo","sched_policy":"SCHED_FIFO","cpu_affinity":"0-3","systemd":"high-priority.slice"}
"#,
        );

        let imported = import_ananicy_rules(import_input(dir.path())).unwrap();
        assert_eq!(imported.rules.len(), 1);

        let serialized = serde_json::to_string(&imported).unwrap();
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

        assert_eq!(imported.rules.len(), 1);
        assert_eq!(imported.rules[0].name, r#"C:\Games\KINGDOMCOME.EXE"#);
        assert_eq!(imported.rules[0].normalized_name, "kingdomcome.exe");
        assert_eq!(imported.rules[0].stutter_class, "Game");
        assert_eq!(
            imported.rules[0].source_path,
            "00-default/Games/wine_proton/wine_proton_k.rules"
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
            .rules
            .iter()
            .find(|rule| rule.normalized_name == "build.exe")
            .unwrap();
        let specific = imported
            .rules
            .iter()
            .find(|rule| rule.normalized_name == "specificgame.exe")
            .unwrap();

        assert!(build.ambiguous);
        assert!(!specific.ambiguous);
        assert!(build.confidence <= 0.70);
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

        assert_eq!(imported.schema_version, 1);
        assert_eq!(imported.source.name, "test ananicy import");
        assert_eq!(
            imported.source.repo.as_deref(),
            Some("https://example.test/ananicy-rules.git")
        );
        assert_eq!(imported.source.commit.as_deref(), Some("abc123"));
        assert_eq!(imported.source.generated_at, "2026-05-09T00:00:00Z");
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
        let serialized = serde_json::to_string(&imported).unwrap();
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
}
