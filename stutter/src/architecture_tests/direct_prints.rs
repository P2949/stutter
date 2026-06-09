//! Direct print architecture guard tests.

use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{
    allowlists::{
        EXISTING_DIRECT_PRINT_ALLOWLIST, allowed_direct_prints_summary,
        allowlisted_direct_print_call,
    },
    crate_src_root, relative_to_crate_root,
    scanners::{production_code_lines_outside_cfg_test_modules, rust_files_under},
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct DirectPrintMacroCall {
    path: String,
    line_number: usize,
    macro_name: &'static str,
    line: String,
}

fn direct_print_forbidden_files() -> Vec<PathBuf> {
    let root = crate_src_root();
    let mut files = vec![
        root.join("autotune/mod.rs"),
        root.join("autotune/planner/mod.rs"),
        root.join("report/analysis.rs"),
        root.join("process_tree.rs"),
    ];

    files.extend(
        rust_files_under(&root.join("agent"))
            .into_iter()
            .filter(|path| {
                !path
                    .components()
                    .any(|component| component.as_os_str().to_str() == Some("tests"))
            }),
    );
    files.extend(rust_files_under(&root.join("actions")));
    files.extend(
        rust_files_under(&root.join("autotune/runtime"))
            .into_iter()
            .filter(|path| path.file_name().and_then(|name| name.to_str()) != Some("stream.rs")),
    );
    files.extend(rust_files_under(&root.join("daemon")));
    files.extend(rust_files_under(&root.join("focus")));

    files.sort();
    files.dedup();
    files
}

fn direct_print_macro_calls_in_file(path: &Path) -> Vec<DirectPrintMacroCall> {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    let relative_path = relative_to_crate_root(path);
    direct_print_macro_calls_in_source(&source, &relative_path)
}

fn direct_print_macro_calls_in_source(source: &str, path: &str) -> Vec<DirectPrintMacroCall> {
    let mut calls = Vec::new();

    for (line_number, line) in production_code_lines_outside_cfg_test_modules(source) {
        if line.trim_start().starts_with("//") {
            continue;
        }

        if line.contains("eprintln!") {
            calls.push(DirectPrintMacroCall {
                path: path.to_owned(),
                line_number,
                macro_name: "eprintln!",
                line: line.trim().to_owned(),
            });
        }

        if line.contains("println!") {
            let mut start = 0;
            let mut found_non_eprintln = false;
            while let Some(idx) = line[start..].find("println!") {
                let absolute_idx = start + idx;
                if absolute_idx == 0 || line.as_bytes()[absolute_idx - 1] != b'e' {
                    found_non_eprintln = true;
                    break;
                }
                start = absolute_idx + "println!".len();
            }
            if found_non_eprintln {
                calls.push(DirectPrintMacroCall {
                    path: path.to_owned(),
                    line_number,
                    macro_name: "println!",
                    line: line.trim().to_owned(),
                });
            }
        }
    }

    calls
}

#[test]
fn direct_print_scanner_finds_print_macros_and_ignores_cfg_test_modules() {
    let source = r#"
fn runtime_stdout() {
    println!("runtime should not print directly");
}

fn runtime_stderr() {
    eprintln!("runtime should not print directly");
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_prints_are_ignored() {
        println!("test output is allowed");
        eprintln!("test output is allowed");
    }
}
"#;

    let calls = direct_print_macro_calls_in_source(source, "src/domain.rs");

    assert_eq!(
        calls,
        vec![
            DirectPrintMacroCall {
                path: "src/domain.rs".to_owned(),
                line_number: 3,
                macro_name: "println!",
                line: "println!(\"runtime should not print directly\");".to_owned(),
            },
            DirectPrintMacroCall {
                path: "src/domain.rs".to_owned(),
                line_number: 7,
                macro_name: "eprintln!",
                line: "eprintln!(\"runtime should not print directly\");".to_owned(),
            },
        ]
    );
}

#[test]
fn runtime_domain_and_service_modules_do_not_print_directly() {
    for allowance in EXISTING_DIRECT_PRINT_ALLOWLIST {
        assert!(
            !allowance.reason.trim().is_empty(),
            "direct print allowlist entry '{}:{} {}' must have a reason",
            allowance.path,
            allowance.line_number,
            allowance.macro_name
        );
    }

    let files = direct_print_forbidden_files();
    let calls = files
        .iter()
        .flat_map(|file| direct_print_macro_calls_in_file(file))
        .collect::<Vec<_>>();

    let mut violations = Vec::new();

    for allowance in EXISTING_DIRECT_PRINT_ALLOWLIST {
        if !calls.iter().any(|call| {
            call.path == allowance.path
                && call.line_number == allowance.line_number
                && call.macro_name == allowance.macro_name
        }) {
            violations.push(format!(
                "allowlisted direct print no longer exists or moved: {}:{} {} -- update EXISTING_DIRECT_PRINT_ALLOWLIST",
                allowance.path, allowance.line_number, allowance.macro_name
            ));
        }
    }

    for call in &calls {
        if allowlisted_direct_print_call(&call.path, call.line_number, call.macro_name).is_none() {
            violations.push(format!(
                "{}:{} uses {} outside approved CLI/rendering/test boundaries: {}",
                call.path, call.line_number, call.macro_name, call.line
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "direct printing architecture guard failed:\n{}\n\nallowed existing direct prints:\n{}",
        violations.join("\n"),
        allowed_direct_prints_summary()
    );
}
