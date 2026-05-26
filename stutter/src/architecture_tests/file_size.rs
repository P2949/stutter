//! Rust source file-size architecture tests.

use std::{fs, path::Path};

use super::{
    PRODUCTION_RUST_FILE_SIZE_LIMIT_LINES, TEST_RUST_FILE_SIZE_LIMIT_LINES,
    allowlists::{OVERSIZED_RUST_FILE_ALLOWLIST, allowlisted_file_size},
    crate_src_root, relative_to_crate_root,
    scanners::rust_files_under,
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
    let is_ebpf = path.contains("stutter-ebpf/src/");
    let is_test_only = path == "src/architecture_tests.rs"
        || path == "src/artifact_contract_tests.rs"
        || path == "src/recording_fixture_tests.rs"
        || path == "src/test_fixture_builder.rs"
        || path.contains("/architecture_tests/")
        || path.contains("/planner_tests/")
        || path.contains("/regression_tests/")
        || path.contains("/test_fixture_builder/")
        || path.contains("/tests/")
        || path.ends_with("/tests.rs")
        || path.ends_with("_tests.rs");

    if is_ebpf {
        if path.ends_with("stutter-ebpf/src/main.rs") {
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
        .map(|file| RustFileLineCount {
            path: relative_to_crate_root(&file),
            lines: rust_source_line_count(&file),
            kind: rust_file_kind(&relative_to_crate_root(&file)),
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

    let mut counts = rust_file_line_counts_under(&crate_src_root());
    let ebpf_src = crate_src_root()
        .parent()
        .unwrap()
        .join("stutter-ebpf")
        .join("src");
    counts.extend(rust_file_line_counts_under(&ebpf_src));

    let largest_files = largest_rust_files(&counts, 20).join("\n");
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
        "Rust source file size gate failed:\n{}\n\nlargest Rust files:\n{}",
        violations.join("\n"),
        largest_files
    );
}

#[cfg(test)]
mod tests {
    use super::{RustFileKind, rust_file_kind};

    #[test]
    fn classifies_test_only_source_paths() {
        assert_eq!(
            rust_file_kind("src/cli/report/tests.rs"),
            RustFileKind::Test
        );
        assert_eq!(
            rust_file_kind("src/autotune/planner_tests/workload_policy.rs"),
            RustFileKind::Test
        );
        assert_eq!(
            rust_file_kind("src/architecture_tests/file_size.rs"),
            RustFileKind::Test
        );
        assert_eq!(
            rust_file_kind("src/recording_fixture_tests.rs"),
            RustFileKind::Test
        );
    }

    #[test]
    fn classifies_regular_source_paths_as_production() {
        assert_eq!(
            rust_file_kind("src/session_io.rs"),
            RustFileKind::Production
        );
        assert_eq!(
            rust_file_kind("src/autotune/runtime/mod.rs"),
            RustFileKind::Production
        );
    }
}
