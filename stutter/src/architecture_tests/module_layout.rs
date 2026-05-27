use std::{fs, path::Path};

fn module_layout_rust_files() -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();

    for src_root in crate::architecture_tests::workspace_src_roots() {
        files.extend(crate::architecture_tests::scanners::rust_files_under(
            &src_root,
        ));
    }

    files
}

fn module_layout_relative_path(path: &Path) -> String {
    crate::architecture_tests::relative_to_workspace_root(path)
}

#[test]
fn module_pairs_must_not_duplicate_names() {
    let rust_files = module_layout_rust_files();

    let mut violations = Vec::new();

    // Define explicitly allowlisted module pairs, if any. Entries must be
    // workspace-relative paths, for example:
    // - stutter/src/foo.rs
    // - stutter-report/src/foo.rs
    let allowlist: &[&str] = &[];

    for file in &rust_files {
        if let Some(file_name) = file.file_name().and_then(|name| name.to_str())
            && file_name.ends_with(".rs")
            && file_name != "mod.rs"
            && file_name != "lib.rs"
            && file_name != "main.rs"
        {
            let stem = file
                .file_stem()
                .and_then(|stem| stem.to_str())
                .expect("Rust source files should have UTF-8 stems");

            let dir_mod = file.with_file_name(stem).join("mod.rs");

            if dir_mod.exists() {
                let relative_file = module_layout_relative_path(file);

                if !allowlist.contains(&relative_file.as_str()) {
                    violations.push(format!(
                        "{} and {}",
                        relative_file,
                        module_layout_relative_path(&dir_mod)
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "Found stale file/mod.rs pairs (delete the stale .rs file):\n{}",
        violations.join("\n")
    );
}

#[test]
fn path_attribute_targets_must_exist() {
    let rust_files = module_layout_rust_files();

    let mut violations = Vec::new();

    for file in &rust_files {
        let content = fs::read_to_string(file)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", file.display()));

        for (line_idx, line) in content.lines().enumerate() {
            let line = line.trim();

            if line.starts_with("#[path = ")
                && let Some(start) = line.find('"')
                && let Some(end) = line[start + 1..].find('"')
            {
                let path_str = &line[start + 1..start + 1 + end];
                let target = file
                    .parent()
                    .expect("Rust source file should have a parent")
                    .join(path_str);

                if !target.exists() {
                    violations.push(format!(
                        "{}:{}: #[path = \"{}\"] does not exist",
                        module_layout_relative_path(file),
                        line_idx + 1,
                        path_str
                    ));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "Found broken #[path] targets:\n{}",
        violations.join("\n")
    );
}

#[cfg(test)]
mod tests {
    use super::module_layout_rust_files;

    #[test]
    fn module_layout_scan_covers_workspace_crates() {
        let mut files: Vec<String> = module_layout_rust_files()
            .into_iter()
            .map(|file| crate::architecture_tests::relative_to_workspace_root(&file))
            .collect();

        files.sort();

        assert!(
            files.iter().any(|path| path == "stutter/src/lib.rs"),
            "module-layout scan should include main stutter crate"
        );
        assert!(
            files.iter().any(|path| path == "stutter-ebpf/src/main.rs"),
            "module-layout scan should include stutter-ebpf crate"
        );
        assert!(
            files.iter().any(|path| path == "stutter-common/src/lib.rs"),
            "module-layout scan should include stutter-common crate"
        );
        assert!(
            files.iter().any(|path| path == "stutter-config/src/lib.rs"),
            "module-layout scan should include stutter-config crate"
        );
        assert!(
            files.iter().any(|path| path == "stutter-core/src/lib.rs"),
            "module-layout scan should include stutter-core crate"
        );
        assert!(
            files.iter().any(|path| path == "stutter-report/src/lib.rs"),
            "module-layout scan should include stutter-report crate"
        );
        assert!(
            files.iter().any(|path| path == "xtask/src/main.rs"),
            "module-layout scan should include xtask crate"
        );
    }
}
