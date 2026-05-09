#![allow(dead_code)]

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::{cli::RulesCommand, process_tree::TaskClass};

pub mod import;

const BUILTIN_FIXTURE_RULES_JSON: &str =
    include_str!("../assets/community-rules/test-fixture.generated.json");

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CommunityRulesFile {
    pub schema_version: u32,
    pub source: CommunityRulesSource,
    pub rules: Vec<CommunityRule>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CommunityRulesSource {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    pub generated_at: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CommunityRule {
    pub name: String,
    pub normalized_name: String,
    pub r#type: String,
    pub stutter_class: String,
    pub confidence: f32,
    pub source_path: String,
    #[serde(default)]
    pub context: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub ambiguous: bool,
}

#[derive(Debug, Clone)]
pub enum CommunityRulesSourceKind {
    BuiltinFixture,
    UserData,
    SystemData,
    ExplicitPath(PathBuf),
}

pub fn load_community_rules_file(
    source: CommunityRulesSourceKind,
) -> anyhow::Result<CommunityRulesFile> {
    match source {
        CommunityRulesSourceKind::BuiltinFixture => parse_community_rules_file(
            BUILTIN_FIXTURE_RULES_JSON,
            "embedded community rules test fixture",
        ),
        CommunityRulesSourceKind::UserData => {
            let path = active_rules_path()?;
            load_community_rules_file_from_path(&path)
        }
        CommunityRulesSourceKind::SystemData => load_community_rules_file_from_path(Path::new(
            "/usr/share/stutter/community-rules.json",
        )),
        CommunityRulesSourceKind::ExplicitPath(path) => load_community_rules_file_from_path(&path),
    }
}

pub fn load_community_rules_db(
    source: CommunityRulesSourceKind,
) -> anyhow::Result<CommunityRulesDb> {
    CommunityRulesDb::from_file(load_community_rules_file(source)?)
}

fn load_community_rules_file_from_path(path: &Path) -> anyhow::Result<CommunityRulesFile> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read community rules file {}", path.display()))?;
    parse_community_rules_file(&data, &path.display().to_string())
}

fn parse_community_rules_file(
    data: &str,
    source_label: &str,
) -> anyhow::Result<CommunityRulesFile> {
    let file: CommunityRulesFile = serde_json::from_str(data)
        .with_context(|| format!("failed to parse community rules JSON from {source_label}"))?;
    anyhow::ensure!(
        matches!(file.schema_version, 1 | 2),
        "unsupported community rules schema version {}",
        file.schema_version
    );
    Ok(file)
}

fn user_data_community_rules_path() -> Option<PathBuf> {
    active_rules_path().ok()
}

pub fn rules_command(command: RulesCommand) -> anyhow::Result<()> {
    match command {
        RulesCommand::Import(args) => rules_import_command(args),
        RulesCommand::List(_) => rules_list_command(),
        RulesCommand::Status(_) => rules_status_command(),
        RulesCommand::Enable(args) => rules_enable_command(&args.name),
        RulesCommand::Disable(_) => rules_disable_command(),
        RulesCommand::Remove(args) => rules_remove_command(&args.name, args.dry_run),
    }
}

fn rules_import_command(args: crate::cli::RulesImportArgs) -> anyhow::Result<()> {
    let generated_at = generated_at_now();
    let source_display = args.source.display().to_string();
    let input = import::ImportInput {
        source_dir: args.source.clone(),
        source_name: args.name.clone(),
        source_repo: args.source_repo.clone(),
        source_commit: args.source_commit.clone(),
        generated_at,
    };

    let imported = import::import_ananicy_rules(input)?;
    let out_path = match args.out.clone() {
        Some(path) => path,
        None => default_imported_rules_path(&args.name)?,
    };

    if args.dry_run {
        println!(
            "dry-run: would import {} reduced rules from {}",
            imported.rules.len(),
            source_display
        );
        println!("dry-run: would write {}", out_path.display());
        println!("license: {}", args.license);
        println!(
            "note: imported rules are user-installed data and are not part of the stutter binary"
        );
        return Ok(());
    }

    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create rules output directory {}",
                parent.display()
            )
        })?;
    }

    let json = serde_json::to_string_pretty(&imported)
        .with_context(|| "failed to serialize imported community rules")?;
    fs::write(&out_path, json)
        .with_context(|| format!("failed to write imported rules to {}", out_path.display()))?;

    println!(
        "imported {} reduced rules from {}",
        imported.rules.len(),
        imported.source.name
    );
    println!("wrote {}", out_path.display());
    println!("license: {}", args.license);
    println!("note: imported rules are user-installed data and are not part of the stutter binary");
    Ok(())
}

