use std::{fs::OpenOptions, io::Write, path::Path};

use anyhow::{Context, bail};
use stutter::api::workload_progress::{FixedWorkIdentity, run_fixed_work};

pub fn run(
    identity_path: &Path,
    markers_path: &Path,
    target_root_pid: Option<u32>,
) -> anyhow::Result<()> {
    let identity: FixedWorkIdentity = serde_json::from_slice(&std::fs::read(identity_path)?)
        .context("read fixed-work identity manifest")?;
    identity.validate().map_err(anyhow::Error::msg)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(markers_path)?;
    if !file.metadata()?.is_file() {
        bail!("fixed-work marker destination must be a regular file");
    }
    let pid = std::process::id();
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    run_fixed_work(
        identity,
        format!("fixed-work-{pid}-{timestamp}"),
        target_root_pid.unwrap_or(pid),
        |marker| {
            serde_json::to_writer(&mut file, marker)?;
            file.write_all(b"\n")?;
            file.flush()?;
            Ok(())
        },
    )
}
