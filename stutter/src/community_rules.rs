#[cfg(test)]
use std::sync::OnceLock;
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::{commands::input, process_tree::TaskClass};

pub mod importer;
pub mod loader;
pub mod paths;

pub mod classify;
pub mod commands;
pub mod db;
pub mod import;
pub mod load;
pub mod model;
pub mod normalize;
pub mod render;

use importer::{ImportInput, ImportReport, import_ananicy_rules};
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
    #[cfg(test)]
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

#[derive(Debug, Clone)]
pub enum CommunityRulesStatus {
    Loaded { db: CommunityRulesDb },
    Disabled,
    Failed { error: String },
}

impl CommunityRulesStatus {
    pub fn as_db(&self) -> Option<&CommunityRulesDb> {
        match self {
            Self::Loaded { db } => Some(db),
            Self::Disabled | Self::Failed { .. } => None,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Loaded { .. } => "loaded",
            Self::Disabled => "disabled",
            Self::Failed { .. } => "failed",
        }
    }
}

pub fn load_community_rules_status(config: &CommunityRulesConfig) -> CommunityRulesStatus {
    if !config.enabled {
        return CommunityRulesStatus::Disabled;
    }

    match load_community_rules(config) {
        Ok(db) => CommunityRulesStatus::Loaded { db },
        Err(error) => CommunityRulesStatus::Failed {
            error: format!("{error:#}"),
        },
    }
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
        #[cfg(test)]
        CommunityRulesSourceKind::BuiltinFixture => {
            load_rules_file(Path::new("__stutter_test_fixture__"))
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

pub fn rules_command(command: input::RulesCommand) -> anyhow::Result<()> {
    match command {
        input::RulesCommand::Import(args) => rules_import_command(args),
        input::RulesCommand::Check(args) => rules_check_command(args),
        input::RulesCommand::List => rules_list_command(),
        input::RulesCommand::Status => rules_status_command(),
        input::RulesCommand::Enable(args) => rules_enable_command(&args.name),
        input::RulesCommand::Disable => rules_disable_command(),
        input::RulesCommand::Remove(args) => rules_remove_command(&args.name, args.dry_run),
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RulesCheckReport {
    source_files: usize,
    json_objects: usize,
    imported_rules: usize,
    duplicates: usize,
    ambiguous_names: usize,
    context_required_game_rules: usize,
    exact_only_non_game_rules: usize,
    skipped_no_name: usize,
    skipped_bad_name: usize,
    skipped_unknown_class: usize,
    unknown_mapped_classes: usize,
    rules_mapped_to_unknown: usize,
    classes: BTreeMap<String, usize>,
    largest_duplicate_groups: Vec<(String, usize)>,
    warnings: Vec<String>,
}

fn rules_check_command(args: input::RulesCheckArgs) -> anyhow::Result<()> {
    let report = match (args.source, args.generated) {
        (Some(source), None) => rules_check_source_command(&source)?,
        (None, Some(generated)) => rules_check_generated_command(&generated)?,
        _ => anyhow::bail!("rules check requires either --source PATH or --generated PATH"),
    };

    print_rules_check_report(&report);
    Ok(())
}

fn rules_check_source_command(source: &Path) -> anyhow::Result<RulesCheckReport> {
    let input = ImportInput {
        source_dir: source.to_path_buf(),
        source_name: "rules check".to_owned(),
        source_repo: None,
        source_commit: None,
        generated_at: generated_at_now(),
    };

    let imported = import_ananicy_rules(input)?;
    let file = imported.file;
    let import_report = imported.report;

    CommunityRulesDb::from_file(file.clone())?;

    Ok(RulesCheckReport::from_source_import(file, import_report))
}

fn rules_check_generated_command(generated: &Path) -> anyhow::Result<RulesCheckReport> {
    let file = load_rules_file(generated)?;
    CommunityRulesDb::from_file(file.clone())?;
    Ok(RulesCheckReport::from_generated_file(file))
}

impl RulesCheckReport {
    fn from_source_import(file: CommunityRulesFile, import_report: ImportReport) -> Self {
        let mut report = Self::from_rules_file(&file);
        report.source_files = import_report.scanned_files;
        report.json_objects = import_report.parsed_objects;
        report.imported_rules = import_report.imported_rules;
        report.skipped_no_name = import_report.skipped_no_name;
        report.skipped_bad_name = import_report.skipped_bad_name;
        report.skipped_unknown_class = import_report.skipped_unknown_class;
        report.duplicates += import_report.duplicate_rules;
        report.ambiguous_names = import_report.ambiguous_rules;
        report.context_required_game_rules = import_report.context_required_game_rules;
        report.exact_only_non_game_rules = import_report.exact_only_non_game_rules;

        for (class, count) in import_report.classes {
            report.classes.entry(class).or_insert(count);
        }

        report
    }

    fn from_generated_file(file: CommunityRulesFile) -> Self {
        let mut report = Self::from_rules_file(&file);
        report.source_files = 1;
        report.json_objects = file.rules.len();
        report.imported_rules = file.rules.len();
        report
    }

    fn from_rules_file(file: &CommunityRulesFile) -> Self {
        let mut report = Self::default();
        let mut name_counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut warning_counts: BTreeMap<String, usize> = BTreeMap::new();

        for rule in &file.rules {
            let normalized_name = if rule.normalized_name.trim().is_empty() {
                normalize_process_name(&rule.name).unwrap_or_else(|| rule.name.clone())
            } else {
                rule.normalized_name.clone()
            };

            *name_counts.entry(normalized_name.clone()).or_default() += 1;
            *report
                .classes
                .entry(rule.stutter_class.clone())
                .or_default() += 1;

            let class = TaskClass::from_str_opt(&rule.stutter_class);
            match class {
                None => {
                    report.unknown_mapped_classes += 1;
                }
                Some(TaskClass::Unknown) => {
                    report.rules_mapped_to_unknown += 1;
                }
                Some(class) => {
                    let guarded = is_guarded_community_rule_name(&normalized_name);
                    let ambiguous = rule.ambiguous || guarded;

                    if ambiguous {
                        report.ambiguous_names += 1;
                    }

                    if class == TaskClass::Game && (ambiguous || rule_requires_context(rule)) {
                        report.context_required_game_rules += 1;
                        let warning = format!("{normalized_name} requires Steam/Proton context");
                        *warning_counts.entry(warning).or_default() += 1;
                    }

                    if exact_only_non_game_check_rule(class, ambiguous) {
                        report.exact_only_non_game_rules += 1;
                    }

                    if ambiguous && !matches!(class, TaskClass::Game) {
                        let warning = format!(
                            "{normalized_name} is guarded and will not classify as a non-game rule without stronger evidence"
                        );
                        *warning_counts.entry(warning).or_default() += 1;
                    }
                }
            }
        }

        let mut duplicate_groups = name_counts
            .into_iter()
            .filter(|(_, count)| *count > 1)
            .collect::<Vec<_>>();
        duplicate_groups
            .sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

        report.duplicates = duplicate_groups
            .iter()
            .map(|(_, count)| count.saturating_sub(1))
            .sum();
        report.largest_duplicate_groups = duplicate_groups.into_iter().take(10).collect();
        report.warnings = warning_counts.into_keys().take(20).collect();
        report
    }

    fn skipped_objects(&self) -> usize {
        self.skipped_no_name + self.skipped_bad_name + self.skipped_unknown_class
    }

    fn ok(&self) -> bool {
        self.unknown_mapped_classes == 0 && self.rules_mapped_to_unknown == 0
    }
}

fn exact_only_non_game_check_rule(class: TaskClass, ambiguous: bool) -> bool {
    !ambiguous
        && !matches!(
            class,
            TaskClass::Unknown
                | TaskClass::Game
                | TaskClass::GameScope
                | TaskClass::WineServer
                | TaskClass::SteamRuntime
        )
}

pub(crate) fn render_rules_check_report(report: &RulesCheckReport) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "rules check: {}\n",
        if report.ok() { "ok" } else { "warning" }
    ));
    out.push_str(&format!("source files: {}\n", report.source_files));
    out.push_str(&format!("json objects: {}\n", report.json_objects));
    out.push_str(&format!("imported rules: {}\n", report.imported_rules));
    out.push_str(&format!("objects skipped: {}\n", report.skipped_objects()));
    out.push_str(&format!("duplicates: {}\n", report.duplicates));
    out.push_str(&format!("ambiguous names: {}\n", report.ambiguous_names));
    out.push_str(&format!(
        "context-required game rules: {}\n",
        report.context_required_game_rules
    ));
    out.push_str(&format!(
        "exact-only non-game rules: {}\n",
        report.exact_only_non_game_rules
    ));
    out.push_str(&format!(
        "unknown mapped classes: {}\n",
        report.unknown_mapped_classes
    ));
    out.push_str(&format!(
        "rules mapped to Unknown: {}\n",
        report.rules_mapped_to_unknown
    ));

    if report.classes.is_empty() {
        out.push_str("classes: none\n");
    } else {
        out.push_str("classes:\n");
        for (class, count) in &report.classes {
            out.push_str(&format!("  {class}: {count}\n"));
        }
    }

    if !report.largest_duplicate_groups.is_empty() {
        out.push_str("largest duplicate groups:\n");
        for (name, count) in &report.largest_duplicate_groups {
            out.push_str(&format!("  {name}: {count}\n"));
        }
    }

    if !report.warnings.is_empty() {
        out.push_str("warnings:\n");
        for warning in &report.warnings {
            out.push_str(&format!("  {warning}\n"));
        }
    }

    out
}

fn print_rules_check_report(report: &RulesCheckReport) {
    print!("{}", render_rules_check_report(report));
}

fn rules_import_command(args: input::RulesImportCommandInput) -> anyhow::Result<()> {
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
    let report = imported.report;
    let imported = imported.file;
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
        print!(
            "{}",
            render_rules_import_dry_run(
                &source_display,
                &report,
                &out_path,
                &metadata_path,
                &args.license
            )
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

    print!(
        "{}",
        render_rules_import_written(
            &report,
            &imported.source.name,
            &out_path,
            &metadata_path,
            &args.license
        )
    );
    Ok(())
}

pub(crate) fn render_import_report(report: &ImportReport) -> String {
    let mut out = String::new();
    out.push_str("import report:\n");
    out.push_str(&format!("  scanned_files: {}\n", report.scanned_files));
    out.push_str(&format!("  parsed_objects: {}\n", report.parsed_objects));
    out.push_str(&format!("  imported_rules: {}\n", report.imported_rules));
    out.push_str(&format!("  skipped_no_name: {}\n", report.skipped_no_name));
    out.push_str(&format!(
        "  skipped_bad_name: {}\n",
        report.skipped_bad_name
    ));
    out.push_str(&format!(
        "  skipped_unknown_class: {}\n",
        report.skipped_unknown_class
    ));
    out.push_str(&format!("  duplicate_rules: {}\n", report.duplicate_rules));
    out.push_str(&format!("  ambiguous_rules: {}\n", report.ambiguous_rules));
    out.push_str(&format!(
        "  context_required_game_rules: {}\n",
        report.context_required_game_rules
    ));
    out.push_str(&format!(
        "  exact_only_non_game_rules: {}\n",
        report.exact_only_non_game_rules
    ));

    if report.classes.is_empty() {
        out.push_str("  classes: none\n");
    } else {
        out.push_str("  classes:\n");
        for (class, count) in &report.classes {
            out.push_str(&format!("    {class}: {count}\n"));
        }
    }

    out
}

pub(crate) fn render_rules_import_dry_run(
    source_display: &str,
    report: &ImportReport,
    out_path: &Path,
    metadata_path: &Path,
    license: &str,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "dry-run: analyzed Ananicy-compatible rules from {source_display}\n"
    ));
    out.push_str(&render_import_report(report));
    out.push_str(&format!("dry-run: would write {}\n", out_path.display()));
    out.push_str(&format!(
        "dry-run: would write {}\n",
        metadata_path.display()
    ));
    out.push_str(&format!("license: {license}\n"));
    out.push_str(
        "note: imported rules are user-installed data and are not part of the stutter binary\n",
    );
    out
}

