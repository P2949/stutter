//! Disposable-cgroup manipulation inside the campaign's own delegated subtree, as the desktop
//! user. Nothing outside that subtree is ever touched.
use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;

/// Destroys and recreates `<subtree>/suite`: the same path with a different inode. Tasks still in
/// it (a fixture's helpers that Stutter did not move) are parked in a sibling first, because a
/// cgroup with members cannot be removed. Returns the parking cgroup.
pub(super) fn recreate_origin(subtree: &Path) -> anyhow::Result<PathBuf> {
    let origin = subtree.join("suite");
    let parking = subtree.join("parking");
    fs::create_dir_all(&parking)?;
    for pid in fs::read_to_string(origin.join("cgroup.procs"))?.lines() {
        fs::write(parking.join("cgroup.procs"), pid.trim())
            .with_context(|| format!("park pid {pid} while the origin is recreated"))?;
    }
    fs::remove_dir(&origin).context("remove the origin cgroup")?;
    fs::create_dir(&origin).context("recreate the origin cgroup")?;
    Ok(parking)
}
