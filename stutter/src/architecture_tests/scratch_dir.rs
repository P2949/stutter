use std::{fs, path::Path};

fn check_dir(dir: &Path, violations: &mut Vec<String>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if name == "scratch" {
                    violations.push(path.to_string_lossy().to_string());
                } else if name != "target" && name != ".git" {
                    check_dir(&path, violations);
                }
            }
        }
    }
}

#[test]
fn test_no_scratch_dirs() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut violations = Vec::new();

    check_dir(workspace_root, &mut violations);

    if !violations.is_empty() {
        panic!(
            "Found forbidden 'scratch' directories in repository:\n{:#?}",
            violations
        );
    }
}
