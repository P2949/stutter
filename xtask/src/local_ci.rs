//! The canonical local verification contract.
//!
//! Verification is repository-owned and runs on the workstation: this module is the single place
//! that lists what must pass. Hosted CI, GitHub runners and check-runs play no part in it (see
//! `local_ci_policy` and docs/LOCAL_VERIFICATION.md).
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, bail, ensure};

use crate::{
    process::{run_process, rustup_toolchain},
    workflow::CommandSpec,
};

const fn spec(program: &'static str, args: &'static [&'static str]) -> CommandSpec {
    CommandSpec { program, args }
}

/// One named, independently rerunnable stage of the full CI.
#[derive(Clone, Copy, Debug)]
pub struct Stage {
    pub name: &'static str,
    pub commands: &'static [CommandSpec],
}

const POLICY: &[CommandSpec] = &[spec(
    "cargo",
    &["run", "-q", "-p", "xtask", "--", "local-ci-policy"],
)];

const LINT: &[CommandSpec] = &[
    spec(
        "cargo",
        &["run", "-q", "-p", "xtask", "--", "no-allow-attrs"],
    ),
    spec(
        "cargo",
        &["run", "-q", "-p", "xtask", "--", "dependency-hygiene"],
    ),
];

/// fmt, build, clippy and the full workspace tests (`scripts/smoke/build.sh`), plus the members
/// that are not default workspace members.
const BUILD: &[CommandSpec] = &[
    spec("bash", &["scripts/smoke/build.sh"]),
    spec(
        "cargo",
        &[
            "clippy",
            "-p",
            "xtask",
            "-p",
            "stutter-validation-runner",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    ),
    spec("cargo", &["test", "-p", "xtask"]),
    spec("cargo", &["test", "-p", "stutter-validation-runner"]),
];

const OBJECTIVE: &[CommandSpec] = &[
    spec(
        "cargo",
        &["test", "-p", "stutter", "--lib", "autotune::objective"],
    ),
    spec(
        "cargo",
        &[
            "run",
            "-q",
            "-p",
            "xtask",
            "--",
            "objective-docs",
            "--check",
        ],
    ),
    spec(
        "cargo",
        &["run", "-q", "-p", "xtask", "--", "capability-check"],
    ),
];

const SMOKE: &[CommandSpec] = &[
    spec("bash", &["scripts/smoke/offline_recommendation.sh"]),
    spec("bash", &["scripts/smoke/advisor_offline.sh"]),
];

const CAMPAIGN: &[CommandSpec] = &[
    spec("cargo", &["test", "-p", "xtask", "validation_campaign"]),
    spec("cargo", &["test", "-p", "stutter-validation-runner"]),
    spec(
        "cargo",
        &["test", "-p", "xtask", "validation_campaign::tests::kernels"],
    ),
    spec(
        "cargo",
        &[
            "test",
            "-p",
            "xtask",
            "--",
            "validation_campaign::tests::scenarios",
            "validation_campaign::tests::capture_exec",
            "validation_campaign::tests::corpus",
            "validation_campaign::tests::frame_score",
            "validation_campaign::tests::overhead_real",
            "validation_campaign::tests::workloads",
        ],
    ),
    spec(
        "cargo",
        &[
            "test",
            "-p",
            "xtask",
            "--",
            "validation_campaign::tests::landing",
            "validation_campaign::tests::integration",
            "validation_campaign::tests::policy",
            "validation_campaign::tests::policy_tasks",
            "validation_campaign::tests::release",
        ],
    ),
    spec(
        "cargo",
        &[
            "test",
            "-p",
            "xtask",
            "--",
            "diagnosis_calibration",
            "capability_check",
        ],
    ),
];

const DAEMON_SAFETY: &[CommandSpec] = &[
    spec("bash", &["scripts/check-autotune-safety.sh"]),
    spec(
        "cargo",
        &["test", "-p", "stutter", "--lib", "startup_recovery"],
    ),
    spec("cargo", &["test", "-p", "stutter", "--lib", "prepared"]),
    spec(
        "cargo",
        &["test", "-p", "stutter", "--lib", "fault_reporting"],
    ),
    spec("cargo", &["test", "-p", "stutter", "autotune::simulation"]),
    spec(
        "cargo",
        &["test", "-p", "stutter", "actions::runner::tests"],
    ),
    spec("cargo", &["test", "-p", "stutter", "daemon::policy::tests"]),
    spec("cargo", &["test", "-p", "stutter", "watchdog"]),
    spec("cargo", &["test", "-p", "stutter", "soak"]),
    spec("cargo", &["test", "-p", "stutter", "service"]),
    spec("cargo", &["test", "-p", "stutter", "release"]),
    spec("cargo", &["test", "-p", "stutter", "acceptance"]),
    spec("cargo", &["test", "-p", "stutter", "architecture_tests"]),
];

const SCHEMA: &[CommandSpec] = &[
    spec(
        "cargo",
        &["test", "-p", "stutter", "artifact_contract_tests"],
    ),
    spec("cargo", &["test", "-p", "stutter", "validation_corpus"]),
];

pub const CI_STAGES: &[Stage] = &[
    Stage {
        name: "policy",
        commands: POLICY,
    },
    Stage {
        name: "lint",
        commands: LINT,
    },
    Stage {
        name: "build",
        commands: BUILD,
    },
    Stage {
        name: "objective",
        commands: OBJECTIVE,
    },
    Stage {
        name: "smoke",
        commands: SMOKE,
    },
    Stage {
        name: "campaign",
        commands: CAMPAIGN,
    },
    Stage {
        name: "daemon-safety",
        commands: DAEMON_SAFETY,
    },
    Stage {
        name: "schema",
        commands: SCHEMA,
    },
];

/// The cheap admission gate: nothing here builds the workspace.
const FAST: &[CommandSpec] = &[
    spec(
        "cargo",
        &["run", "-q", "-p", "xtask", "--", "local-ci-policy"],
    ),
    spec("cargo", &["fmt", "--all", "--", "--check"]),
    spec(
        "cargo",
        &["run", "-q", "-p", "xtask", "--", "no-allow-attrs"],
    ),
    spec(
        "cargo",
        &[
            "run",
            "-q",
            "-p",
            "xtask",
            "--",
            "objective-docs",
            "--check",
        ],
    ),
    spec(
        "cargo",
        &["run", "-q", "-p", "xtask", "--", "capability-check"],
    ),
    spec("git", &["diff", "--check"]),
];

pub fn verify_fast(root: &Path) -> anyhow::Result<()> {
    for command in FAST {
        run_process(root, command.program, command.args)?;
    }
    println!("verify-fast: PASS");
    Ok(())
}

pub fn list_stages() {
    for stage in CI_STAGES {
        println!("{}", stage.name);
    }
}

/// The full deterministic, non-root CI; `only` reruns one named stage.
pub fn ci(root: &Path, only: Option<&str>) -> anyhow::Result<()> {
    let stages: Vec<&Stage> = match only {
        Some(name) => {
            let stage = CI_STAGES.iter().find(|stage| stage.name == name);
            vec![stage.with_context(|| {
                format!(
                    "no CI stage `{name}`; stages: {}",
                    CI_STAGES
                        .iter()
                        .map(|s| s.name)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?]
        }
        None => CI_STAGES.iter().collect(),
    };
    for stage in stages {
        println!("=== ci stage: {} ===", stage.name);
        for command in stage.commands {
            run_process(root, command.program, command.args)?;
        }
    }
    println!("ci: PASS");
    Ok(())
}

/// Capabilities the live smokes need, granted to the finished test processes only.
const LIVE_CAPABILITIES: &str = "+bpf,+perfmon,+sys_resource,+dac_read_search,+sys_nice";

fn current_ids() -> anyhow::Result<(u32, u32)> {
    let meta = fs::metadata("/proc/self")?;
    Ok((
        std::os::unix::fs::MetadataExt::uid(&meta),
        std::os::unix::fs::MetadataExt::gid(&meta),
    ))
}

/// Run a finished script as the same user with just the capabilities a live smoke needs, through
/// non-interactive sudo. Cargo and xtask never run as root; a skipped smoke is a failure.
fn run_live_script(root: &Path, script: &str) -> anyhow::Result<()> {
    let (uid, gid) = current_ids()?;
    println!("--- STAGE: live smoke {script} ---");
    let output = Command::new("sudo")
        .args(["-n", "setpriv"])
        .arg(format!("--reuid={uid}"))
        .arg(format!("--regid={gid}"))
        .args(["--init-groups"])
        .arg(format!("--inh-caps={LIVE_CAPABILITIES}"))
        .arg(format!("--ambient-caps={LIVE_CAPABILITIES}"))
        .args(["--", "env"])
        .arg(format!(
            "PATH={}",
            std::env::var("PATH").unwrap_or_default()
        ))
        .arg(format!(
            "HOME={}",
            std::env::var("HOME").unwrap_or_default()
        ))
        .arg(format!("RUSTUP_TOOLCHAIN={}", rustup_toolchain()))
        .arg("RUST_LOG=info")
        .args(["bash", script])
        .current_dir(root)
        .output()
        .context(
            "sudo -n setpriv is required for the live smokes (non-interactive privilege only)",
        )?;
    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));
    ensure!(
        output.status.success(),
        "{script} failed with {}",
        output.status
    );
    ensure!(
        !String::from_utf8_lossy(&output.stdout).contains("SKIP:"),
        "{script} skipped: the live smoke needs the capabilities this stage grants"
    );
    Ok(())
}

pub fn ci_live(root: &Path) -> anyhow::Result<()> {
    ensure!(cfg!(target_os = "linux"), "the live smokes require Linux");
    run_process(root, "cargo", &["build", "-p", "stutter"])?;
    run_process(
        root,
        "cargo",
        &["run", "-q", "-p", "xtask", "--", "privileged-ebpf-smoke"],
    )?;
    run_live_script(root, "scripts/smoke/sleep_tree_churn.sh")?;
    run_live_script(root, "scripts/smoke/record_minimal.sh")?;
    println!("ci-live: PASS");
    Ok(())
}

/// The crate version, from the workspace metadata.
fn stutter_version(root: &Path) -> anyhow::Result<String> {
    let output = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .output()?;
    ensure!(output.status.success(), "cargo metadata failed");
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    metadata["packages"]
        .as_array()
        .and_then(|packages| packages.iter().find(|p| p["name"] == "stutter"))
        .and_then(|package| package["version"].as_str())
        .map(str::to_owned)
        .context("the stutter package has no version")
}

fn find_ebpf_object(root: &Path) -> anyhow::Result<PathBuf> {
    let builds = root.join("target/release/build");
    for entry in fs::read_dir(&builds).with_context(|| format!("read {}", builds.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("stutter-") {
            let candidate = entry.path().join("out/stutter");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    bail!("the eBPF object is not in the release build output; refusing to release")
}

/// Build, package and verify the release locally (the release workflow's whole job).
pub fn release_check(root: &Path, out: &Path, expected_tag: Option<&str>) -> anyhow::Result<()> {
    run_process(root, "cargo", &["build", "-p", "stutter", "--release"])?;
    if let Some(tag) = expected_tag {
        let version = stutter_version(root)?;
        ensure!(
            tag == format!("v{version}"),
            "tag {tag} does not match crate version v{version}"
        );
    }
    let object = find_ebpf_object(root)?;
    let out_text = out.to_string_lossy().into_owned();
    let object_text = object.to_string_lossy().into_owned();
    let executable = std::env::current_exe()?;
    let executable = executable.to_string_lossy().into_owned();
    run_process(
        root,
        &executable,
        &[
            "release-package",
            "--binary",
            "target/release/stutter",
            "--bpf-object",
            &object_text,
            "--out",
            &out_text,
        ],
    )?;
    run_process(root, &executable, &["release-verify", &out_text])?;
    println!("release-check: PASS ({})", out.display());
    Ok(())
}

pub fn ci_all(root: &Path, out: &Path) -> anyhow::Result<()> {
    ci(root, None)?;
    ci_live(root)?;
    release_check(root, out, None)?;
    println!("ci-all: PASS");
    Ok(())
}

/// Point this checkout's hooks at the repository's own and verify they can run.
pub fn setup(root: &Path) -> anyhow::Result<()> {
    for hook in ["pre-commit", "pre-push"] {
        let path = root.join(".githooks").join(hook);
        ensure!(path.is_file(), "{} is missing", path.display());
        let mode = std::os::unix::fs::PermissionsExt::mode(&fs::metadata(&path)?.permissions());
        ensure!(mode & 0o111 != 0, "{} is not executable", path.display());
    }
    let status = Command::new("git")
        .args(["config", "core.hooksPath", ".githooks"])
        .current_dir(root)
        .status()?;
    ensure!(status.success(), "git config core.hooksPath failed");
    println!("setup-local-ci: core.hooksPath = .githooks");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(command: &CommandSpec) -> String {
        format!("{} {}", command.program, command.args.join(" "))
    }

    fn all_commands() -> Vec<String> {
        CI_STAGES
            .iter()
            .flat_map(|stage| stage.commands.iter())
            .map(text)
            .collect()
    }

    /// Every command the retired hosted workflow ran, so deleting it drops no coverage.
    #[test]
    fn the_stages_cover_every_command_of_the_retired_hosted_workflow() {
        let retired = [
            "bash scripts/smoke/build.sh",
            "cargo run -q -p xtask -- no-allow-attrs",
            "cargo run -q -p xtask -- dependency-hygiene",
            "cargo test -p stutter --lib autotune::objective",
            "cargo run -q -p xtask -- objective-docs --check",
            "cargo run -q -p xtask -- capability-check",
            "bash scripts/smoke/offline_recommendation.sh",
            "bash scripts/smoke/advisor_offline.sh",
            "cargo test -p xtask validation_campaign",
            "cargo test -p stutter-validation-runner",
            "cargo test -p xtask validation_campaign::tests::kernels",
            "cargo test -p xtask -- validation_campaign::tests::scenarios validation_campaign::tests::capture_exec validation_campaign::tests::corpus validation_campaign::tests::frame_score validation_campaign::tests::overhead_real validation_campaign::tests::workloads",
            // The retired workflow's `repo_pr` tests are the `landing` tests now: the pull-request
            // flow was replaced by local landing.
            "cargo test -p xtask -- validation_campaign::tests::landing validation_campaign::tests::integration validation_campaign::tests::policy validation_campaign::tests::policy_tasks validation_campaign::tests::release",
            "cargo test -p xtask -- diagnosis_calibration capability_check",
            "bash scripts/check-autotune-safety.sh",
            "cargo test -p stutter --lib startup_recovery",
            "cargo test -p stutter --lib prepared",
            "cargo test -p stutter --lib fault_reporting",
            "cargo test -p stutter autotune::simulation",
            "cargo test -p stutter actions::runner::tests",
            "cargo test -p stutter daemon::policy::tests",
            "cargo test -p stutter watchdog",
            "cargo test -p stutter soak",
            "cargo test -p stutter service",
            "cargo test -p stutter release",
            "cargo test -p stutter acceptance",
            "cargo test -p stutter architecture_tests",
            "cargo test -p stutter artifact_contract_tests",
            "cargo test -p stutter validation_corpus",
        ];
        let commands = all_commands();
        for command in retired {
            assert!(commands.iter().any(|c| c == command), "missing: {command}");
        }
    }

    #[test]
    fn the_full_ci_also_runs_the_policy_and_the_whole_xtask_suite() {
        let commands = all_commands();
        assert!(commands.iter().any(|c| c.ends_with("local-ci-policy")));
        assert!(commands.iter().any(|c| c == "cargo test -p xtask"));
        assert_eq!(CI_STAGES[0].name, "policy");
    }

    #[test]
    fn stage_names_are_unique_and_a_missing_stage_is_an_error() {
        let mut names: Vec<&str> = CI_STAGES.iter().map(|stage| stage.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), CI_STAGES.len());
        assert!(ci(Path::new("."), Some("no-such-stage")).is_err());
    }

    #[test]
    fn nothing_in_the_ci_runs_as_root_or_needs_a_hosted_service() {
        for command in all_commands() {
            assert!(!command.starts_with("sudo"), "{command}");
            assert!(!command.contains("gh "), "{command}");
        }
    }
}
