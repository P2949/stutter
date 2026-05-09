#![allow(dead_code)]

#[cfg(test)]
use std::sync::OnceLock;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::{cli::RulesCommand, process_tree::TaskClass};

pub mod import;
pub mod importer;
pub mod loader;
pub mod paths;

pub use importer::{ImportInput, import_ananicy_rules};
pub use loader::{LoadCommunityRulesInput, load_rules_db, load_rules_dir, load_rules_file};
pub use paths::{default_system_rules_dirs, default_user_rules_dir};

#[cfg(test)]
const TEST_FIXTURE_RULES_JSON: &str =
    include_str!("../assets/community-rules/ananicy.fixture.generated.json");

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
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

#[derive(Debug, Clone)]
pub struct CommunityRulesConfig {
    pub enabled: bool,
    pub load_builtin_fixture: bool,
    pub user_rules_dir: Option<PathBuf>,
    pub explicit_rules_files: Vec<PathBuf>,
}

impl Default for CommunityRulesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            load_builtin_fixture: cfg!(test),
            user_rules_dir: default_user_rules_dir(),
            explicit_rules_files: Vec::new(),
        }
    }
}

impl CommunityRulesConfig {
    pub fn from_config_file(file: crate::config_file::CommunityRulesConfigFile) -> Self {
        let mut config = Self::default();

        if let Some(enabled) = file.enabled {
            config.enabled = enabled;
        }

        config.explicit_rules_files = file.paths.unwrap_or_default();

        if let Some(sources) = file.sources {
            let wants_user = sources
                .iter()
                .any(|source| source.trim().eq_ignore_ascii_case("user"));
            let wants_fixture = sources.iter().any(|source| {
                let source = source.trim();
                source.eq_ignore_ascii_case("fixture")
                    || source.eq_ignore_ascii_case("builtin")
                    || source.eq_ignore_ascii_case("builtin_fixture")
                    || source.eq_ignore_ascii_case("builtin-fixture")
            });

            config.user_rules_dir = if wants_user {
                default_user_rules_dir()
            } else {
                None
            };
            config.load_builtin_fixture = wants_fixture;
        }

        config
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CommunityRulesMetadataFile {
    pub schema_version: u32,
    pub name: String,
    pub license: String,
    pub source_repo: Option<String>,
    pub source_commit: Option<String>,
    pub generated_at: String,
    pub generated_by: String,
    pub rule_file: String,
}

pub fn load_community_rules(config: &CommunityRulesConfig) -> anyhow::Result<CommunityRulesDb> {
    load_rules_db(LoadCommunityRulesInput {
        enabled: config.enabled,
        load_test_fixture: config.load_builtin_fixture,
        user_rules_dir: config.user_rules_dir.clone(),
        explicit_rules_files: config.explicit_rules_files.clone(),
        system_rules_dirs: default_system_rules_dirs(),
    })
}

pub fn load_community_rules_file(
    source: CommunityRulesSourceKind,
) -> anyhow::Result<CommunityRulesFile> {
    match source {
        CommunityRulesSourceKind::BuiltinFixture => {
            #[cfg(test)]
            {
                load_rules_file(Path::new("__stutter_test_fixture__"))
            }

            #[cfg(not(test))]
            {
                anyhow::bail!("built-in community rules fixture is only available in tests")
            }
        }
        CommunityRulesSourceKind::UserData => {
            let dir = default_user_rules_dir().ok_or_else(|| {
                anyhow::anyhow!(
                    "cannot locate user community rules directory because neither XDG_DATA_HOME nor HOME is set"
                )
            })?;
            let mut files = load_rules_dir(&dir)?;
            files.drain(..).next().ok_or_else(|| {
                anyhow::anyhow!("no user community rules files found in {}", dir.display())
            })
        }
        CommunityRulesSourceKind::SystemData => {
            let mut files = Vec::new();
            for dir in default_system_rules_dirs() {
                files.extend(load_rules_dir(&dir)?);
            }
            files
                .drain(..)
                .next()
                .ok_or_else(|| anyhow::anyhow!("no system community rules files found"))
        }
        CommunityRulesSourceKind::ExplicitPath(path) => load_rules_file(&path),
    }
}

pub fn load_community_rules_db(
    source: CommunityRulesSourceKind,
) -> anyhow::Result<CommunityRulesDb> {
    CommunityRulesDb::from_file(load_community_rules_file(source)?)
}

pub fn default_community_rules_dir() -> Option<PathBuf> {
    default_user_rules_dir()
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
    let input = ImportInput {
        source_dir: args.source.clone(),
        source_name: args.name.clone(),
        source_repo: args.source_repo.clone(),
        source_commit: args.source_commit.clone(),
        generated_at: generated_at.clone(),
    };

    let imported = import_ananicy_rules(input)?;
    let out_path = match args.out.clone() {
        Some(path) => path,
        None => default_imported_rules_path(&args.name)?,
    };
    let metadata_path = metadata_path_for_rules_path(&out_path, &args.name);
    let rule_file_name = out_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("ananicy.generated.json")
        .to_owned();

    let metadata = CommunityRulesMetadataFile {
        schema_version: 1,
        name: args.name.clone(),
        license: args.license.clone(),
        source_repo: args.source_repo.clone(),
        source_commit: args.source_commit.clone(),
        generated_at,
        generated_by: "stutter rules import".to_owned(),
        rule_file: rule_file_name,
    };

    if args.dry_run {
        println!(
            "dry-run: would import {} reduced rules from {}",
            imported.rules.len(),
            source_display
        );
        println!("dry-run: would write {}", out_path.display());
        println!("dry-run: would write {}", metadata_path.display());
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

    let metadata_json = serde_json::to_string_pretty(&metadata)
        .with_context(|| "failed to serialize imported community rules metadata")?;
    fs::write(&metadata_path, metadata_json).with_context(|| {
        format!(
            "failed to write imported rules metadata to {}",
            metadata_path.display()
        )
    })?;

    println!(
        "imported {} reduced rules from {}",
        imported.rules.len(),
        imported.source.name
    );
    println!("wrote {}", out_path.display());
    println!("wrote {}", metadata_path.display());
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
    default_community_rules_dir().ok_or_else(|| {
        anyhow::anyhow!(
            "cannot determine community rules directory because neither XDG_DATA_HOME nor HOME is set"
        )
    })
}

fn default_imported_rules_path(name: &str) -> anyhow::Result<PathBuf> {
    Ok(default_rules_dir()?.join(format!("{}.generated.json", sanitize_rules_name(name))))
}

fn default_imported_metadata_path(name: &str) -> anyhow::Result<PathBuf> {
    Ok(default_rules_dir()?.join(format!("{}.metadata.json", sanitize_rules_name(name))))
}

fn metadata_path_for_rules_path(rules_path: &Path, name: &str) -> PathBuf {
    rules_path
        .parent()
        .map(|parent| parent.join(format!("{}.metadata.json", sanitize_rules_name(name))))
        .unwrap_or_else(|| {
            default_imported_metadata_path(name).unwrap_or_else(|_| {
                PathBuf::from(format!("{}.metadata.json", sanitize_rules_name(name)))
            })
        })
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

pub fn classify_process_identity_with_db(
    db: &CommunityRulesDb,
    identity: &CommunityProcessIdentity<'_>,
) -> Option<CommunityRuleHit> {
    db.classify(identity, true)
}

#[cfg(test)]
static TEST_FIXTURE_RULES: OnceLock<CommunityRulesDb> = OnceLock::new();

#[cfg(test)]
pub fn classify_process_identity(
    identity: &CommunityProcessIdentity<'_>,
) -> Option<CommunityRuleHit> {
    test_fixture_rules().classify(identity, true)
}

#[cfg(test)]
fn test_fixture_rules() -> &'static CommunityRulesDb {
    TEST_FIXTURE_RULES.get_or_init(|| {
        load_community_rules_db(CommunityRulesSourceKind::BuiltinFixture)
            .expect("embedded community rules test fixture JSON must be valid")
    })
}

impl CommunityRulesDb {
    pub fn empty() -> Self {
        Self {
            rules_by_name: HashMap::new(),
        }
    }

    pub fn rule_count(&self) -> usize {
        self.rules_by_name.values().map(Vec::len).sum()
    }

    pub fn from_files(files: Vec<CommunityRulesFile>) -> anyhow::Result<Self> {
        let mut db = Self::empty();
        for file in files {
            db.merge_file(file)?;
        }
        Ok(db)
    }

    pub fn merge_file(&mut self, file: CommunityRulesFile) -> anyhow::Result<()> {
        anyhow::ensure!(
            matches!(file.schema_version, 1 | 2),
            "unsupported community rules schema version {}",
            file.schema_version
        );

        for mut rule in file.rules {
            if rule.normalized_name.trim().is_empty() {
                rule.normalized_name =
                    normalize_process_name(&rule.name).unwrap_or_else(|| rule.name.clone());
            }

            self.rules_by_name
                .entry(rule.normalized_name.clone())
                .or_default()
                .push(rule);
        }

        Ok(())
    }

    pub fn from_json(data: &str) -> anyhow::Result<Self> {
        let file: CommunityRulesFile = serde_json::from_str(data)?;
        Self::from_file(file)
    }

    pub fn from_file(file: CommunityRulesFile) -> anyhow::Result<Self> {
        let mut db = Self::empty();
        db.merge_file(file)?;
        Ok(db)
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

                let Some(context_label) =
                    classification_context_label(class, rule, identity, source, strict_context)
                else {
                    continue;
                };

                let confidence_cap = if rule.ambiguous {
                    source.confidence_cap().min(0.70)
                } else {
                    source.confidence_cap()
                };
                let confidence = rule.confidence.min(confidence_cap);
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

fn classification_context_label(
    class: TaskClass,
    rule: &CommunityRule,
    identity: &CommunityProcessIdentity<'_>,
    source: CommunityRuleIdentitySource,
    strict_context: bool,
) -> Option<&'static str> {
    match class {
        TaskClass::Unknown => None,
        TaskClass::Game => game_rule_context_label(rule, identity, strict_context),
        TaskClass::GameScope | TaskClass::WineServer | TaskClass::SteamRuntime => {
            gaming_runtime_rule_context_label(identity, strict_context)
        }
        _ => non_game_rule_context_label(rule, source),
    }
}

fn game_rule_context_label(
    rule: &CommunityRule,
    identity: &CommunityProcessIdentity<'_>,
    strict_context: bool,
) -> Option<&'static str> {
    let context_signal = game_context_signal(identity);

    if strict_context && rule_requires_context(rule) && context_signal.is_none() {
        return None;
    }

    if rule.ambiguous && context_signal.is_none() {
        return None;
    }

    Some(context_signal.unwrap_or("exact-name"))
}

fn gaming_runtime_rule_context_label(
    identity: &CommunityProcessIdentity<'_>,
    strict_context: bool,
) -> Option<&'static str> {
    let context_signal = gaming_runtime_context_signal(identity);

    if strict_context && context_signal.is_none() {
        return None;
    }

    Some(context_signal.unwrap_or("exact-name"))
}

fn non_game_rule_context_label(
    rule: &CommunityRule,
    source: CommunityRuleIdentitySource,
) -> Option<&'static str> {
    if rule.ambiguous {
        return None;
    }

    match source {
        CommunityRuleIdentitySource::ExeBasename => Some("exact-exe"),
        CommunityRuleIdentitySource::CmdlineBasename => Some("exact-cmdline"),
        CommunityRuleIdentitySource::ProcessComm | CommunityRuleIdentitySource::ThreadComm => None,
    }
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

fn gaming_runtime_context_signal(identity: &CommunityProcessIdentity<'_>) -> Option<&'static str> {
    if let Some(signal) = game_context_signal(identity) {
        return Some(signal);
    }

    let combined = [
        identity.cmdline,
        identity.exe_path,
        identity.cgroup_path,
        identity.process_comm,
        identity.thread_comm,
    ]
    .join(" ")
    .to_ascii_lowercase();

    if combined.contains("steam-runtime") {
        Some("steam-runtime")
    } else if combined.contains("steamrt") {
        Some("steamrt")
    } else if combined.contains("steam-runtime-tools") {
        Some("steam-runtime-tools")
    } else {
        None
    }
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

        rules_import_command(crate::cli::RulesImportArgs {
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
    fn non_game_rule_does_not_classify_from_comm_only() {
        let db = rules_db_with_rules(vec![rule("rustc", "Compiler")]);

        let hit = db.classify(
            &identity(
                "rustc",
                "rustc",
                "",
                "",
                "/user.slice/build.scope",
            ),
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
