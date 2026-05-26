use std::fs;

#[test]
fn module_pairs_must_not_duplicate_names() {
    let src_root = crate::architecture_tests::crate_src_root();
    let rust_files = crate::architecture_tests::scanners::rust_files_under(&src_root);

    let mut violations = Vec::new();

    // Define explicitly allowlisted module pairs, if any.
    let allowlist: &[&str] = &[];

    for file in &rust_files {
        if let Some(file_name) = file.file_name().and_then(|n| n.to_str())
            && file_name.ends_with(".rs")
            && file_name != "mod.rs"
            && file_name != "lib.rs"
            && file_name != "main.rs"
        {
            let stem = file.file_stem().unwrap().to_str().unwrap();
            let dir_mod = file.with_file_name(stem).join("mod.rs");
            if dir_mod.exists() {
                let relative_file = file
                    .strip_prefix(src_root.parent().unwrap())
                    .unwrap_or(file)
                    .to_str()
                    .unwrap();
                if !allowlist.contains(&relative_file) {
                    violations.push(format!("{} and {}", file.display(), dir_mod.display()));
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
    let src_root = crate::architecture_tests::crate_src_root();
    let rust_files = crate::architecture_tests::scanners::rust_files_under(&src_root);

    let mut violations = Vec::new();

    for file in &rust_files {
        let content = fs::read_to_string(file).unwrap();
        for (line_idx, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.starts_with("#[path = ")
                && let Some(start) = line.find('"')
                && let Some(end) = line[start + 1..].find('"')
            {
                let path_str = &line[start + 1..start + 1 + end];
                let target = file.parent().unwrap().join(path_str);
                if !target.exists() {
                    violations.push(format!(
                        "{}:{}: #[path = \"{}\"] does not exist",
                        file.display(),
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