pub(crate) fn render_rules_import_written(
    report: &ImportReport,
    source_name: &str,
    out_path: &Path,
    metadata_path: &Path,
    license: &str,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "imported {} reduced rules from {source_name}\n",
        report.imported_rules
    ));
    out.push_str(&format!("wrote {}\n", out_path.display()));
    out.push_str(&format!("wrote {}\n", metadata_path.display()));
    out.push_str(&format!("license: {license}\n"));
    out.push_str(
        "note: imported rules are user-installed data and are not part of the stutter binary\n",
    );
    out
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
        println!(
            "enabled rules status: {}",
            describe_rules_db(CommunityRulesSourceKind::ExplicitPath(active.clone()))
        );
    } else {
        println!("enabled rules: none");
        println!("enabled rules status: not loaded");
    }

    let files = if dir.exists() {
        imported_rules_files(&dir)?
    } else {
        Vec::new()
    };
    println!("imported rules files: {}", files.len());
    println!(
        "user rules source: {}",
        describe_rules_file(CommunityRulesSourceKind::UserData)
    );
    println!(
        "system rules source: {}",
        describe_rules_file(CommunityRulesSourceKind::SystemData)
    );

    Ok(())
}

fn describe_rules_file(source: CommunityRulesSourceKind) -> String {
    match load_community_rules_file(source) {
        Ok(file) => format!(
            "available name={} rules={} generated_at={}",
            file.source.name,
            file.rules.len(),
            file.source.generated_at
        ),
        Err(error) => format!("unavailable ({error:#})"),
    }
}

