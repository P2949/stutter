use std::{env, process::Command as ProcessCommand};

use anyhow::{Context, bail};

use crate::process::{check_prebuilt_bpf_object, check_program_on_path};

pub fn run_preflight() -> anyhow::Result<()> {
    check_program_on_path(
        "cargo",
        "install Rust with rustup, then run this command from the repository root",
    )?;
    if env::var_os("STUTTER_USE_PREBUILT_BPF").as_deref() == Some(std::ffi::OsStr::new("1")) {
        check_prebuilt_bpf_object()?;
    } else {
        check_program_on_path(
            "bpf-linker",
            "install it with `cargo install bpf-linker`, or build stutter with STUTTER_USE_PREBUILT_BPF=1 and STUTTER_BPF_OBJECT=/path/to/stutter",
        )?;
    }

    let toolchain = crate::process::rustup_toolchain();
    let output = if let Some(path) = crate::process::executable_on_path("rustup") {
        println!("rustup OK: {}", path.display());
        let output = ProcessCommand::new("rustup")
            .args(["run", toolchain.as_str(), "rustc", "--version"])
            .output()
            .with_context(|| format!("failed to query rustup toolchain `{toolchain}`"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "rustup toolchain `{}` is not usable: {}. Install it with `rustup toolchain install {}` and include rust-src/rustfmt/clippy components.",
                toolchain,
                stderr.trim(),
                toolchain
            );
        }
        output
    } else {
        check_program_on_path(
            "rustc",
            "install Rust or include rustc on PATH so the configured toolchain can be checked",
        )?;
        let output = ProcessCommand::new("rustc")
            .arg("--version")
            .env("RUSTUP_TOOLCHAIN", toolchain.as_str())
            .output()
            .context("failed to query rustc version")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("rustc on PATH is not usable: {}", stderr.trim());
        }
        output
    };

    let version = String::from_utf8_lossy(&output.stdout);
    println!("toolchain `{toolchain}` OK: {}", version.trim());
    println!(
        "preflight OK: build as your normal user, then run the built binary with sudo/doas when eBPF loading needs privileges"
    );
    Ok(())
}
