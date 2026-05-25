use std::{
    env,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};
use anyhow::{Context, bail};

pub const DEFAULT_TOOLCHAIN: &str = "nightly";

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

    let status = command
        .status()
        .with_context(|| format!("failed to start `{command_text}`"))?;

    if !status.success() {
        bail!("command `{command_text}` failed with status {status}");
    }

    Ok(())
}

pub fn run_process_capture_stdout(root: &Path, program: &str, args: &[&str]) -> anyhow::Result<String> {
    let command_text = format_command(program, args);
    println!("--- STAGE: {command_text} ---");

    let output = ProcessCommand::new(program)
        .args(args)
        .current_dir(root)
        .env("RUSTUP_TOOLCHAIN", rustup_toolchain())
        .output()
        .with_context(|| format!("failed to start `{command_text}`"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    if !stdout.is_empty() {
        print!("{stdout}");
    }
    if !stderr.is_empty() {
        eprint!("{stderr}");
    }

    if !output.status.success() {
        bail!(
            "command `{command_text}` failed with status {}",
            output.status
        );
    }

    Ok(stdout)
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
