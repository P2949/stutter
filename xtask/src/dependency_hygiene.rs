use std::{collections::BTreeSet, path::Path};

use anyhow::{Context, bail};
use cargo_metadata::{Dependency, Metadata, MetadataCommand, Package};

use crate::{
    process::run_process_capture_stdout,
    workflow::{
        CommandSpec, WorkflowSpec, print_workflow_header, run_command_specs,
        workflow_failure_message,
    },
};

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

const NETWORK_TLS_PACKAGE_NAMES: &[&str] = &[
    "axum",
    "h2",
    "http",
    "http-body",
    "http-body-util",
    "hyper",
    "hyper-rustls",
    "hyper-tls",
    "hyper-util",
    "mio",
    "native-tls",
    "openssl",
    "openssl-sys",
    "reqwest",
    "rustls",
    "rustls-native-certs",
    "rustls-pemfile",
    "socket2",
    "tokio",
    "tokio-rustls",
    "tonic",
    "tower",
    "tower-layer",
    "tower-service",
    "url",
    "webpki-roots",
];

pub const DEPENDENCY_HYGIENE_WORKFLOW: WorkflowSpec = WorkflowSpec {
    name: "dependency-hygiene",
    description: "validates cargo-deny policy, dependency features, network/TLS surface, and duplicate dependency families",
    affected_paths: &[
        "Cargo.toml",
        "Cargo.lock",
        "deny.toml",
        "*/Cargo.toml",
        "xtask/src/dependency_hygiene.rs",
    ],
    commands: DEPENDENCY_HYGIENE_COMMANDS,
};

pub fn run_dependency_hygiene(root: &Path) -> anyhow::Result<()> {
    print_workflow_header(DEPENDENCY_HYGIENE_WORKFLOW);
    run_command_specs(root, DEPENDENCY_HYGIENE_WORKFLOW.commands)
        .with_context(|| workflow_failure_message(DEPENDENCY_HYGIENE_WORKFLOW))?;
    run_duplicate_dependency_check(root)
        .with_context(|| workflow_failure_message(DEPENDENCY_HYGIENE_WORKFLOW))?;
    run_dependency_feature_audit(root)
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
            "duplicate dependency check found unapproved duplicate packages: {}. Inspect `cargo tree -d` and either unify versions or add a deliberate allowlist entry in xtask/src/dependency_hygiene.rs.",
            unexpected_names.join(", ")
        );
    }

    if duplicate_names.is_empty() {
        println!("duplicate versions: none");
    } else {
        println!(
            "duplicate versions: {} (approved baseline)",
            duplicate_names.join(", ")
        );
    }

    Ok(())
}

pub fn run_dependency_feature_audit(root: &Path) -> anyhow::Result<()> {
    println!("--- STAGE: dependency feature audit ---");
    let metadata = MetadataCommand::new()
        .manifest_path(root.join("Cargo.toml"))
        .exec()
        .context("failed to read cargo metadata for dependency feature audit")?;

    print_audit_section(
        "default features enabled",
        default_feature_dependency_lines(&metadata),
    );
    print_audit_section(
        "unused optional feature mappings",
        unused_optional_dependency_lines(&metadata),
    );
    print_audit_section(
        "network/TLS dependency surface",
        network_tls_dependency_lines(&metadata),
    );

    Ok(())
}

fn print_audit_section(title: &str, lines: Vec<String>) {
    println!("{title}:");
    if lines.is_empty() {
        println!("  none");
        return;
    }
    for line in lines {
        println!("  {line}");
    }
}

fn default_feature_dependency_lines(metadata: &Metadata) -> Vec<String> {
    let mut lines = metadata
        .workspace_packages()
        .into_iter()
        .flat_map(|package| {
            package
                .dependencies
                .iter()
                .filter(|dependency| dependency.uses_default_features)
                .map(|dependency| {
                    format!(
                        "{} -> {} ({})",
                        package.name,
                        dependency_name(dependency),
                        dependency.kind
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    lines.sort();
    lines
}

fn unused_optional_dependency_lines(metadata: &Metadata) -> Vec<String> {
    let mut lines = metadata
        .workspace_packages()
        .into_iter()
        .flat_map(|package| optional_dependencies_without_feature_wiring(package))
        .collect::<Vec<_>>();
    lines.sort();
    lines
}

fn optional_dependencies_without_feature_wiring(package: &Package) -> Vec<String> {
    package
        .dependencies
        .iter()
        .filter(|dependency| dependency.optional)
        .filter(|dependency| !feature_graph_references_dependency(package, dependency))
        .map(|dependency| {
            format!(
                "{} optional dependency `{}` is not referenced by any explicit feature",
                package.name,
                dependency_name(dependency)
            )
        })
        .collect()
}

fn feature_graph_references_dependency(package: &Package, dependency: &Dependency) -> bool {
    feature_entries_reference_dependency(&package.features, &dependency_name(dependency))
}

fn feature_entries_reference_dependency(
    features: &std::collections::BTreeMap<String, Vec<String>>,
    name: &str,
) -> bool {
    if features.contains_key(name) {
        return true;
    }

    features.values().flatten().any(|entry| {
        entry == name
            || entry == &format!("dep:{name}")
            || entry.starts_with(&format!("{name}/"))
            || entry.starts_with(&format!("{name}?/"))
    })
}

fn dependency_name(dependency: &Dependency) -> String {
    dependency
        .rename
        .clone()
        .unwrap_or_else(|| dependency.name.clone())
}

fn network_tls_dependency_lines(metadata: &Metadata) -> Vec<String> {
    let network_tls_names = NETWORK_TLS_PACKAGE_NAMES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let workspace_names = metadata
        .workspace_packages()
        .into_iter()
        .map(|package| package.name.to_string())
        .collect::<BTreeSet<_>>();

    let mut lines = metadata
        .packages
        .iter()
        .filter(|package| network_tls_names.contains(package.name.as_str()))
        .map(|package| {
            let scope = if workspace_names.contains(package.name.as_str()) {
                "workspace"
            } else {
                "resolved"
            };
            format!("{scope}: {} v{}", package.name, package.version)
        })
        .collect::<Vec<_>>();
    lines.sort();
    lines.dedup();
    lines
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

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn feature_reference_detector_accepts_dep_and_forwarded_feature_forms() {
        let features = BTreeMap::from([
            ("otel".to_owned(), vec!["dep:opentelemetry".to_owned()]),
            (
                "wayland-probe".to_owned(),
                vec!["wayland-client?/dlopen".to_owned()],
            ),
            ("implicit".to_owned(), Vec::new()),
        ]);

        assert!(feature_entries_reference_dependency(
            &features,
            "opentelemetry"
        ));
        assert!(feature_entries_reference_dependency(
            &features,
            "wayland-client"
        ));
        assert!(feature_entries_reference_dependency(&features, "implicit"));
        assert!(!feature_entries_reference_dependency(
            &features,
            "unused-optional"
        ));
    }
}
