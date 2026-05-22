//! Architecture gate for temporary migration markers.

use std::{collections::BTreeSet, fs};

use super::{
    crate_src_root, relative_to_crate_root,
    scanners::rust_files_under,
    transitional_allowlist::{MAX_MIGRATION_MARKER_MODULES, MIGRATION_MODULE_ALLOWLIST},
};

const MARKER: &str = concat!("Trans", "itional");
const EXIT_MARKER: &str = "Exit:";

fn bullet_list(items: &[String]) -> String {
    items
        .iter()
        .map(|item| format!("  - {item}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_meaningful_non_marker_code(source: &str) -> bool {
    source.lines().any(|line| {
        let trimmed = line.trim();
        !trimmed.is_empty()
            && !trimmed.starts_with("//")
            && !trimmed.starts_with("//!")
            && !trimmed.starts_with("#![allow")
    })
}

fn looks_like_single_zero_sized_placeholder(source: &str) -> bool {
    let meaningful = source
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with("//")
                && !line.starts_with("//!")
                && !line.starts_with("#![allow")
                && !line.starts_with("#[derive")
        })
        .collect::<Vec<_>>();

    meaningful.len() == 1
        && meaningful[0].starts_with("pub(crate) struct ")
        && meaningful[0].ends_with(';')
}

#[test]
fn temporary_migration_markers_are_tracked() {
    let mut allowance_paths = BTreeSet::new();
    let mut allowance_errors = Vec::new();

    for allowance in MIGRATION_MODULE_ALLOWLIST {
        if !allowance_paths.insert(allowance.path) {
            allowance_errors.push(format!("duplicate allowlist entry: {}", allowance.path));
        }
        if allowance.reason.trim().is_empty() {
            allowance_errors.push(format!("missing reason: {}", allowance.path));
        }
        if allowance.exit_criteria.trim().is_empty() {
            allowance_errors.push(format!("missing exit criteria: {}", allowance.path));
        }
    }

    assert!(
        allowance_errors.is_empty(),
        "migration marker allowlist errors:\n{}",
        allowance_errors.join("\n")
    );

    let mut discovered = Vec::new();
    let mut missing_exit = Vec::new();
    let mut empty_markers = Vec::new();
    let mut marker_struct_placeholders = Vec::new();

    for path in rust_files_under(&crate_src_root()) {
        let relative = relative_to_crate_root(&path);
        if relative.starts_with("src/architecture_tests/") {
            continue;
        }

        let source = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        let lines = source.lines().collect::<Vec<_>>();
        let mut has_marker = false;

        for (index, line) in lines.iter().enumerate() {
            if !line.contains(MARKER) {
                continue;
            }

            has_marker = true;
            let window_end = lines.len().min(index + 3);
            let marker_window = lines[index..window_end].join("\n");
            if !marker_window.contains(EXIT_MARKER) {
                missing_exit.push(format!("{}:{}", relative, index + 1));
            }
        }

        if has_marker {
            if !has_meaningful_non_marker_code(&source) {
                empty_markers.push(relative.clone());
            }
            if looks_like_single_zero_sized_placeholder(&source) {
                marker_struct_placeholders.push(relative.clone());
            }
            discovered.push(relative);
        }
    }

    discovered.sort();
    discovered.dedup();

    let discovered_paths = discovered
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();

    let untracked = discovered
        .iter()
        .filter(|path| !allowance_paths.contains(path.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let stale = allowance_paths
        .iter()
        .filter(|path| !discovered_paths.contains(**path))
        .map(|path| (*path).to_owned())
        .collect::<Vec<_>>();

    assert!(
        missing_exit.is_empty(),
        "migration markers missing an Exit line:\n{}",
        bullet_list(&missing_exit)
    );
    assert!(
        empty_markers.is_empty(),
        "migration marker modules with no meaningful code:\n{}",
        bullet_list(&empty_markers)
    );
    assert!(
        marker_struct_placeholders.is_empty(),
        "migration marker modules that only define a zero-sized placeholder struct:\n{}",
        bullet_list(&marker_struct_placeholders)
    );
    assert!(
        discovered.len() <= MAX_MIGRATION_MARKER_MODULES,
        "migration marker module count increased from ceiling {} to {}:\n{}",
        MAX_MIGRATION_MARKER_MODULES,
        discovered.len(),
        bullet_list(&discovered)
    );
    assert!(
        untracked.is_empty(),
        "untracked migration marker modules:\n{}",
        bullet_list(&untracked)
    );
    assert!(
        stale.is_empty(),
        "stale migration marker allowlist entries:\n{}",
        bullet_list(&stale)
    );
}
