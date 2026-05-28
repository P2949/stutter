use crate::architecture_tests::{
    relative_to_workspace_root,
    scanners::{production_code_lines_outside_cfg_test_modules, rust_files_under},
};

fn is_test_action_path(path: &str) -> bool {
    path.ends_with("/tests.rs")
        || path.contains("/tests/")
        || path.contains("/runner_tests/")
        || path.ends_with("/fake_action.rs")
}

#[test]
fn production_actions_use_typed_boundary_errors_instead_of_string_coded_bail() {
    let actions_root = crate::architecture_tests::workspace_root()
        .join("stutter")
        .join("src")
        .join("actions");

    let mut violations = Vec::new();

    for file in rust_files_under(&actions_root) {
        let relative = relative_to_workspace_root(&file);

        // Core framework files are not boundary layers
        if relative.starts_with("stutter/src/actions/runner") ||
           relative.starts_with("stutter/src/actions/transaction.rs") ||
           relative.starts_with("stutter/src/actions/traits.rs") ||
           relative.starts_with("stutter/src/actions/rollback.rs") ||
           relative.starts_with("stutter/src/actions/token.rs") ||
           relative.starts_with("stutter/src/actions/factory.rs") ||
           relative.starts_with("stutter/src/actions/syscalls.rs") ||
           relative.starts_with("stutter/src/actions/restore_write.rs") ||
           relative.starts_with("stutter/src/actions/restore_identity.rs") ||
           relative.starts_with("stutter/src/actions/model.rs") ||
           relative.starts_with("stutter/src/actions/mod.rs") ||
           relative.starts_with("stutter/src/actions/error")
        {
            continue;
        }

        if is_test_action_path(&relative) {
            continue;
        }

        let source = std::fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display()));

        for (line_number, line) in production_code_lines_outside_cfg_test_modules(&source) {
            if line.contains("anyhow::bail!") && !line.trim().starts_with("//") {
                violations.push(format!(
                    "{}:{}: use ActionBoundaryError or another typed action error instead of string-coded anyhow::bail!",
                    relative,
                    line_number
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "production action modules still have string-coded bail paths:\n{}",
        violations.join("\n")
    );
}
