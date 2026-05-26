use std::path::Path;

use anyhow::bail;

use crate::{
    preflight::run_preflight,
    process::{run_process, run_process_with_env},
    workflow::CommandSpec,
};

pub const EBPF_BUILD_COMMAND: CommandSpec = CommandSpec {
    program: "cargo",
    args: &["build", "-p", "stutter"],
};

pub const PRIVILEGED_EBPF_SMOKE_TEST_COMMAND: CommandSpec = CommandSpec {
    program: "cargo",
    args: &["test", "-p", "stutter", "privileged_", "--", "--nocapture"],
};

#[cfg(test)]
pub const PRIVILEGED_EBPF_SMOKE_COMMANDS: &[CommandSpec] =
    &[EBPF_BUILD_COMMAND, PRIVILEGED_EBPF_SMOKE_TEST_COMMAND];

#[cfg(test)]
pub const UNPRIVILEGED_EBPF_SMOKE_COMMANDS: &[CommandSpec] = &[EBPF_BUILD_COMMAND];

pub fn run_unprivileged_ebpf_smoke(root: &Path) -> anyhow::Result<()> {
    run_preflight()?;
    run_process(root, EBPF_BUILD_COMMAND.program, EBPF_BUILD_COMMAND.args)?;
    println!("non-privileged eBPF build smoke passed");
    println!("to run loader tests, use:");
    println!(
        "doas env HOME=\"$HOME\" CARGO_HOME=\"$HOME/.cargo\" RUSTUP_HOME=\"$HOME/.rustup\" PATH=\"$PATH\" RUSTUP_TOOLCHAIN=nightly cargo run -p xtask -- privileged-ebpf-smoke"
    );
    Ok(())
}

pub fn run_privileged_ebpf_smoke(root: &Path) -> anyhow::Result<()> {
    if !cfg!(target_os = "linux") {
        bail!("privileged eBPF smoke tests require Linux");
    }

    run_preflight()?;
    run_process(root, EBPF_BUILD_COMMAND.program, EBPF_BUILD_COMMAND.args)?;
    run_process_with_env(
        root,
        PRIVILEGED_EBPF_SMOKE_TEST_COMMAND.program,
        PRIVILEGED_EBPF_SMOKE_TEST_COMMAND.args,
        &[("STUTTER_RUN_PRIVILEGED_EBPF_TESTS", "1")],
    )
}