fn rules_list_command() -> anyhow::Result<()> {
    let dir = default_rules_dir()?;
    if !dir.exists() {
        println!(
            "no imported community rules directory found at {}",
            dir.display()
        );
        return Ok(());
    }

    let files = imported_rules_files(&dir)?;
    if files.is_empty() {
        println!(
            "no imported community rules files found at {}",
            dir.display()
        );
        return Ok(());
    }

    let active_path = active_rules_path()?;
    for file in files {
        let active_marker = if file == active_path { " enabled" } else { "" };
        println!("{}{}", file.display(), active_marker);
    }

    Ok(())
}

fn rules_status_command() -> anyhow::Result<()> {
    let dir = default_rules_dir()?;
    let active = active_rules_path()?;

    println!("rules directory: {}", dir.display());
    if active.exists() {
        println!("enabled rules: {}", active.display());
    } else {
        println!("enabled rules: none");
    }

    let files = if dir.exists() {
        imported_rules_files(&dir)?
    } else {
        Vec::new()
    };
    println!("imported rules files: {}", files.len());

    Ok(())
}

fn rules_enable_command(name: &str) -> anyhow::Result<()> {
    let source = default_imported_rules_path(name)?;
    anyhow::ensure!(
        source.exists(),
        "cannot enable rules named '{}': {} does not exist; run `stutter rules import --source PATH --name {}` first",
        name,
        source.display(),
        name
    );

    let active = active_rules_path()?;
    if let Some(parent) = active.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create rules directory {}", parent.display()))?;
    }

    fs::copy(&source, &active).with_context(|| {
        format!(
            "failed to enable community rules by copying {} to {}",
            source.display(),
            active.display()
        )
    })?;

    println!("enabled community rules {}", source.display());
    println!("active rules file: {}", active.display());
    Ok(())
}

fn rules_disable_command() -> anyhow::Result<()> {
    let active = active_rules_path()?;
    if active.exists() {
        fs::remove_file(&active)
            .with_context(|| format!("failed to remove active rules file {}", active.display()))?;
        println!("disabled community rules");
    } else {
        println!("community rules are already disabled");
    }

    Ok(())
}

fn rules_remove_command(name: &str, dry_run: bool) -> anyhow::Result<()> {
    let path = default_imported_rules_path(name)?;

    if dry_run {
        println!("dry-run: would remove {}", path.display());
        return Ok(());
    }

    if path.exists() {
        fs::remove_file(&path)
            .with_context(|| format!("failed to remove imported rules file {}", path.display()))?;
        println!("removed {}", path.display());
    } else {
        println!(
            "no imported community rules file found at {}",
            path.display()
        );
    }

    Ok(())
}

fn default_rules_dir() -> anyhow::Result<PathBuf> {
    if let Some(xdg_data_home) = std::env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(xdg_data_home)
            .join("stutter")
            .join("community-rules"));
    }

    if let Some(home) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("stutter")
            .join("community-rules"));
    }

    anyhow::bail!(
        "cannot determine community rules directory because neither XDG_DATA_HOME nor HOME is set"
    )
}

fn default_imported_rules_path(name: &str) -> anyhow::Result<PathBuf> {
    Ok(default_rules_dir()?.join(format!("{}.generated.json", sanitize_rules_name(name))))
}

fn active_rules_path() -> anyhow::Result<PathBuf> {
    Ok(default_rules_dir()?.join("enabled.generated.json"))
}

