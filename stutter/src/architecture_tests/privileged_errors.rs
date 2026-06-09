use crate::architecture_tests::{relative_to_workspace_root, scanners::rust_files_under};

#[test]
fn privileged_boundary_uses_typed_errors_instead_of_string_coded_bail() {
    let privilege_roots = [
        crate::architecture_tests::workspace_root()
            .join("stutter")
            .join("src")
            .join("daemon")
            .join("privilege.rs"),
        crate::architecture_tests::workspace_root()
            .join("stutter")
            .join("src")
            .join("daemon")
            .join("privilege"),
    ];

    let mut files = Vec::new();
    for root in privilege_roots {
        if root.is_file() {
            files.push(root);
        } else if root.is_dir() {
            files.extend(rust_files_under(&root));
        }
    }

    let mut violations = Vec::new();

    for file in files {
        let relative = relative_to_workspace_root(&file);
        let content = std::fs::read_to_string(&file)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display()));

        for (line_idx, line) in content.lines().enumerate() {
            if line.contains("anyhow::bail!") {
                violations.push(format!(
                    "{}:{}: use PrivilegedWorkerError instead of string-coded anyhow::bail!",
                    relative,
                    line_idx + 1
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "privileged boundary has string-coded bail paths:\n{}",
        violations.join("\n")
    );
}
