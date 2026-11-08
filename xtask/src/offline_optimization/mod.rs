//! Offline PGO, AutoFDO and BOLT plans (package W13): validation only.
//!
//! These workflows build a *separate* binary from a representative profile and compare it against
//! the original on the same work. They are never part of the daemon, never replace an installed
//! executable in place, and never fabricate a profile: a missing compiler or tool is a typed
//! `Unavailable`, not a skipped pass. This module checks a plan before anything is built or run.
mod elf;
#[cfg(test)]
mod tests;

use std::{collections::BTreeSet, path::PathBuf};

use anyhow::Context;
use clap::{Args, Subcommand};
pub use elf::build_id;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Pgo,
    AutoFdo,
    Bolt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputRef {
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileRef {
    pub path: String,
    pub sha256: String,
    /// The build id of the binary the profile was collected from.
    pub build_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guardrails {
    pub max_startup_regression_percent: f64,
    pub max_size_growth_percent: f64,
    pub max_build_time_ratio: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    pub kind: Kind,
    pub source_revision: String,
    pub compiler: String,
    pub compiler_version: String,
    pub flags: Vec<String>,
    /// What the profile is trained on and what the result is judged on: they must not overlap.
    pub training_inputs: Vec<InputRef>,
    pub test_inputs: Vec<InputRef>,
    /// The binary the profile describes (the original build).
    pub input_binary: PathBuf,
    /// The separate binary this plan produces. Never an installed path.
    pub output_binary: PathBuf,
    pub profile: Option<ProfileRef>,
    pub guardrails: Guardrails,
}

/// Why a plan cannot be started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Finding {
    EmptyInputs(&'static str),
    TrainingOverlapsTest(String),
    OutputIsInput,
    /// The output would replace an installed or immutable executable.
    OutputIsInstalledPath(String),
    /// AutoFDO and BOLT need a profile.
    ProfileRequired,
    /// The profile was collected from a different build than the binary it will be applied to.
    BuildIdMismatch {
        profile: String,
        binary: String,
    },
    BuildIdUnreadable,
    /// A required tool is not installed: the workflow is unavailable here, not passed.
    Unavailable(&'static str),
    BadGuardrails,
}

/// Tools a kind needs on `PATH`.
pub fn required_tools(kind: Kind, compiler: &str) -> Vec<&'static str> {
    let compiler_tool: &'static str = if compiler.contains("clang") {
        "clang"
    } else {
        "gcc"
    };
    match kind {
        Kind::Pgo => vec![compiler_tool, "llvm-profdata"],
        Kind::AutoFdo => vec![compiler_tool, "perf", "create_llvm_prof"],
        Kind::Bolt => vec!["perf", "perf2bolt", "llvm-bolt"],
    }
}

const INSTALLED_PREFIXES: [&str; 6] = [
    "/nix/store",
    "/usr/bin",
    "/usr/local/bin",
    "/bin",
    "/usr/sbin",
    "/run/current-system",
];

/// Validates `plan`. `installed_tools` is what is on `PATH`; `binary_build_id` reads the build id
/// of a file (the real one in production, a stand-in in tests).
pub fn validate(
    plan: &Plan,
    installed_tools: &BTreeSet<String>,
    binary_build_id: impl Fn(&std::path::Path) -> Option<String>,
) -> Vec<Finding> {
    let mut findings = Vec::new();
    if plan.training_inputs.is_empty() {
        findings.push(Finding::EmptyInputs("training"));
    }
    if plan.test_inputs.is_empty() {
        findings.push(Finding::EmptyInputs("test"));
    }
    let training: BTreeSet<&String> = plan.training_inputs.iter().map(|i| &i.sha256).collect();
    for input in &plan.test_inputs {
        if training.contains(&input.sha256) {
            findings.push(Finding::TrainingOverlapsTest(input.path.clone()));
        }
    }
    if plan.output_binary == plan.input_binary {
        findings.push(Finding::OutputIsInput);
    }
    let output = plan.output_binary.to_string_lossy();
    if let Some(prefix) = INSTALLED_PREFIXES
        .iter()
        .find(|prefix| output.starts_with(**prefix))
    {
        findings.push(Finding::OutputIsInstalledPath((*prefix).to_owned()));
    }
    let g = &plan.guardrails;
    if ![
        g.max_startup_regression_percent,
        g.max_size_growth_percent,
        g.max_build_time_ratio,
    ]
    .iter()
    .all(|value| value.is_finite() && *value > 0.0)
    {
        findings.push(Finding::BadGuardrails);
    }
    if matches!(plan.kind, Kind::AutoFdo | Kind::Bolt) {
        match &plan.profile {
            None => findings.push(Finding::ProfileRequired),
            Some(profile) => match binary_build_id(&plan.input_binary) {
                None => findings.push(Finding::BuildIdUnreadable),
                Some(binary) if binary != profile.build_id => {
                    findings.push(Finding::BuildIdMismatch {
                        profile: profile.build_id.clone(),
                        binary,
                    });
                }
                Some(_) => {}
            },
        }
    }
    for tool in required_tools(plan.kind, &plan.compiler) {
        if !installed_tools.contains(tool) {
            findings.push(Finding::Unavailable(tool));
        }
    }
    findings
}

#[derive(Args, Clone, Debug, Eq, PartialEq)]
pub struct OfflineOptimizationArgs {
    #[command(subcommand)]
    pub command: OfflineOptimizationCommand,
}

#[derive(Subcommand, Clone, Debug, Eq, PartialEq)]
pub enum OfflineOptimizationCommand {
    /// Check a plan against this host's tools and the safety rules; builds and runs nothing.
    Check { plan: PathBuf },
}

pub fn run(args: &OfflineOptimizationArgs) -> anyhow::Result<()> {
    match &args.command {
        OfflineOptimizationCommand::Check { plan } => {
            let plan: Plan = serde_json::from_slice(&std::fs::read(plan)?)
                .context("not an offline optimization plan")?;
            let tools: BTreeSet<String> = [
                "clang",
                "gcc",
                "llvm-profdata",
                "perf",
                "perf2bolt",
                "llvm-bolt",
                "create_llvm_prof",
            ]
            .into_iter()
            .filter(|tool| crate::process::executable_on_path(tool).is_some())
            .map(str::to_owned)
            .collect();
            let findings = validate(&plan, &tools, |path| build_id(path).ok().flatten());
            if findings.is_empty() {
                println!("plan is valid for {:?} on this host", plan.kind);
                return Ok(());
            }
            for finding in &findings {
                println!("{finding:?}");
            }
            anyhow::bail!("{} finding(s): the workflow is not started", findings.len())
        }
    }
}