fn imported_rules_files(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut files = Vec::new();

    for entry in fs::read_dir(dir)
        .with_context(|| format!("failed to read community rules directory {}", dir.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry under {}", dir.display()))?;
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if file_name.ends_with(".generated.json") {
            files.push(path);
        }
    }

    files.sort();
    Ok(files)
}

fn sanitize_rules_name(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();

    if sanitized.is_empty() {
        "ananicy".to_owned()
    } else {
        sanitized
    }
}

fn generated_at_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    format!("unix-seconds:{seconds}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommunityRuleIdentitySource {
    ExeBasename,
    CmdlineBasename,
    ProcessComm,
    ThreadComm,
}

impl CommunityRuleIdentitySource {
    fn confidence_cap(self) -> f32 {
        match self {
            Self::ExeBasename => 0.90,
            Self::CmdlineBasename => 0.88,
            Self::ProcessComm | Self::ThreadComm => 0.75,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::ExeBasename => "exe basename",
            Self::CmdlineBasename => "cmdline basename",
            Self::ProcessComm => "process comm",
            Self::ThreadComm => "thread comm",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CommunityProcessIdentity<'a> {
    pub thread_comm: &'a str,
    pub process_comm: &'a str,
    pub cmdline: &'a str,
    pub exe_path: &'a str,
    pub cgroup_path: &'a str,
}

#[derive(Debug, Clone)]
pub struct CommunityRuleHit {
    pub class: TaskClass,
    pub confidence: f32,
    pub rule_name: String,
    pub source_path: String,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct CommunityRulesDb {
    rules_by_name: HashMap<String, Vec<CommunityRule>>,
}

static BUILTIN_RULES: OnceLock<CommunityRulesDb> = OnceLock::new();

pub fn classify_process_identity(
    identity: &CommunityProcessIdentity<'_>,
) -> Option<CommunityRuleHit> {
    builtin_rules().classify(identity, true)
}

fn builtin_rules() -> &'static CommunityRulesDb {
    BUILTIN_RULES.get_or_init(|| {
        load_community_rules_db(CommunityRulesSourceKind::BuiltinFixture)
            .expect("embedded community rules test fixture JSON must be valid")
    })
}

impl CommunityRulesDb {
    pub fn from_json(data: &str) -> anyhow::Result<Self> {
        let file: CommunityRulesFile = serde_json::from_str(data)?;
        Self::from_file(file)
    }

    pub fn from_file(file: CommunityRulesFile) -> anyhow::Result<Self> {
        anyhow::ensure!(
            matches!(file.schema_version, 1 | 2),
            "unsupported community rules schema version {}",
            file.schema_version
        );

        let mut rules_by_name: HashMap<String, Vec<CommunityRule>> = HashMap::new();
        for mut rule in file.rules {
            if rule.normalized_name.trim().is_empty() {
                rule.normalized_name =
                    normalize_process_name(&rule.name).unwrap_or_else(|| rule.name.clone());
            }

            rules_by_name
                .entry(rule.normalized_name.clone())
                .or_default()
                .push(rule);
        }

        Ok(Self { rules_by_name })
    }

    pub fn classify(
        &self,
        identity: &CommunityProcessIdentity<'_>,
        strict_context: bool,
    ) -> Option<CommunityRuleHit> {
        let candidates = identity_candidates(identity);
        for (candidate, source) in candidates {
            let Some(rules) = self.rules_by_name.get(&candidate) else {
                continue;
            };

            for rule in rules {
                let Some(class) = TaskClass::from_str_opt(&rule.stutter_class) else {
                    continue;
                };
                if class != TaskClass::Game {
                    continue;
                }

                let context_signal = game_context_signal(identity);
                if strict_context && rule_requires_context(rule) && context_signal.is_none() {
                    continue;
                }
                if rule.ambiguous && context_signal.is_none() {
                    continue;
                }

                let confidence_cap = if rule.ambiguous {
                    source.confidence_cap().min(0.70)
                } else {
                    source.confidence_cap()
                };
                let confidence = rule.confidence.min(confidence_cap);
                let context_label = context_signal.unwrap_or("exact-name");
                let reason = format!(
                    "community-rules: matched community rule '{}' from {}; via {}; context={}",
                    rule.name,
                    rule.source_path,
                    source.label(),
                    context_label
                );

                return Some(CommunityRuleHit {
                    class,
                    confidence,
                    rule_name: rule.name.clone(),
                    source_path: rule.source_path.clone(),
                    reason,
                });
            }
        }

        None
    }
}

pub fn normalize_process_name(value: &str) -> Option<String> {
    let mut value = value.trim();
    while let Some(stripped) = value.strip_suffix(" (deleted)") {
        value = stripped.trim_end();
    }

    value = value.trim_matches('"').trim_matches('\'');
    if value.is_empty() {
        return None;
    }

    let slash_normalized = value.replace('\\', "/");
    let basename = Path::new(&slash_normalized)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(slash_normalized.as_str())
        .trim();

    if basename.is_empty() {
        None
    } else {
        Some(basename.to_ascii_lowercase())
    }
}

fn identity_candidates(
    identity: &CommunityProcessIdentity<'_>,
) -> Vec<(String, CommunityRuleIdentitySource)> {
    let mut candidates = Vec::new();
    push_candidate(
        &mut candidates,
        identity.exe_path,
        CommunityRuleIdentitySource::ExeBasename,
    );
    if let Some(first_arg) = first_cmdline_arg(identity.cmdline) {
        push_candidate(
            &mut candidates,
            first_arg,
            CommunityRuleIdentitySource::CmdlineBasename,
        );
    }
    push_candidate(
        &mut candidates,
        identity.process_comm,
        CommunityRuleIdentitySource::ProcessComm,
    );
    push_candidate(
        &mut candidates,
        identity.thread_comm,
        CommunityRuleIdentitySource::ThreadComm,
    );
    candidates
}

fn push_candidate(
    candidates: &mut Vec<(String, CommunityRuleIdentitySource)>,
    value: &str,
    source: CommunityRuleIdentitySource,
) {
    let Some(normalized) = normalize_process_name(value) else {
        return;
    };
    if candidates
        .iter()
        .any(|(candidate, _)| candidate == &normalized)
    {
        return;
    }
    candidates.push((normalized, source));
}

fn first_cmdline_arg(cmdline: &str) -> Option<&str> {
    if cmdline.contains('\0') {
        return cmdline.split('\0').find(|arg| !arg.trim().is_empty());
    }

    cmdline
        .split_whitespace()
        .find(|arg| !arg.trim().is_empty())
}

fn rule_requires_context(rule: &CommunityRule) -> bool {
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

fn game_context_signal(identity: &CommunityProcessIdentity<'_>) -> Option<&'static str> {
    let cmdline = identity.cmdline.to_ascii_lowercase();
    let exe_path = identity.exe_path.to_ascii_lowercase();
    let cgroup_path = identity.cgroup_path.to_ascii_lowercase();
    let process_comm = identity.process_comm.to_ascii_lowercase();
    let thread_comm = identity.thread_comm.to_ascii_lowercase();

    let combined = [
        cmdline.as_str(),
        exe_path.as_str(),
        cgroup_path.as_str(),
        process_comm.as_str(),
        thread_comm.as_str(),
    ]
    .join(" ");

    if combined.contains("steamapps/") || combined.contains("\\steamapps\\") {
        Some("steamapps")
    } else if combined.contains("compatdata/") || combined.contains("\\compatdata\\") {
        Some("compatdata")
    } else if combined.contains("app-steam") {
        Some("app-steam")
    } else if combined.contains("pressure-vessel") {
        Some("pressure-vessel")
    } else if combined.contains("pv-bwrap") {
        Some("pv-bwrap")
    } else if combined.contains("gamescope") {
        Some("gamescope")
    } else if combined.contains("wineserver") {
        Some("wineserver")
    } else if combined.contains("proton") {
        Some("proton")
    } else if combined.contains("wine") {
        Some("wine")
    } else {
        None
    }
}

#[cfg(test)]
mod rules_command_tests {
    use std::fs;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn rules_import_dry_run_does_not_write() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        let out = dir.path().join("out").join("ananicy.generated.json");
        fs::create_dir_all(source.join("00-default/Games")).unwrap();
        fs::write(
            source.join("00-default/Games/example.rules"),
            r#"{"name":"example-game.exe","type":"Game","nice":-20}"#,
        )
        .unwrap();

        rules_import_command(crate::cli::RulesImportArgs {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
