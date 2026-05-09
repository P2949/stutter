use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde_json::{Map, Value};

use super::{
    CommunityRule, CommunityRulesFile, CommunityRulesSource, is_guarded_community_rule_name,
    normalize_process_name,
};
use crate::process_tree::TaskClass;

#[derive(Debug, Clone)]
pub struct ImportInput {
    pub source_dir: PathBuf,
    pub source_name: String,
    pub source_repo: Option<String>,
    pub source_commit: Option<String>,
    pub generated_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub scanned_files: usize,
    pub parsed_objects: usize,
    pub imported_rules: usize,
    pub skipped_no_name: usize,
    pub skipped_bad_name: usize,
    pub skipped_unknown_class: usize,
    pub duplicate_rules: usize,
    pub ambiguous_rules: usize,
    pub context_required_game_rules: usize,
    pub exact_only_non_game_rules: usize,
    pub classes: BTreeMap<String, usize>,
}

#[derive(Debug, Clone)]
pub struct ImportedCommunityRules {
    pub file: CommunityRulesFile,
    pub report: ImportReport,
}

#[derive(Debug, Clone)]
struct ParsedRuleObject {
    value: Value,
    preceding_comments: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct RuleCommentEvidence {
    title: Option<String>,
    source_url: Option<String>,
    comment: Option<String>,
}

enum RuleImportDecision {
    Import(Box<CommunityRule>),
    SkipNoName,
    SkipBadName,
    SkipUnknownClass,
}

pub fn import_ananicy_rules(input: ImportInput) -> anyhow::Result<ImportedCommunityRules> {
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
    let mut report = ImportReport {
        scanned_files: rule_files.len(),
        ..ImportReport::default()
    };

    for path in rule_files {
        let source_path = relative_source_path(&input.source_dir, &path);
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read Ananicy rule file {}", path.display()))?;
        let objects = extract_json_objects(&content)
            .with_context(|| format!("failed to parse JSON objects in {}", path.display()))?;

        for object in objects {
            report.parsed_objects += 1;

            let rule = match community_rule_from_json_object(&object, &source_path) {
                RuleImportDecision::Import(rule) => *rule,
                RuleImportDecision::SkipNoName => {
                    report.skipped_no_name += 1;
                    continue;
                }
                RuleImportDecision::SkipBadName => {
                    report.skipped_bad_name += 1;
                    continue;
                }
                RuleImportDecision::SkipUnknownClass => {
                    report.skipped_unknown_class += 1;
                    continue;
                }
            };

            let key = (rule.normalized_name.clone(), rule.source_path.clone());
            if !seen.insert(key) {
                report.duplicate_rules += 1;
                continue;
            }

            let class = TaskClass::from_str_opt(&rule.stutter_class).unwrap_or(TaskClass::Unknown);

            if rule.ambiguous {
                report.ambiguous_rules += 1;
            }

            if class == TaskClass::Game && rule_requires_game_context_at_import(&rule) {
                report.context_required_game_rules += 1;
            }

            if exact_only_non_game_rule_at_import(class, &rule) {
                report.exact_only_non_game_rules += 1;
            }

            *report
                .classes
                .entry(rule.stutter_class.clone())
                .or_default() += 1;
            rules.push(rule);
        }
    }

    report.imported_rules = rules.len();

    Ok(ImportedCommunityRules {
        file: CommunityRulesFile {
            schema_version: 2,
            source: CommunityRulesSource {
                name: input.source_name,
                repo: input.source_repo,
                commit: input.source_commit,
                generated_at: input.generated_at,
            },
            rules,
        },
        report,
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

fn extract_json_objects(content: &str) -> anyhow::Result<Vec<ParsedRuleObject>> {
    let mut objects = Vec::new();
    let mut buffer = String::new();
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut pending_comments: Vec<String> = Vec::new();
    let mut object_comments: Vec<String> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim_start();

        if depth == 0 {
            if trimmed.is_empty() {
                pending_comments.clear();
                continue;
            }

            if let Some(comment) = comment_from_trimmed_line(trimmed) {
                if !comment.is_empty() {
                    pending_comments.push(comment);
                }
                continue;
            }
        }

        let mut saw_object_start_on_line = false;

        for ch in line.chars() {
            if depth == 0 {
                if ch == '{' {
                    buffer.clear();
                    buffer.push(ch);
                    depth = 1;
                    saw_object_start_on_line = true;
                    object_comments = std::mem::take(&mut pending_comments);
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
                        objects.push(ParsedRuleObject {
                            value,
                            preceding_comments: std::mem::take(&mut object_comments),
                        });
                        buffer.clear();
                    }
                }
                _ => {}
            }
        }

        if depth > 0 {
            buffer.push('\n');
        } else if !saw_object_start_on_line && !trimmed.is_empty() {
            pending_comments.clear();
        }
    }

    anyhow::ensure!(depth == 0, "unterminated JSON object in Ananicy rules file");
    Ok(objects)
}

fn comment_from_trimmed_line(trimmed: &str) -> Option<String> {
    if let Some(comment) = trimmed.strip_prefix('#') {
        return Some(comment.trim().to_owned());
    }

    if let Some(comment) = trimmed.strip_prefix("//") {
        return Some(comment.trim().to_owned());
    }

    None
}

fn community_rule_from_json_object(
    parsed: &ParsedRuleObject,
    source_path: &str,
) -> RuleImportDecision {
    let Some(object) = parsed.value.as_object() else {
        return RuleImportDecision::SkipNoName;
    };

    let Some(name) = string_field(object, &["name", "comm", "process", "exe"]) else {
        return RuleImportDecision::SkipNoName;
    };

    let Some(normalized_name) = normalize_process_name(name) else {
        return RuleImportDecision::SkipBadName;
    };

    let source_type = string_field(object, &["type", "category"])
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| broad_type_from_path(source_path).to_owned());

    let Some(stutter_class) = map_ananicy_category_to_task_class(&source_type, source_path) else {
        return RuleImportDecision::SkipUnknownClass;
    };

    let ambiguous = is_import_ambiguous_rule_name(&normalized_name);
    let context = context_hints_for_rule(&normalized_name, source_path);
    let evidence = comment_evidence(object, &parsed.preceding_comments);

    RuleImportDecision::Import(Box::new(CommunityRule {
        name: name.to_owned(),
        normalized_name,
        r#type: source_type,
        stutter_class: stutter_class.as_str().to_owned(),
        confidence: imported_confidence(stutter_class, ambiguous),
        source_path: source_path.to_owned(),
        context,
        title: evidence.title,
        source_url: evidence.source_url,
        comment: evidence.comment,
        ambiguous,
    }))
}

fn comment_evidence(object: &Map<String, Value>, comments: &[String]) -> RuleCommentEvidence {
    let json_title = string_field(object, &["title", "description", "desc"]).map(ToOwned::to_owned);
    let json_source_url =
        string_field(object, &["source_url", "store_url", "url"]).map(ToOwned::to_owned);

    let joined_comment = comments
        .iter()
        .map(|comment| comment.trim())
        .filter(|comment| !comment.is_empty())
        .collect::<Vec<_>>()
        .join(" ");

    let comment = if joined_comment.is_empty() {
        None
    } else {
        Some(joined_comment)
    };

    let comment_source_url = comment.as_deref().and_then(extract_first_url_from_comment);
    let source_url = json_source_url.or(comment_source_url.clone());
    let title = json_title.or_else(|| {
        comment
            .as_deref()
            .and_then(|comment| title_from_comment(comment, source_url.as_deref()))
    });

    RuleCommentEvidence {
        title,
        source_url,
        comment,
    }
}

fn extract_first_url_from_comment(comment: &str) -> Option<String> {
    comment.split_whitespace().find_map(|token| {
        let token = token.trim_matches(|ch: char| {
            matches!(
                ch,
                '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | '"' | '\''
            )
        });
        let token = token.trim_end_matches(|ch: char| {
            matches!(ch, ',' | '.' | ';' | ':' | ')' | ']' | '}' | '"' | '\'')
        });

        if token.starts_with("https://") || token.starts_with("http://") {
            Some(token.to_owned())
        } else {
            None
        }
    })
}

fn title_from_comment(comment: &str, source_url: Option<&str>) -> Option<String> {
    let without_url = if let Some(source_url) = source_url {
        comment.replacen(source_url, "", 1)
    } else {
        comment.to_owned()
    };

    let title = without_url
        .trim()
        .trim_matches(|ch: char| matches!(ch, '-' | ':' | '|' | '–' | '—'))
        .trim()
        .to_owned();

    if title.is_empty() { None } else { Some(title) }
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

fn map_ananicy_category_to_task_class(category: &str, source_path: &str) -> Option<TaskClass> {
    let combined = format!("{category} {source_path}").to_ascii_lowercase();

    let class = if combined.contains("gamescope") {
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
        return None;
    };

    Some(class)
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

fn is_import_ambiguous_rule_name(normalized_name: &str) -> bool {
    is_guarded_community_rule_name(normalized_name)
}

fn rule_requires_game_context_at_import(rule: &CommunityRule) -> bool {
    rule.ambiguous
        || rule
            .context
            .iter()
            .any(|context| context == "wine_or_proton_or_steam")
        || rule
            .source_path
            .to_ascii_lowercase()
            .contains("wine_proton")
}

fn exact_only_non_game_rule_at_import(class: TaskClass, rule: &CommunityRule) -> bool {
    !rule.ambiguous
        && !matches!(
            class,
            TaskClass::Unknown
                | TaskClass::Game
                | TaskClass::GameScope
                | TaskClass::WineServer
                | TaskClass::SteamRuntime
        )
}

fn imported_confidence(class: TaskClass, ambiguous: bool) -> f32 {
    if ambiguous {
        0.70
    } else if class == TaskClass::Game {
        0.82
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
            imported.file.rules.iter().all(|rule| rule.ambiguous
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
}
