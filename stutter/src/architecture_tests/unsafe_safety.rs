//! Unsafe-code documentation architecture guard tests.

use std::{fs, path::PathBuf};

use super::relative_to_crate_root;

#[derive(Clone, Debug, Eq, PartialEq)]
struct UnsafeSite {
    path: String,
    line_number: usize,
    line: String,
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("stutter crate has a workspace parent")
        .to_path_buf()
}

fn scanned_roots() -> Vec<PathBuf> {
    let root = workspace_root();
    [
        "stutter/src",
        "stutter/build.rs",
        "stutter-common/src",
        "stutter-config/src",
        "stutter-core/src",
        "stutter-report/src",
        "xtask/src",
    ]
    .into_iter()
    .map(|path| root.join(path))
    .collect()
}

fn collect_rust_files(path: PathBuf, files: &mut Vec<PathBuf>) {
    let Ok(metadata) = fs::metadata(&path) else {
        return;
    };

    if metadata.is_file() {
        if path.extension().is_some_and(|ext| ext == "rs") {
            files.push(path);
        }
        return;
    }

    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        collect_rust_files(entry.path(), files);
    }
}

fn unsafe_sites_without_safety_comment(path: &std::path::Path) -> Vec<UnsafeSite> {
    unsafe_sites(path)
        .into_iter()
        .filter(|site| {
            let source = fs::read_to_string(path)
                .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
            let lines = source.lines().collect::<Vec<_>>();
            !has_immediate_safety_comment(&lines, site.line_number - 1)
        })
        .collect()
}

fn unsafe_sites(path: &std::path::Path) -> Vec<UnsafeSite> {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    let display_path = relative_to_workspace(path);

    source
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            line_contains_unsafe_keyword(line).then_some(UnsafeSite {
                path: display_path.clone(),
                line_number: index + 1,
                line: line.trim().to_owned(),
            })
        })
        .collect()
}

fn relative_to_workspace(path: &std::path::Path) -> String {
    let root = workspace_root();
    path.strip_prefix(&root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn has_immediate_safety_comment(lines: &[&str], unsafe_line_index: usize) -> bool {
    let Some(mut cursor) = unsafe_line_index.checked_sub(1) else {
        return false;
    };

    let mut saw_comment = false;
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
            return saw_comment && trimmed.contains("SAFETY:");
        }
        saw_comment = true;
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

fn is_unsafe_wrapper_path(path: &str) -> bool {
    path == "stutter/src/syscall.rs"
        || path.ends_with("/syscall.rs")
        || path.ends_with("/syscalls.rs")
        || path.ends_with("/ffi.rs")
        || path.ends_with("/memfd.rs")
        || path.ends_with("/decode.rs")
}

fn is_test_or_architecture_unsafe_path(path: &str) -> bool {
    path.contains("/tests/")
        || path.ends_with("_tests.rs")
        || path.ends_with("/tests.rs")
        || path.contains("/architecture_tests/")
        || path == "stutter/src/architecture_tests.rs"
        || path == "stutter/src/community_rules/paths.rs"
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

#[test]
fn unsafe_scanner_requires_local_safety_comment_and_ignores_strings() {
    let source = [
        r#"let marker = "unsafe { not code }";"#,
        "fn documented() {",
        "    // SAFETY: pointer is valid in this fixture.",
        "    let _ = unsafe { 1 };",
        "}",
        "fn undocumented() {",
        "    let _ = unsafe { 2 };",
        "}",
    ]
    .join("\n");
    let path = workspace_root().join("stutter/src/architecture_tests/unsafe_fixture.rs");
    let lines = source.lines().collect::<Vec<_>>();

    let findings = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            line_contains_unsafe_keyword(line)
                .then_some((index, line))
                .filter(|(index, _)| !has_immediate_safety_comment(&lines, *index))
        })
        .map(|(index, line)| UnsafeSite {
            path: relative_to_crate_root(&path),
            line_number: index + 1,
            line: line.trim().to_owned(),
        })
        .collect::<Vec<_>>();

    assert_eq!(
        findings,
        vec![UnsafeSite {
            path: "src/architecture_tests/unsafe_fixture.rs".to_owned(),
            line_number: 7,
            line: "let _ = unsafe { 2 };".to_owned(),
        }]
    );
}

#[test]
fn all_non_ebpf_unsafe_has_local_safety_documentation() {
    let mut files = Vec::new();
    for root in scanned_roots() {
        collect_rust_files(root, &mut files);
    }
    files.sort();

    let violations = files
        .iter()
        .flat_map(|p| unsafe_sites_without_safety_comment(p))
        .map(|site| {
            format!(
                "{}:{} unsafe code lacks an immediately preceding SAFETY comment: {}",
                site.path, site.line_number, site.line
            )
        })
        .collect::<Vec<_>>();

    assert!(
        violations.is_empty(),
        "unsafe safety documentation guard failed:\n{}",
        violations.join("\n")
    );
}

#[test]
fn production_unsafe_stays_inside_wrapper_modules() {
    let mut files = Vec::new();
    collect_rust_files(workspace_root().join("stutter/src"), &mut files);
    files.sort();

    let violations = files
        .iter()
        .flat_map(|path| unsafe_sites(path))
        .filter(|site| {
            !is_unsafe_wrapper_path(&site.path) && !is_test_or_architecture_unsafe_path(&site.path)
        })
        .map(|site| {
            format!(
                "{}:{} unsafe code must live in a syscall/ffi/memfd/decode wrapper module: {}",
                site.path, site.line_number, site.line
            )
        })
        .collect::<Vec<_>>();

    assert!(
        violations.is_empty(),
        "unsafe wrapper boundary guard failed:\n{}",
        violations.join("\n")
    );
}
