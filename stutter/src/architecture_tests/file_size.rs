//! Rust source file-size architecture tests.

use std::{fs, path::Path};

use super::{
    PRODUCTION_RUST_FILE_SIZE_LIMIT_LINES, TEST_RUST_FILE_SIZE_LIMIT_LINES, WORKSPACE_SOURCE_ROOTS,
    allowlists::{OVERSIZED_RUST_FILE_ALLOWLIST, allowlisted_file_size},
    relative_to_workspace_root,
    scanners::rust_files_under,
    workspace_src_roots,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RustFileKind {
    Production,
    Test,
    EbpfMain,
    EbpfOther,
}

const EBPF_MAIN_RUST_FILE_SIZE_LIMIT_LINES: usize = 500;
const EBPF_OTHER_RUST_FILE_SIZE_LIMIT_LINES: usize = 600;

#[derive(Clone, Debug, Eq, PartialEq)]
struct RustFileLineCount {
    path: String,
    lines: usize,
    kind: RustFileKind,
}

impl RustFileLineCount {
    fn limit(&self) -> usize {
        match self.kind {
            RustFileKind::Production => PRODUCTION_RUST_FILE_SIZE_LIMIT_LINES,
            RustFileKind::Test => TEST_RUST_FILE_SIZE_LIMIT_LINES,
            RustFileKind::EbpfMain => EBPF_MAIN_RUST_FILE_SIZE_LIMIT_LINES,
            RustFileKind::EbpfOther => EBPF_OTHER_RUST_FILE_SIZE_LIMIT_LINES,
        }
    }
}

fn rust_file_kind(path: &str) -> RustFileKind {
    let is_ebpf = path.starts_with("stutter-ebpf/src/");
    let is_test_only = path == "stutter/src/architecture_tests.rs"
        || path == "stutter/src/artifact_contract_tests.rs"
        || path == "stutter/src/recording_fixture_tests.rs"
        || path == "stutter/src/test_fixture_builder.rs"
        || path.contains("/architecture_tests/")
        || path.contains("/planner_tests/")
        || path.contains("/regression_tests/")
        || path.contains("/test_fixture_builder/")
        || path.contains("/tests/")
        || path.ends_with("/tests.rs")
        || path.ends_with("_tests.rs");

    if is_ebpf {
        if path == "stutter-ebpf/src/main.rs" {
            RustFileKind::EbpfMain
        } else {
            RustFileKind::EbpfOther
        }
    } else if is_test_only {
        RustFileKind::Test
    } else {
        RustFileKind::Production
    }
}

fn rust_source_line_count(path: &Path) -> usize {
    fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
        .lines()
        .count()
}

fn rust_file_line_counts_under(path: &Path) -> Vec<RustFileLineCount> {
    rust_files_under(path)
        .into_iter()
        .map(|file| {
            let relative_path = relative_to_workspace_root(&file);

            RustFileLineCount {
                path: relative_path.clone(),
                lines: rust_source_line_count(&file),
                kind: rust_file_kind(&relative_path),
            }
        })
        .collect()
}

fn largest_rust_files(counts: &[RustFileLineCount], limit: usize) -> Vec<String> {
    let mut largest = counts.to_vec();
    largest.sort_by(|left, right| {
        right
            .lines
            .cmp(&left.lines)
            .then_with(|| left.path.cmp(&right.path))
    });
    largest
        .into_iter()
        .take(limit)
        .map(|count| format!("{} lines {:?} {}", count.lines, count.kind, count.path))
        .collect()
}

#[test]
fn rust_source_file_sizes_do_not_grow_without_architecture_allowlist() {
    for allowance in OVERSIZED_RUST_FILE_ALLOWLIST {
        assert!(
            !allowance.reason.trim().is_empty(),
            "oversized Rust file '{}' must have an allowlist reason",
            allowance.path
        );
    }

    let mut counts = Vec::new();

    for src_root in workspace_src_roots() {
        counts.extend(rust_file_line_counts_under(&src_root));
    }

    let largest_files = largest_rust_files(&counts, 20).join("\n");
    let scanned_roots = WORKSPACE_SOURCE_ROOTS.join("\n");
    let mut violations = Vec::new();

    for allowance in OVERSIZED_RUST_FILE_ALLOWLIST {
        match counts.iter().find(|count| count.path == allowance.path) {
            Some(count) if count.lines > allowance.max_lines => violations.push(format!(
                "{} has {} lines, exceeding allowlisted maximum {} lines; split the file or update OVERSIZED_RUST_FILE_ALLOWLIST with an explicit reason",
                count.path, count.lines, allowance.max_lines
            )),
            Some(count) if count.lines <= count.limit() => violations.push(format!(
                "allowlisted oversized Rust file '{}' now has {} lines, at or below the {:?} {} line limit; remove its allowlist entry",
                count.path, count.lines, count.kind, count.limit()
            )),
            Some(count) if count.lines < allowance.max_lines => violations.push(format!(
                "allowlisted oversized Rust file '{}' now has {} lines, below allowlisted maximum {}; lower the allowlist ceiling",
                count.path, count.lines, allowance.max_lines
            )),
            Some(_) => {}
            None => violations.push(format!(
                "allowlisted oversized Rust file '{}' no longer exists; remove or update its allowlist entry",
                allowance.path
            )),
        }
    }

    for count in &counts {
        if count.lines > count.limit() && allowlisted_file_size(&count.path).is_none() {
            violations.push(format!(
                "{} has {} lines, exceeding {:?} {} line limit without an allowlist entry",
                count.path,
                count.lines,
                count.kind,
                count.limit()
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "Rust source file size gate failed:\n{}\n\nscanned source roots:\n{}\n\nlargest Rust files:\n{}",
        violations.join("\n"),
        scanned_roots,
        largest_files
    );
}

#[test]
fn no_oversized_production_files_are_currently_allowlisted() {
    assert!(
        OVERSIZED_RUST_FILE_ALLOWLIST.is_empty(),
        "All oversized production files have been split. The allowlist should remain empty."
    );
}

#[cfg(test)]
mod tests {
    use super::{RustFileKind, WORKSPACE_SOURCE_ROOTS, rust_file_kind, workspace_src_roots};

    #[test]
    fn workspace_source_roots_cover_all_workspace_crate_src_dirs() {
        assert_eq!(
            WORKSPACE_SOURCE_ROOTS,
            &[
                "stutter/src",
                "stutter-ebpf/src",
                "stutter-common/src",
                "stutter-config/src",
                "stutter-core/src",
                "stutter-report/src",
                "xtask/src",
            ]
        );

        let roots = workspace_src_roots();
        assert_eq!(roots.len(), WORKSPACE_SOURCE_ROOTS.len());

        for root in roots {
            assert!(
                root.is_dir(),
                "workspace source root should exist: {}",
                root.display()
            );
        }
    }

    #[test]
    fn classifies_test_only_source_paths() {
        assert_eq!(
            rust_file_kind("stutter/src/cli/report/tests.rs"),
            RustFileKind::Test
        );
        assert_eq!(
            rust_file_kind("stutter/src/autotune/planner_tests/workload_policy.rs"),
            RustFileKind::Test
        );
        assert_eq!(
            rust_file_kind("stutter/src/architecture_tests/file_size.rs"),
            RustFileKind::Test
        );
        assert_eq!(
            rust_file_kind("stutter/src/recording_fixture_tests.rs"),
            RustFileKind::Test
        );
    }

    #[test]
    fn classifies_regular_source_paths_as_production() {
        assert_eq!(
            rust_file_kind("stutter/src/session_io/mod.rs"),
            RustFileKind::Production
        );
        assert_eq!(
            rust_file_kind("stutter/src/autotune/runtime/mod.rs"),
            RustFileKind::Production
        );
        assert_eq!(
            rust_file_kind("stutter-report/src/model/mod.rs"),
            RustFileKind::Production
        );
        assert_eq!(
            rust_file_kind("stutter-config/src/config_model.rs"),
            RustFileKind::Production
        );
        assert_eq!(
            rust_file_kind("xtask/src/maturity_report.rs"),
            RustFileKind::Production
        );
    }

    #[test]
    fn classifies_ebpf_source_paths() {
        assert_eq!(
            rust_file_kind("stutter-ebpf/src/main.rs"),
            RustFileKind::EbpfMain
        );
        assert_eq!(
            rust_file_kind("stutter-ebpf/src/scheduler.rs"),
            RustFileKind::EbpfOther
        );
    }
}
