//! Production panic-path architecture guard tests.

use std::{fs, path::Path};

use super::{
    allowlists::{EXISTING_PRODUCTION_PANIC_ALLOWLIST, allowlisted_production_panic_call},
    crate_src_root, relative_to_crate_root,
    scanners::{production_code_lines_outside_cfg_test_modules, rust_files_under},
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProductionPanicCall {
    path: String,
    line_number: usize,
    macro_name: &'static str,
    line: String,
}

fn production_panic_calls_in_file(path: &Path) -> Vec<ProductionPanicCall> {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    let relative_path = relative_to_crate_root(path);
    production_panic_calls_in_source(&source, &relative_path)
}

fn production_panic_calls_in_source(source: &str, path: &str) -> Vec<ProductionPanicCall> {
    if is_test_source_path(path) {
        return Vec::new();
    }

    let lines = source.lines().collect::<Vec<_>>();
    let mut calls = Vec::new();

    for (line_number, line) in production_code_lines_outside_cfg_test_modules(source) {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }

        let preceding_line_has_invariant = line_number
            .checked_sub(2)
            .and_then(|index| lines.get(index))
            .is_some_and(|line| line.contains("// invariant:"));

        for macro_name in ["panic!", "unreachable!", "todo!"] {
            if !line.contains(macro_name) {
                continue;
            }
            if macro_name == "unreachable!" && preceding_line_has_invariant {
                continue;
            }
            calls.push(ProductionPanicCall {
                path: path.to_owned(),
                line_number,
                macro_name,
                line: line.trim().to_owned(),
            });
        }
    }

    calls
}

fn is_test_source_path(path: &str) -> bool {
    path.ends_with("_tests.rs")
        || path.ends_with("/tests.rs")
        || path.contains("/tests/")
        || path.split('/').any(|segment| segment.ends_with("_tests"))
}

#[test]
fn production_panic_scanner_ignores_tests_and_documented_unreachable_invariants() {
    let source = r#"
fn bad_panic() {
    panic!("nope");
}

fn bad_unreachable() {
    unreachable!();
}

fn documented_unreachable() {
    // invariant: enum has already been checked
    unreachable!();
}

fn bad_todo() {
    todo!("finish me");
}

#[cfg(test)]
fn cfg_test_function_is_ignored() {
    panic!("test helper");
}

#[cfg(test)]
mod tests {
    fn test_panic_is_ok() {
        panic!("test assertion");
    }
}
"#;

    let calls = production_panic_calls_in_source(source, "src/new_module.rs");

    assert_eq!(
        calls,
        vec![
            ProductionPanicCall {
                path: "src/new_module.rs".to_owned(),
                line_number: 3,
                macro_name: "panic!",
                line: "panic!(\"nope\");".to_owned(),
            },
            ProductionPanicCall {
                path: "src/new_module.rs".to_owned(),
                line_number: 7,
                macro_name: "unreachable!",
                line: "unreachable!();".to_owned(),
            },
            ProductionPanicCall {
                path: "src/new_module.rs".to_owned(),
                line_number: 16,
                macro_name: "todo!",
                line: "todo!(\"finish me\");".to_owned(),
            },
        ]
    );

    assert!(
        production_panic_calls_in_source(source, "src/agent/tests/security.rs").is_empty(),
        "child test modules under tests/ must not be scanned as production code"
    );
    assert!(
        production_panic_calls_in_source(
            "#![cfg(test)]\nfn helper() { panic!(\"ok\"); }\n",
            "src/test_support.rs"
        )
        .is_empty(),
        "source files gated behind #![cfg(test)] must not be scanned as production code"
    );
}

#[test]
fn new_production_panic_paths_require_allowlist_or_invariant() {
    for allowance in EXISTING_PRODUCTION_PANIC_ALLOWLIST {
        assert!(
            !allowance.reason.trim().is_empty(),
            "production panic allowlist entry '{}:{} {}' must have a reason",
            allowance.path,
            allowance.line_number,
            allowance.macro_name
        );
    }

    let mut violations = Vec::new();

    for file in rust_files_under(&crate_src_root()) {
        for call in production_panic_calls_in_file(&file) {
            if allowlisted_production_panic_call(&call.path, call.line_number, call.macro_name)
                .is_some()
            {
                continue;
            }

            violations.push(format!(
                "{}:{} uses {} in production code without a temporary panic allowlist entry: {}",
                call.path, call.line_number, call.macro_name, call.line
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "production panic guard failed:\n{}",
        violations.join("\n")
    );
}
