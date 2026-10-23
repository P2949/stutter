//! The representative real fixed-work overhead workload: rebuild a real Stutter package a fixed
//! number of times in a campaign-local target directory, never touching the main `target/`.
//! The cycle count `N` is calibrated once (long enough to outlive the profiling window), frozen
//! in campaign state, and never changed after counted evidence begins.
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::{CommandSpec, ToolHost};
use crate::evidence_bundle::sha256::sha256_hex;

pub const PACKAGE: &str = "stutter-core";
pub const PROFILE: &str = "release";
pub const LABEL: &str = "fixed-compile-v1";
/// Headroom over the profiling window so every measured run outlives it.
pub const MARGIN_SECONDS: f64 = 4.0;

/// Campaign-local target directory.
pub fn target_dir(campaign_dir: &Path) -> PathBuf {
    campaign_dir.join("workloads/fixed-compile-target")
}

/// `cargo clean -p <pkg>` then `cargo build -p <pkg> --release --offline`, all in `target_dir`.
pub fn cycle_commands(target_dir: &Path) -> [CommandSpec; 2] {
    let dir = target_dir.to_string_lossy().into_owned();
    [
        CommandSpec::new("cargo", &["clean", "-p", PACKAGE, "--target-dir", &dir]),
        CommandSpec::new(
            "cargo",
            &[
                "build",
                "-p",
                PACKAGE,
                "--release",
                "--offline",
                "--target-dir",
                &dir,
            ],
        ),
    ]
}

/// Cycles needed so one run lasts at least `window + margin` seconds, with 25% safety on top of
/// the measured single-cycle time. Never zero.
pub fn choose_cycles(single_cycle_seconds: f64, window_seconds: u64, margin_seconds: f64) -> u64 {
    if !single_cycle_seconds.is_finite() || single_cycle_seconds <= 0.0 {
        return 1;
    }
    let needed = (window_seconds as f64 + margin_seconds) * 1.25 / single_cycle_seconds;
    needed.ceil().max(1.0) as u64
}

/// Runs `cycles` identical rebuild cycles in `cwd`, returning the seconds taken.
pub fn run_cycles(
    host: &dyn ToolHost,
    cwd: &Path,
    target_dir: &Path,
    cycles: u64,
    now: &dyn Fn() -> std::time::Instant,
) -> Result<f64, String> {
    let started = now();
    for _ in 0..cycles {
        for mut command in cycle_commands(target_dir) {
            command.cwd = Some(cwd.to_path_buf());
            host.output(&command)?;
        }
    }
    Ok(now().duration_since(started).as_secs_f64())
}

/// Facts stored with the frozen workload (no paths, no argument text beyond the digest).
pub struct Identity<'a> {
    pub source_commit: &'a str,
    pub cargo_lock_sha256: &'a str,
    pub cargo_version: &'a str,
    pub rustc_version: &'a str,
    pub cycles: u64,
}

pub fn identity(facts: &Identity<'_>) -> Value {
    let command_digest = sha256_hex(
        format!(
            "{LABEL}\u{1f}{PACKAGE}\u{1f}{PROFILE}\u{1f}{}",
            facts.cycles
        )
        .as_bytes(),
    );
    json!({
        "label": LABEL,
        "source_commit": facts.source_commit,
        "cargo_lock_sha256": facts.cargo_lock_sha256,
        "cargo_version": facts.cargo_version,
        "rustc_version": facts.rustc_version,
        "package": PACKAGE,
        "profile": PROFILE,
        "target_dir_policy": "campaign-local, outside the main target directory",
        "cycles": facts.cycles,
        "command_sha256": command_digest,
    })
}

/// Digest bound into the task fingerprint so a changed workload invalidates its results.
pub fn workload_sha256(identity: &Value) -> String {
    sha256_hex(identity.to_string().as_bytes())
}
