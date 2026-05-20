//! Production unwrap/expect architecture guard tests.

use std::{fs, path::Path};

use super::{
    allowlists::{
        EXISTING_PRODUCTION_UNWRAP_EXPECT_FILE_ALLOWLIST,
        allowlisted_existing_production_unwrap_expect_file,
    },
    crate_src_root, relative_to_crate_root,
    scanners::{production_code_lines_outside_cfg_test_modules, rust_files_under},
};

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProductionUnwrapExpectCall {
    path: String,
    line_number: usize,
    call: &'static str,
    line: String,
}

fn production_unwrap_expect_calls_in_file(path: &Path) -> Vec<ProductionUnwrapExpectCall> {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
    let relative_path = relative_to_crate_root(path);
    production_unwrap_expect_calls_in_source(&source, &relative_path)
}

fn production_unwrap_expect_calls_in_source(
    source: &str,
    path: &str,
) -> Vec<ProductionUnwrapExpectCall> {
    if is_test_source_path(path) {
        return Vec::new();
    }

    let lines = source.lines().collect::<Vec<_>>();
    let mut calls = Vec::new();

    for (line_number, line) in production_code_lines_outside_cfg_test_modules(source) {
        let preceding_line_has_invariant = line_number
            .checked_sub(2)
            .and_then(|index| lines.get(index))
            .is_some_and(|line| line.contains("// invariant:"));

        for call in [".unwrap()", ".expect("] {
            if line.contains(call) && !preceding_line_has_invariant {
                calls.push(ProductionUnwrapExpectCall {
                    path: path.to_owned(),
                    line_number,
                    call,
                    line: line.trim().to_owned(),
                });
            }
        }
    }

    calls
}

fn is_test_source_path(path: &str) -> bool {
    path.ends_with("_tests.rs") || path.ends_with("/tests.rs") || path.contains("/tests/")
}

#[test]
fn production_unwrap_expect_scanner_ignores_cfg_test_modules_and_invariant_comments() {
    let source = r#"
fn bad_unwrap(value: Option<u8>) -> u8 {
    value.unwrap()
}

fn bad_expect(value: Option<u8>) -> u8 {
    value.expect("value must exist")
}

fn documented_unwrap(value: Option<u8>) -> u8 {
    // invariant: value was checked by the caller
    value.unwrap()
}

fn documented_expect(value: Option<u8>) -> u8 {
    // invariant: value was checked by the caller
    value.expect("value must exist")
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_unwraps_are_ignored() {
        Some(1).unwrap();
        Some(1).expect("present");
    }
}
"#;

    let calls = production_unwrap_expect_calls_in_source(source, "src/new_module.rs");

    assert_eq!(
        calls,
        vec![
            ProductionUnwrapExpectCall {
                path: "src/new_module.rs".to_owned(),
                line_number: 3,
                call: ".unwrap()",
                line: "value.unwrap()".to_owned(),
            },
            ProductionUnwrapExpectCall {
                path: "src/new_module.rs".to_owned(),
                line_number: 7,
                call: ".expect(",
                line: "value.expect(\"value must exist\")".to_owned(),
            },
        ]
    );

    assert!(
        production_unwrap_expect_calls_in_source(source, "src/process/tests/scanner.rs").is_empty(),
        "child test modules under tests/ must not be scanned as production code"
    );
}

#[test]
fn new_production_unwrap_expect_calls_require_invariant_or_allowlist() {
    for allowance in EXISTING_PRODUCTION_UNWRAP_EXPECT_FILE_ALLOWLIST {
        assert!(
            !allowance.reason.trim().is_empty(),
            "existing production unwrap/expect allowlist entry '{}' must have a reason",
            allowance.path
        );
    }

    let mut violations = Vec::new();

    for file in rust_files_under(&crate_src_root()) {
        let relative_path = relative_to_crate_root(&file);
        if allowlisted_existing_production_unwrap_expect_file(&relative_path).is_some() {
            continue;
        }

        for call in production_unwrap_expect_calls_in_file(&file) {
            violations.push(format!(
                "{}:{} uses {} without preceding '// invariant:' comment: {}",
                call.path, call.line_number, call.call, call.line
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "production unwrap/expect guard failed:\n{}",
        violations.join("\n")
    );
}
