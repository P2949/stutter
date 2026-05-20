//! Community-rules command dispatch, command reports, and imported-file management.
//!
//! Owns the `stutter rules` command boundary, imported rules file mutation, and command-only
//! report summaries. Does not own rule model definitions, database classification, loader
//! internals, normalization policy, or user-visible output rendering.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Context;

use super::{
    CommunityRulesDb, CommunityRulesFile, CommunityRulesMetadataFile, CommunityRulesSourceKind,
    default_community_rules_dir,
    importer::{ImportInput, ImportReport, import_ananicy_rules},
    is_guarded_community_rule_name, load_community_rules_db, load_community_rules_file,
    load_rules_file, normalize_process_name,
    render::{render_rules_check_report, render_rules_import_dry_run, render_rules_import_written},
    rule_requires_context,
};
use crate::{commands::input, process_tree::TaskClass};

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
    pub(crate) source_files: usize,
    pub(crate) json_objects: usize,
    pub(crate) imported_rules: usize,
    pub(crate) duplicates: usize,
    pub(crate) ambiguous_names: usize,
    pub(crate) context_required_game_rules: usize,
    pub(crate) exact_only_non_game_rules: usize,
    pub(crate) skipped_no_name: usize,
    pub(crate) skipped_bad_name: usize,
    pub(crate) skipped_unknown_class: usize,
    pub(crate) unknown_mapped_classes: usize,
    pub(crate) rules_mapped_to_unknown: usize,
    pub(crate) classes: BTreeMap<String, usize>,
    pub(crate) largest_duplicate_groups: Vec<(String, usize)>,
    pub(crate) warnings: Vec<String>,
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

pub(crate) fn rules_check_source_command(source: &Path) -> anyhow::Result<RulesCheckReport> {
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

pub(crate) fn rules_check_generated_command(generated: &Path) -> anyhow::Result<RulesCheckReport> {
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

    pub(crate) fn skipped_objects(&self) -> usize {
        self.skipped_no_name + self.skipped_bad_name + self.skipped_unknown_class
    }

    pub(crate) fn ok(&self) -> bool {
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

fn print_rules_check_report(report: &RulesCheckReport) {
    print!("{}", render_rules_check_report(report));
}

pub(crate) fn rules_import_command(args: input::RulesImportCommandInput) -> anyhow::Result<()> {
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
