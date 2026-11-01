//! A controlled build used by the service acceptance: a `make` parent with busy `cc1` children.
use std::path::Path;

use anyhow::{Context, ensure};

/// A controlled build: this process (run under the name `make`) keeps `jobs` compiler children
/// busy for `seconds`. The children are copies of this binary named by the caller (`cc1`).
pub fn compile(child: &Path, jobs: u32, seconds: u64) -> anyhow::Result<()> {
    ensure!(
        (1..=16).contains(&jobs) && (1..=3_600).contains(&seconds),
        "invalid compile parameters"
    );
    let mut children = Vec::new();
    for _ in 0..jobs {
        children.push(
            std::process::Command::new(child)
                .args(["live-burst", "--burst-ms", "18", "--period-ms", "20"])
                .args(["--seconds", &seconds.to_string()])
                .spawn()
                .context("start a compiler job")?,
        );
    }
    for mut job in children {
        job.wait()?;
    }
    Ok(())
}
