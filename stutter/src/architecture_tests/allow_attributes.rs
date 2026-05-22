//! Architecture guard against allow attributes.

use std::fs;

use super::{crate_src_root, relative_to_crate_root, scanners::rust_files_under};

#[test]
fn allow_attributes_are_forbidden() {
    let files = rust_files_under(&crate_src_root());
    let mut violations = Vec::new();

    for file in files {
        let relative_path = relative_to_crate_root(&file);
        let source = fs::read_to_string(&file).unwrap_or_default();

        for (zero_based_line, line) in source.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }

            let allow_bang = format!("{}allow(", "#![");
            let allow_hash = format!("{}allow(", "#[");
            if trimmed.contains(&allow_bang) || trimmed.contains(&allow_hash) {
                violations.push(format!(
                    "{}:{}: allow attributes are architecture debt; fix the warning or add a narrowly reviewed architecture exception",
                    relative_path,
                    zero_based_line + 1
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "allow attribute architecture guard failed:\n{}",
        violations.join("\n")
    );
}