fn describe_rules_db(source: CommunityRulesSourceKind) -> String {
    match load_community_rules_db(source) {
        Ok(db) => format!("loaded rules={}", db.rule_count()),
        Err(error) => format!("failed ({error:#})"),
    }
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
            Self::ProcessComm => 0.75,
            Self::ThreadComm => 0.65,
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

pub fn is_guarded_community_rule_name(normalized_name: &str) -> bool {
    const GUARDED_NAMES: &[&str] = &[
        "python",
        "python3",
        "java",
        "node",
        "wine",
        "bash",
        "sh",
        "zsh",
        "steam",
        "steamwebhelper",
        "electron",
        "chrome",
        "firefox",
        "setup.exe",
        "launcher.exe",
        "client.exe",
        "server.exe",
        "main.exe",
        "build.exe",
        "run.exe",
        "app.exe",
        "game.exe",
        "start.exe",
    ];

    if GUARDED_NAMES.contains(&normalized_name) {
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

            if is_guarded_community_rule_name(&rule.normalized_name) {
                rule.ambiguous = true;
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

                let confidence_cap = confidence_cap_for_rule(class, rule, source);
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

fn confidence_cap_for_rule(
    class: TaskClass,
    rule: &CommunityRule,
    source: CommunityRuleIdentitySource,
) -> f32 {
    let mut cap = source.confidence_cap();

    if rule.ambiguous {
        cap = cap.min(0.70);
    }

    match class {
        TaskClass::Unknown => 0.0,
        TaskClass::Game => cap,
        TaskClass::GameScope | TaskClass::WineServer | TaskClass::SteamRuntime => cap,
        TaskClass::Service | TaskClass::NetworkDaemon | TaskClass::StorageDaemon => {
            if service_rule_source_path_is_specific(rule) {
                cap.min(0.80)
            } else {
                cap.min(0.60)
            }
        }
        _ => cap.min(0.80),
    }
}

fn service_rule_source_path_is_specific(rule: &CommunityRule) -> bool {
    let source_path = rule.source_path.to_ascii_lowercase();

    if source_path.contains("systemd")
        || source_path.contains("dbus")
        || source_path.contains("network")
        || source_path.contains("storage")
        || source_path.contains("daemon")
        || source_path.contains("service")
    {
        return true;
    }

    source_path
        .split('/')
        .filter(|component| !component.trim().is_empty())
        .count()
        >= 3
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
            include_str!("../tests/snapshots/community_rules_import_report.txt")
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
}
