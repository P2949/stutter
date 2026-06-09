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

fn is_action_framework_path(relative: &str) -> bool {
    const EXACT_PATHS: &[&str] = &[
        "stutter/src/actions/transaction.rs",
        "stutter/src/actions/traits.rs",
        "stutter/src/actions/rollback.rs",
        "stutter/src/actions/token.rs",
        "stutter/src/actions/factory.rs",
        "stutter/src/actions/syscalls.rs",
        "stutter/src/actions/restore_write.rs",
        "stutter/src/actions/restore_identity.rs",
        "stutter/src/actions/model.rs",
        "stutter/src/actions/mod.rs",
    ];

    const DIRECTORY_PREFIXES: &[&str] = &[
        "stutter/src/actions/runner/",
        "stutter/src/actions/rollback/",
        "stutter/src/actions/error/",
    ];

    EXACT_PATHS.contains(&relative)
        || DIRECTORY_PREFIXES
            .iter()
            .any(|prefix| relative.starts_with(prefix))
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

        // Core framework files are not action boundary modules.
        if is_action_framework_path(&relative) {
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

#[cfg(test)]
mod tests {
    use super::is_action_framework_path;

    #[test]
    fn action_error_framework_path_excludes_old_flat_rollback_file() {
        assert!(is_action_framework_path("stutter/src/actions/rollback.rs"));
    }

    #[test]
    fn action_error_framework_path_excludes_rollback_directory() {
        assert!(is_action_framework_path(
            "stutter/src/actions/rollback/mod.rs"
        ));
        assert!(is_action_framework_path(
            "stutter/src/actions/rollback/tests.rs"
        ));
    }

    #[test]
    fn action_error_framework_path_does_not_overmatch_rollback_prefix() {
        assert!(!is_action_framework_path(
            "stutter/src/actions/rollback_extra.rs"
        ));
    }

    #[test]
    fn action_error_framework_path_does_not_overmatch_flat_framework_prefixes() {
        assert!(!is_action_framework_path(
            "stutter/src/actions/model_extra.rs"
        ));
        assert!(!is_action_framework_path(
            "stutter/src/actions/transaction_extra.rs"
        ));
        assert!(!is_action_framework_path(
            "stutter/src/actions/token_extra.rs"
        ));
    }
}
