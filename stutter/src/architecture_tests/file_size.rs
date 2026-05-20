//! Rust source file-size architecture tests.

use std::{fs, path::Path};

use super::{
    RUST_FILE_SIZE_LIMIT_LINES,
    allowlists::{OVERSIZED_RUST_FILE_ALLOWLIST, allowlisted_file_size},
    crate_src_root, relative_to_crate_root,
    scanners::rust_files_under,
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct RustFileLineCount {
    path: String,
    lines: usize,
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
        .map(|count| format!("{} lines {}", count.lines, count.path))
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

    let counts = rust_file_line_counts_under(&crate_src_root());
    let largest_files = largest_rust_files(&counts, 20).join("\n");
    let mut violations = Vec::new();

    for allowance in OVERSIZED_RUST_FILE_ALLOWLIST {
        match counts.iter().find(|count| count.path == allowance.path) {
            Some(count) if count.lines > allowance.max_lines => violations.push(format!(
                "{} has {} lines, exceeding allowlisted maximum {} lines; split the file or update OVERSIZED_RUST_FILE_ALLOWLIST with an explicit reason",
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
        if count.lines > RUST_FILE_SIZE_LIMIT_LINES && allowlisted_file_size(&count.path).is_none()
        {
            violations.push(format!(
                "{} has {} lines, exceeding {} lines without an allowlist entry",
                count.path, count.lines, RUST_FILE_SIZE_LIMIT_LINES
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
