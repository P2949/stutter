//! Community-rules command output rendering.
//!
//! Owns string formatting for `stutter rules` command reports. Does not own command dispatch,
//! filesystem mutation, rule loading, or classification policy.

use std::path::Path;

use super::{commands::RulesCheckReport, importer::ImportReport};

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
