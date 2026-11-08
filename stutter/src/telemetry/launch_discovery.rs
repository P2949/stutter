//! Finds the launches that `stutter game-run` recorded, and verifies each against live state.
//!
//! A record is correlation data: it is trusted only after the capture directory passes the
//! confinement checks, the record names this boot and this user, and the recorded process is still
//! the same live process (same start ticks). Anything else is a typed rejection.
use std::{
    fs,
    path::{Path, PathBuf},
};

use super::{frame_source::LiveProcess, proc_view::live_process};
use crate::game_run::{LaunchRecord, read_launch_record};

/// Capture directories examined per discovery pass.
const MAX_CAPTURES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchRejection {
    UnsafeDirectory,
    BadRecord,
    OtherBoot,
    OtherUser,
    ProcessGone,
    ProcessReplaced,
}

#[derive(Clone, Debug)]
pub struct DiscoveredLaunch {
    pub dir: PathBuf,
    pub record: LaunchRecord,
    pub live: LiveProcess,
}

/// Verifies one capture directory.
pub fn verify_launch(
    dir: &Path,
    uid: u32,
    boot_id: &str,
    proc_root: &Path,
) -> Result<DiscoveredLaunch, LaunchRejection> {
    let record = read_launch_record(dir, uid).map_err(|error| {
        if error.to_string().contains("unsafe") {
            LaunchRejection::UnsafeDirectory
        } else {
            LaunchRejection::BadRecord
        }
    })?;
    if record.boot_id != boot_id {
        return Err(LaunchRejection::OtherBoot);
    }
    if record.uid != uid {
        return Err(LaunchRejection::OtherUser);
    }
    let live = live_process(proc_root, record.pid).ok_or(LaunchRejection::ProcessGone)?;
    if live.start_ticks != record.start_ticks || live.uid != uid {
        return Err(LaunchRejection::ProcessReplaced);
    }
    Ok(DiscoveredLaunch {
        dir: dir.to_owned(),
        record,
        live,
    })
}

/// Every verified launch under `root`, newest first. Rejected directories are skipped, never
/// guessed at.
pub fn discover(root: &Path, uid: u32, boot_id: &str, proc_root: &Path) -> Vec<DiscoveredLaunch> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = entries
        .flatten()
        .take(MAX_CAPTURES)
        .filter_map(|entry| {
            let meta = fs::symlink_metadata(entry.path()).ok()?;
            if meta.file_type().is_dir() {
                Some((meta.modified().ok()?, entry.path()))
            } else {
                None
            }
        })
        .collect();
    dirs.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    dirs.into_iter()
        .filter_map(|(_, dir)| verify_launch(&dir, uid, boot_id, proc_root).ok())
        .collect()
}
