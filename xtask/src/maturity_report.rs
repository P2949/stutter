use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

const CLEANUP_BASELINE_PATH: &str = "docs/internal/archive/cleanup-baseline.md";
const MAX_FILE_SIZE: usize = 1000;
const LARGEST_FILE_LIMIT: usize = 20;

#[derive(Debug)]
struct BaselineEntry {
    old_loc: usize,
    path: String,
}

pub fn run_maturity_report(root: &Path) -> Result<()> {
    let baseline_path = root.join(CLEANUP_BASELINE_PATH);
    let content = fs::read_to_string(&baseline_path).with_context(|| {
        format!(
            "failed to read maturity-report baseline at {}",
            baseline_path.display()
        )
    })?;

    let mut baseline = Vec::new();
    let mut parsing_table = false;

    for line in content.lines() {
        if line.starts_with("| LOC") || line.starts_with("|------") {
            parsing_table = true;
            continue;
        }
        if parsing_table && line.starts_with('|') {
            let parts: Vec<&str> = line.split('|').map(|s| s.trim()).collect();
            if parts.len() >= 3 {
                if let Ok(loc) = parts[1].parse::<usize>() {
                    let path = parts[2].to_string();
                    baseline.push(BaselineEntry { old_loc: loc, path });
                }
            }
        }
    }

    println!("--- MATURITY REPORT ---");
    let mut baseline_paths = std::collections::HashSet::new();

    for entry in &baseline {
        let full_path = root.join(entry.path.trim_start_matches("./"));
        baseline_paths.insert(full_path.clone());

        let new_loc = if full_path.exists() {
            fs::read_to_string(&full_path)
                .map(|c| c.lines().count())
                .unwrap_or(0)
        } else {
            0
        };

        let diff = new_loc as isize - entry.old_loc as isize;
        let percent = if entry.old_loc > 0 {
            (diff as f64 / entry.old_loc as f64) * 100.0
        } else {
            0.0
        };

        println!(
            "{}: {} -> {} ({:+.1}%)",
            entry.path, entry.old_loc, new_loc, percent
        );
    }

    let rust_files = rust_files_under(root);

    println!("\n--- LARGEST RUST FILES ---");
    for (path, loc) in largest_rust_files(root, &rust_files, LARGEST_FILE_LIMIT) {
        println!("{}: {} LOC", path.display(), loc);
    }

    println!("\n--- ARCHITECTURE GATES ---");
    let mut oversized = Vec::new();
    find_oversized_rs_files(root, root, &baseline_paths, &mut oversized);

    if !oversized.is_empty() {
        println!(
            "WARNING: Found new or unbaselined files exceeding {} LOC:",
            MAX_FILE_SIZE
        );
        for (path, loc) in oversized {
            println!("  {}: {} LOC", path.display(), loc);
        }
    } else {
        println!("OK: No new files exceed {} LOC.", MAX_FILE_SIZE);
    }

    println!("\n--- DEBT COUNTS ---");
    println!(
        "unwrap/expect allowlist entries: {}",
        unwrap_expect_allowlist_entry_count(root)
    );
    println!(
        "panic-like macro calls: {}",
        panic_like_macro_count(&rust_files)
    );
    println!("TODO markers: {}", todo_marker_count(&rust_files));
    println!(
        "unsafe items without preceding SAFETY comment: {}",
        unsafe_without_safety_count(&rust_files)
    );

    println!("\n--- SCAFFOLD CRATES ---");
    for (crate_name, status) in scaffold_crate_statuses(root) {
        println!("{crate_name}: {status}");
    }

    println!("\n--- TEST COUNT ---");
    println!("test attributes: {}", test_attribute_count(&rust_files));

    Ok(())
}

fn find_oversized_rs_files(
    root: &Path,
    dir: &Path,
    baseline: &std::collections::HashSet<PathBuf>,
    oversized: &mut Vec<(PathBuf, usize)>,
) {
    if !dir.exists() {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && should_descend_into_dir(&path) {
            find_oversized_rs_files(root, &path, baseline, oversized);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            if !baseline.contains(&path) {
                if let Ok(content) = fs::read_to_string(&path) {
                    let loc = content.lines().count();
                    if loc > MAX_FILE_SIZE {
                        let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
                        oversized.push((rel, loc));
                    }
                }
            }
        }
    }
}

