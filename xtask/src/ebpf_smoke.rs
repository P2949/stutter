use std::path::Path;

use anyhow::bail;

use crate::{preflight::run_preflight, process::run_process, workflow::CommandSpec};

pub const EBPF_BUILD_COMMAND: CommandSpec = CommandSpec {
    program: "cargo",
    args: &["build", "-p", "stutter"],
};

pub const PRIVILEGED_EBPF_SMOKE_TEST_COMMAND: CommandSpec = CommandSpec {
    program: "bash",
    args: &["scripts/validate-privileged-ebpf-smoke.sh"],
};

#[cfg(test)]
pub const PRIVILEGED_EBPF_SMOKE_COMMANDS: &[CommandSpec] = &[PRIVILEGED_EBPF_SMOKE_TEST_COMMAND];

#[cfg(test)]
pub const UNPRIVILEGED_EBPF_SMOKE_COMMANDS: &[CommandSpec] = &[EBPF_BUILD_COMMAND];

pub fn run_unprivileged_ebpf_smoke(root: &Path) -> anyhow::Result<()> {
    run_preflight()?;
    run_process(root, EBPF_BUILD_COMMAND.program, EBPF_BUILD_COMMAND.args)?;
    println!("non-privileged eBPF build smoke passed");
    println!("to run loader tests, use:");
    println!("scripts/validate-privileged-ebpf-smoke.sh");
    Ok(())
}

pub fn run_privileged_ebpf_smoke(root: &Path) -> anyhow::Result<()> {
    if !cfg!(target_os = "linux") {
        bail!("privileged eBPF smoke tests require Linux");
    }

    run_preflight()?;
    run_process(
        root,
        PRIVILEGED_EBPF_SMOKE_TEST_COMMAND.program,
        PRIVILEGED_EBPF_SMOKE_TEST_COMMAND.args,
    )
}
