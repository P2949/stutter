use std::{
    env,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use anyhow::bail;

pub mod ownership;
pub mod supervisor;
use supervisor::{ExecutionPolicy, Outcome, Supervised};

pub const DEFAULT_TOOLCHAIN: &str = "nightly-2026-06-06";

pub fn run_cargo(root: &Path, args: &[&str]) -> anyhow::Result<()> {
    run_process(root, "cargo", args)
}

pub fn run_process(root: &Path, program: &str, args: &[&str]) -> anyhow::Result<()> {
    run_process_with_env(root, program, args, &[])
}

pub fn run_process_with_env(
    root: &Path,
    program: &str,
    args: &[&str],
    extra_env: &[(&str, &str)],
) -> anyhow::Result<()> {
    run_process_with_policy(root, program, args, extra_env, &stage_policy())
}

pub fn run_process_with_policy(
    root: &Path,
    program: &str,
    args: &[&str],
    extra_env: &[(&str, &str)],
    policy: &ExecutionPolicy,
) -> anyhow::Result<()> {
    let command_text = format_command(program, args);
    println!("--- STAGE: {command_text} ---");

    let mut command = ProcessCommand::new(program);
    command
        .args(args)
        .current_dir(root)
        .env("RUSTUP_TOOLCHAIN", rustup_toolchain());
    for (key, value) in extra_env {
        command.env(key, value);
    }

    check_outcome(&command_text, &supervisor::run(command, policy)?)
}

pub fn run_process_capture_stdout(
    root: &Path,
    program: &str,
    args: &[&str],
) -> anyhow::Result<String> {
    let command_text = format_command(program, args);
    println!("--- STAGE: {command_text} ---");

    let mut command = ProcessCommand::new(program);
    command
        .args(args)
        .current_dir(root)
        .env("RUSTUP_TOOLCHAIN", rustup_toolchain());
    let run = supervisor::run(command, &stage_policy())?;
    check_outcome(&command_text, &run)?;
    Ok(run.stdout)
}

pub fn rustup_toolchain() -> String {
    env::var("RUSTUP_TOOLCHAIN").unwrap_or_else(|_| DEFAULT_TOOLCHAIN.to_owned())
}

pub fn format_command(program: &str, args: &[&str]) -> String {
    std::iter::once(program)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn check_prebuilt_bpf_object() -> anyhow::Result<()> {
    let object = env::var_os("STUTTER_BPF_OBJECT")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("STUTTER_USE_PREBUILT_BPF=1 requires STUTTER_BPF_OBJECT"))?;

    if !object.is_file() {
        bail!(
            "STUTTER_BPF_OBJECT must point at an existing BPF object file: {}",
            object.display()
        );
    }

    println!("prebuilt BPF object OK: {}", object.display());
    Ok(())
}

pub fn check_program_on_path(program: &str, hint: &str) -> anyhow::Result<PathBuf> {
    if let Some(path) = executable_on_path(program) {
        println!("{program} OK: {}", path.display());
        return Ok(path);
    }

    bail!("required program `{program}` was not found on PATH; {hint}")
}

pub fn executable_on_path(program: &str) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    env::split_paths(&paths)
        .map(|dir| dir.join(program))
        .find(|path| path.is_file())
}

/// Per-stage policy: a generous bound (long builds are legitimate) that can be tightened with
/// `STUTTER_XTASK_STAGE_TIMEOUT_SECS`. Output is echoed as it arrives and a bounded copy kept.
pub fn stage_policy() -> ExecutionPolicy {
    let mut policy = ExecutionPolicy::long_build();
    if let Some(seconds) = env::var("STUTTER_XTASK_STAGE_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
    {
        policy.timeout = Some(std::time::Duration::from_secs(seconds));
    }
    policy
}

fn check_outcome(command_text: &str, run: &Supervised) -> anyhow::Result<()> {
    match &run.outcome {
        Outcome::Exited(status) if status.success() => Ok(()),
        Outcome::Exited(status) => bail!("command `{command_text}` failed with status {status}"),
        Outcome::TimedOut => bail!(
            "command `{command_text}` exceeded its deadline; its owned cgroup was terminated and its child reaped"
        ),
        Outcome::Cancelled => bail!(
            "command `{command_text}` was cancelled; its owned cgroup was terminated and its child reaped"
        ),
    }
}
