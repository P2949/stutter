//! The permanent local-only rule: verification is repository-owned and runs on the workstation, so
//! the tree must not carry hosted-CI architecture or depend on the hosted-runner protocol.
//!
//! The scanner is deliberately not a grep for the word "github": flake inputs may name it,
//! documentation must be able to explain this policy, and publishing may target it. It rejects
//! hosted-CI files and the runner protocol that only a hosted runner provides.
use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::bail;

/// Files and directories that only exist to configure a hosted CI service.
const FORBIDDEN_PATHS: &[&str] = &[
    ".github/workflows",
    ".github/actions",
    ".gitlab-ci.yml",
    ".circleci",
    ".travis.yml",
    "azure-pipelines.yml",
    "Jenkinsfile",
    ".buildkite",
    ".drone.yml",
    ".woodpecker",
    "bitbucket-pipelines.yml",
];

/// Names that exist only when a hosted runner provides them.
const FORBIDDEN_TOKENS: &[&str] = &[
    "GITHUB_ACTIONS",
    "GITHUB_WORKFLOW",
    "GITHUB_EVENT_",
    "GITHUB_OUTPUT",
    "GITHUB_ENV",
    "GITHUB_STEP_SUMMARY",
    "GITHUB_REF_NAME",
    "github.event",
    "${{ runner.",
    "${{runner.",
    "${{ github.",
    "${{github.",
];

/// Where the detector itself and its explanations may name the forbidden things.
const ALLOWED_PATHS: &[&str] = &["xtask/src/local_ci_policy.rs"];
const ALLOWED_EXTENSIONS: &[&str] = &["md"];
const SCANNED_EXTENSIONS: &[&str] = &[
    "rs", "sh", "bash", "py", "toml", "yml", "yaml", "nix", "json", "cfg", "mk", "lock",
];
const SKIPPED_DIRECTORIES: &[&str] = &[
    ".git",
    "target",
    ".llm-runs",
    "dist",
    "result",
    "node_modules",
];
const MAX_SCANNED_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Eq, PartialEq)]
pub struct Violation {
    pub path: PathBuf,
    pub reason: String,
}

fn is_allowed(root: &Path, path: &Path) -> bool {
    let relative = path.strip_prefix(root).unwrap_or(path);
    ALLOWED_PATHS
        .iter()
        .any(|allowed| relative == Path::new(allowed))
        || relative
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ALLOWED_EXTENSIONS.contains(&ext))
}

fn walk(directory: &Path, files: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            let skipped = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| SKIPPED_DIRECTORIES.contains(&name));
            if !skipped {
                walk(&path, files)?;
            }
        } else if file_type.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

/// Every hosted-CI violation under `root`.
pub fn scan(root: &Path) -> anyhow::Result<Vec<Violation>> {
    let mut violations = Vec::new();
    for forbidden in FORBIDDEN_PATHS {
        let path = root.join(forbidden);
        if path.exists() {
            violations.push(Violation {
                path,
                reason: "hosted CI configuration must not exist in this repository".to_owned(),
            });
        }
    }
    let mut files = Vec::new();
    walk(root, &mut files)?;
    files.sort();
    for path in files {
        if is_allowed(root, &path) {
            continue;
        }
        let scanned = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| SCANNED_EXTENSIONS.contains(&ext));
        if !scanned || fs::metadata(&path).map_or(true, |meta| meta.len() > MAX_SCANNED_BYTES) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        for token in FORBIDDEN_TOKENS {
            if text.contains(token) {
                violations.push(Violation {
                    path: path.clone(),
                    reason: format!("depends on the hosted-runner protocol (`{token}`)"),
                });
            }
        }
        if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("yml" | "yaml")
        ) && text.lines().any(|line| {
            let line = line.trim_start().trim_start_matches("- ");
            line.starts_with("uses:") && line.contains('@')
        }) {
            violations.push(Violation {
                path: path.clone(),
                reason: "uses a hosted CI action (`uses: owner/action@ref`)".to_owned(),
            });
        }
    }
    Ok(violations)
}

pub fn run(root: &Path) -> anyhow::Result<()> {
    let violations = scan(root)?;
    if violations.is_empty() {
        println!("local-ci-policy: no hosted CI architecture found");
        return Ok(());
    }
    for violation in &violations {
        eprintln!(
            "local-ci-policy: {}: {}",
            violation
                .path
                .strip_prefix(root)
                .unwrap_or(&violation.path)
                .display(),
            violation.reason
        );
    }
    bail!(
        "{} hosted-CI violation(s): verification is repository-owned and local-only; see docs/LOCAL_VERIFICATION.md",
        violations.len()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "stutter-local-ci-policy-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(root: &Path, relative: &str, text: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn a_clean_tree_passes() {
        let root = temp("clean");
        write(&root, "src/main.rs", "fn main() {}\n");
        write(
            &root,
            "flake.nix",
            "{ inputs.nixpkgs.url = \"github:NixOS/nixpkgs\"; }\n",
        );
        assert!(scan(&root).unwrap().is_empty());
    }

    #[test]
    fn a_hosted_workflow_fails() {
        let root = temp("workflow");
        write(&root, ".github/workflows/test.yml", "name: x\n");
        let violations = scan(&root).unwrap();
        assert!(
            violations
                .iter()
                .any(|v| v.path.ends_with(".github/workflows"))
        );
    }

    #[test]
    fn another_hosted_ci_config_fails() {
        for file in [
            ".gitlab-ci.yml",
            ".travis.yml",
            "azure-pipelines.yml",
            ".circleci/config.yml",
        ] {
            let root = temp("other");
            write(&root, file, "script: true\n");
            assert!(!scan(&root).unwrap().is_empty(), "{file}");
        }
    }

    #[test]
    fn an_executable_dependency_on_the_runner_protocol_fails() {
        let root = temp("protocol");
        write(
            &root,
            "scripts/emit.sh",
            "echo \"path=x\" >> \"$GITHUB_OUTPUT\"\n",
        );
        let violations = scan(&root).unwrap();
        assert_eq!(violations.len(), 1);
        assert!(violations[0].reason.contains("GITHUB_OUTPUT"));
    }

    #[test]
    fn documentation_may_explain_the_policy() {
        let root = temp("docs");
        write(
            &root,
            "docs/LOCAL_VERIFICATION.md",
            "Hosted CI such as GitHub Actions (`.github/workflows`, `GITHUB_OUTPUT`) must not return.\n",
        );
        assert!(scan(&root).unwrap().is_empty());
    }

    #[test]
    fn the_detector_s_own_source_is_allowed_to_name_what_it_rejects() {
        let root = temp("self");
        write(
            &root,
            "xtask/src/local_ci_policy.rs",
            "const T: &str = \"GITHUB_ACTIONS\";\n",
        );
        assert!(scan(&root).unwrap().is_empty());
    }

    #[test]
    fn a_hosted_action_reference_in_yaml_fails() {
        let root = temp("uses");
        write(
            &root,
            "ci/steps.yml",
            "steps:\n  - uses: actions/checkout@v4\n",
        );
        assert!(
            scan(&root)
                .unwrap()
                .iter()
                .any(|v| v.reason.contains("hosted CI action"))
        );
    }

    #[test]
    fn build_output_directories_are_not_scanned() {
        let root = temp("target");
        write(
            &root,
            "target/debug/build/x/out.sh",
            "echo $GITHUB_OUTPUT\n",
        );
        assert!(scan(&root).unwrap().is_empty());
    }
}