fn rust_files_under(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_rust_files(root, &mut files);
    files.sort();
    files
}

fn collect_rust_files(path: &Path, files: &mut Vec<PathBuf>) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };

    if metadata.is_file() {
        if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path.to_path_buf());
        }
        return;
    }

    if !should_descend_into_dir(path) {
        return;
    }

    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        collect_rust_files(&entry.path(), files);
    }
}

fn should_descend_into_dir(path: &Path) -> bool {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    name != "target" && name != "target-test" && name != "target_tests" && name != ".git"
}

fn largest_rust_files(root: &Path, files: &[PathBuf], limit: usize) -> Vec<(PathBuf, usize)> {
    let mut counts = files
        .iter()
        .filter_map(|path| {
            let loc = fs::read_to_string(path).ok()?.lines().count();
            let rel = path.strip_prefix(root).unwrap_or(path).to_path_buf();
            Some((rel, loc))
        })
        .collect::<Vec<_>>();
    counts.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    counts.truncate(limit);
    counts
}

fn unwrap_expect_allowlist_entry_count(root: &Path) -> usize {
    let path = root.join("stutter/src/architecture_tests/allowlists.rs");
    fs::read_to_string(path)
        .map(|source| {
            let mut in_unwrap_allowlist = false;
            let mut count = 0;

            for line in source.lines() {
                if line.contains("UNWRAP_EXPECT_FILE_ALLOWLIST") {
                    in_unwrap_allowlist = true;
                    continue;
                }
                if in_unwrap_allowlist && line.trim() == "];" {
                    in_unwrap_allowlist = false;
                    continue;
                }
                if in_unwrap_allowlist && line.trim_start().starts_with("path: \"src/") {
                    count += 1;
                }
            }

            count
        })
        .unwrap_or(0)
}

fn panic_like_macro_count(files: &[PathBuf]) -> usize {
    count_lines_matching(files, |line| {
        line.contains("panic!(") || line.contains("unreachable!(") || line.contains("todo!(")
    })
}

fn todo_marker_count(files: &[PathBuf]) -> usize {
    count_lines_matching(files, |line| line.contains("TODO"))
}

fn test_attribute_count(files: &[PathBuf]) -> usize {
    count_lines_matching(files, |line| {
        let line = line.trim();
        line == "#[test]" || line.starts_with("#[tokio::test")
    })
}

fn unsafe_without_safety_count(files: &[PathBuf]) -> usize {
    let mut count = 0;

    for file in files {
        if file
            .components()
            .any(|component| component.as_os_str().to_string_lossy().as_ref() == "stutter-ebpf")
        {
            continue;
        }

        let Ok(source) = fs::read_to_string(file) else {
            continue;
        };
        let lines = source.lines().collect::<Vec<_>>();
        for (index, line) in lines.iter().enumerate() {
            if !line_contains_unsafe_keyword(line) {
                continue;
            }
            if !has_immediate_safety_comment(&lines, index) {
                count += 1;
            }
        }
    }

    count
}

fn has_immediate_safety_comment(lines: &[&str], unsafe_line_index: usize) -> bool {
    let Some(mut cursor) = unsafe_line_index.checked_sub(1) else {
        return false;
    };

    loop {
        let trimmed = lines[cursor].trim_start();
        if trimmed.starts_with("#[") {
            let Some(previous) = cursor.checked_sub(1) else {
                return false;
            };
            cursor = previous;
            continue;
        }
        if !trimmed.starts_with("//") {
            return false;
        }
        if trimmed.contains("SAFETY:") {
            return true;
        }
        let Some(previous) = cursor.checked_sub(1) else {
            return false;
        };
        cursor = previous;
    }
}

fn line_contains_unsafe_keyword(line: &str) -> bool {
    let sanitized = strip_strings_and_line_comments(line);
    let mut start = 0;
    while let Some(offset) = sanitized[start..].find("unsafe") {
        let index = start + offset;
        let before = sanitized[..index].chars().next_back();
        let after = sanitized[index + "unsafe".len()..].chars().next();
        if !is_ident_char(before)
            && !is_ident_char(after)
            && starts_unsafe_code_marker(&sanitized[index + "unsafe".len()..])
        {
            return true;
        }
        start = index + "unsafe".len();
    }
    false
}

