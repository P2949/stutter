use std::{collections::BTreeSet, path::Path};
use anyhow::{Context, bail};

use crate::workflow::{CommandSpec, WorkflowSpec, print_workflow_header, workflow_failure_message, run_command_specs};
use crate::process::run_process_capture_stdout;

pub const DEPENDENCY_HYGIENE_COMMANDS: &[CommandSpec] = &[CommandSpec {
    program: "cargo",
    args: &["deny", "check"],
}];

pub const DUPLICATE_DEPENDENCY_COMMAND: CommandSpec = CommandSpec {
    program: "cargo",
    args: &["tree", "-d"],
};

pub const APPROVED_DUPLICATE_PACKAGES: &[&str] = &[
    "bitflags",
    "either",
    "foldhash",
    "getrandom",
    "hashbrown",
    "indexmap",
    "linux-raw-sys",
    "memchr",
    "r-efi",
    "rand",
    "rand_chacha",
    "rand_core",
    "rustix",
    "serde",
    "serde_core",
    "serde_json",
    "socket2",
    "stutter-common",
    "syn",
    "thiserror",
    "thiserror-impl",
    "tower",
    "which",
    "windows-sys",
    "wit-bindgen",
];

pub const DEPENDENCY_HYGIENE_WORKFLOW: WorkflowSpec = WorkflowSpec {
    name: "dependency-hygiene",
    description: "validates cargo-deny policy and rejects newly introduced duplicate dependency families",
    affected_paths: &[
        "Cargo.toml",
        "Cargo.lock",
        "deny.toml",
        "*/Cargo.toml",
        "xtask/src/main.rs",
    ],
    commands: DEPENDENCY_HYGIENE_COMMANDS,
};

pub fn run_dependency_hygiene(root: &Path) -> anyhow::Result<()> {
    print_workflow_header(DEPENDENCY_HYGIENE_WORKFLOW);
    run_command_specs(root, DEPENDENCY_HYGIENE_WORKFLOW.commands)
        .with_context(|| workflow_failure_message(DEPENDENCY_HYGIENE_WORKFLOW))?;
    run_duplicate_dependency_check(root)
        .with_context(|| workflow_failure_message(DEPENDENCY_HYGIENE_WORKFLOW))
}

pub fn run_duplicate_dependency_check(root: &Path) -> anyhow::Result<()> {
    let output = run_process_capture_stdout(
        root,
        DUPLICATE_DEPENDENCY_COMMAND.program,
        DUPLICATE_DEPENDENCY_COMMAND.args,
    )?;
    let duplicate_names = duplicate_package_names(&output);
    let approved_names = APPROVED_DUPLICATE_PACKAGES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    let unexpected_names = duplicate_names
        .iter()
        .filter(|name| !approved_names.contains(name.as_str()))
        .cloned()
        .collect::<Vec<_>>();

    if !unexpected_names.is_empty() {
        bail!(
            "duplicate dependency check found unapproved duplicate packages: {}. Inspect `cargo tree -d` and either unify versions or add a deliberate allowlist entry in xtask/src/main.rs.",
            unexpected_names.join(", ")
        );
    }

    Ok(())
}

pub fn duplicate_package_names(output: &str) -> Vec<String> {
    let mut names = BTreeSet::new();

    for line in output.lines() {
        let Some(first) = line.chars().next() else {
            continue;
        };

        if first.is_whitespace() || matches!(first, '├' | '└' | '│') {
            continue;
        }

        let Some((name, version_tail)) = line.split_once(" v") else {
            continue;
        };

        if version_tail
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
        {
            names.insert(name.to_owned());
        }
    }

    names.into_iter().collect()
}
