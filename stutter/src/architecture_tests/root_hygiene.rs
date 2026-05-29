use std::{fs, path::Path};

const FORBIDDEN_ROOT_FILE_NAMES: &[&str] =
    &["plan.md", "migrate.py", "scratch.py", "generate_input.rs"];

const FORBIDDEN_ROOT_FILE_PREFIXES: &[&str] = &["fix_", "split_", "refactor_"];

const FORBIDDEN_ROOT_FILE_SUFFIXES: &[&str] = &[".tmp", ".bak", ".orig", ".rej"];

fn root_hygiene_violation(name: &str) -> bool {
    FORBIDDEN_ROOT_FILE_NAMES.contains(&name)
        || FORBIDDEN_ROOT_FILE_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
        || FORBIDDEN_ROOT_FILE_SUFFIXES
            .iter()
            .any(|suffix| name.ends_with(suffix))
}

#[test]
fn repository_root_has_no_one_off_migration_or_scratch_artifacts() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("stutter crate should live under workspace root");

    let mut violations = Vec::new();

    for entry in fs::read_dir(workspace_root).expect("read workspace root") {
        let entry = entry.expect("read workspace root entry");
        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();

        if root_hygiene_violation(&name) {
            violations.push(name);
        }
    }

    violations.sort();

    assert!(
        violations.is_empty(),
        "repository root contains one-off migration/scratch artifacts:\n{}",
        violations.join("\n")
    );
}

#[cfg(test)]
mod tests {
    use super::root_hygiene_violation;

    #[test]
    fn root_hygiene_rejects_known_one_off_names() {
        for name in ["plan.md", "migrate.py", "scratch.py", "generate_input.rs"] {
            assert!(
                root_hygiene_violation(name),
                "{name} should be rejected at repository root"
            );
        }
    }

    #[test]
    fn root_hygiene_rejects_known_one_off_prefixes() {
        for name in [
            "fix_tests.py",
            "fix_codegen.py",
            "split_report_tests.py",
            "split_big_file.py",
            "refactor_actions.py",
            "refactor_report.rs",
        ] {
            assert!(
                root_hygiene_violation(name),
                "{name} should be rejected at repository root"
            );
        }
    }

    #[test]
    fn root_hygiene_rejects_common_temp_suffixes() {
        for name in [
            "README.md.bak",
            "Cargo.toml.orig",
            "patch.tmp",
            "fix.patch.rej",
        ] {
            assert!(
                root_hygiene_violation(name),
                "{name} should be rejected at repository root"
            );
        }
    }

    #[test]
    fn root_hygiene_allows_normal_project_root_files() {
        for name in [
            "Cargo.toml",
            "Cargo.lock",
            "README.md",
            "CONTRIBUTING.md",
            "LICENSE-MIT",
            "LICENSE-APACHE",
            "LICENSE-GPL2",
            "deny.toml",
            "rust-toolchain.toml",
            "rustfmt.toml",
        ] {
            assert!(
                !root_hygiene_violation(name),
                "{name} should be allowed at repository root"
            );
        }
    }
}