fn starts_unsafe_code_marker(after_unsafe: &str) -> bool {
    let trimmed = after_unsafe.trim_start();
    trimmed.starts_with('{')
        || starts_keyword(trimmed, "fn")
        || starts_keyword(trimmed, "impl")
        || starts_keyword(trimmed, "trait")
        || starts_keyword(trimmed, "extern")
}

fn starts_keyword(source: &str, keyword: &str) -> bool {
    source.starts_with(keyword) && !is_ident_char(source[keyword.len()..].chars().next())
}

fn is_ident_char(ch: Option<char>) -> bool {
    ch.is_some_and(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn strip_strings_and_line_comments(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    let mut in_string = false;
    let mut in_char = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if !in_string && !in_char && ch == '/' && chars.peek() == Some(&'/') {
            break;
        }

        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            out.push(' ');
            continue;
        }

        if in_char {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '\'' {
                in_char = false;
            }
            out.push(' ');
            continue;
        }

        if ch == 'r' {
            let mut probe = chars.clone();
            let mut hashes = 0;
            while probe.peek() == Some(&'#') {
                probe.next();
                hashes += 1;
            }
            if probe.peek() == Some(&'"') {
                for _ in 0..hashes {
                    chars.next();
                    out.push(' ');
                }
                chars.next();
                out.push(' ');

                while let Some(raw_ch) = chars.next() {
                    out.push(' ');
                    if raw_ch != '"' {
                        continue;
                    }

                    let mut matched_hashes = 0;
                    while matched_hashes < hashes && chars.peek() == Some(&'#') {
                        chars.next();
                        out.push(' ');
                        matched_hashes += 1;
                    }
                    if matched_hashes == hashes {
                        break;
                    }
                }
                continue;
            }
        }

        match ch {
            '"' => {
                in_string = true;
                out.push(' ');
            }
            '\'' => {
                in_char = true;
                out.push(' ');
            }
            _ => out.push(ch),
        }
    }

    out
}

fn count_lines_matching(files: &[PathBuf], predicate: impl Fn(&str) -> bool) -> usize {
    files
        .iter()
        .filter_map(|path| fs::read_to_string(path).ok())
        .flat_map(|source| source.lines().map(str::to_owned).collect::<Vec<_>>())
        .filter(|line| predicate(line))
        .count()
}

fn scaffold_crate_statuses(root: &Path) -> Vec<(&'static str, &'static str)> {
    [
        ("stutter-report", root.join("stutter-report/src")),
        ("stutter-config", root.join("stutter-config/src")),
    ]
    .into_iter()
    .map(|(name, src)| {
        let status = if rust_files_under(&src).iter().any(|path| {
            fs::read_to_string(path).is_ok_and(|source| {
                source.contains("placeholder") || source.contains("future migration")
            })
        }) {
            "contains scaffold markers"
        } else {
            "no scaffold markers"
        };
        (name, status)
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use super::*;

    fn unique_temp_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "stutter-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        path
    }

    #[test]
    fn maturity_report_reads_archived_cleanup_baseline() {
        let root = unique_temp_dir("maturity-report");
        let baseline_dir = root.join("docs/internal/archive");
        fs::create_dir_all(&baseline_dir).unwrap();

        fs::write(
            baseline_dir.join("cleanup-baseline.md"),
            "\
# Cleanup Baseline

| LOC  | File Path | Target Phase / Split Plan |
|------|-----------|---------------------------|
| 1 | ./src/lib.rs | |
",
        )
        .unwrap();

        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "fn main() {}\n").unwrap();

        let result = run_maturity_report(&root);

        fs::remove_dir_all(&root).unwrap();

        assert!(result.is_ok());
    }

    #[test]
    fn missing_maturity_report_baseline_error_names_path() {
        let root = unique_temp_dir("maturity-report-missing");
        fs::create_dir_all(&root).unwrap();

        let err = run_maturity_report(&root).unwrap_err();

        fs::remove_dir_all(&root).unwrap();

        let message = format!("{err:#}");
        assert!(message.contains(CLEANUP_BASELINE_PATH), "{message}");
    }
}
