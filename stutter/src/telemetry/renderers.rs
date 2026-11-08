//! Which processes in a launch tree actually render, and how many logs are live.
//!
//! MangoHud is injected into each process that draws; the CSV it writes does not say which. A
//! launch tree can hold a launcher, helpers and several rendering children, and a capture
//! directory can hold several logs, so the producer of a given row is only knowable when there is
//! exactly one candidate. These helpers count the candidates; the caller treats anything other
//! than exactly one as ambiguity.
use std::{
    fs,
    io::Read,
    path::Path,
    time::{Duration, SystemTime},
};

use super::proc_view::live_process;

/// Most `/proc` entries examined per scan.
const MAX_SCAN: usize = 8192;
/// Most bytes of one `maps` file searched.
const MAX_MAPS_BYTES: u64 = 4 * 1024 * 1024;

/// What inspecting one process's `maps` found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MapsVerdict {
    Mentions,
    /// The whole file was read and MangoHud is not in it.
    Absent,
    /// The file could not be opened or read, or was longer than the bound: nothing is proven.
    Unknown,
}

fn inspect_maps(path: &Path) -> MapsVerdict {
    let Ok(file) = fs::File::open(path) else {
        return MapsVerdict::Unknown;
    };
    let mut bytes = Vec::new();
    // `maps` is text; an invalid byte must not hide a mapping, so it is searched as bytes.
    if file.take(MAX_MAPS_BYTES).read_to_end(&mut bytes).is_err() {
        return MapsVerdict::Unknown;
    }
    let found = bytes
        .windows(b"libMangoHud".len())
        .any(|window| window == b"libMangoHud");
    if found {
        MapsVerdict::Mentions
    } else if bytes.len() as u64 >= MAX_MAPS_BYTES {
        // Read to the bound without finding it: the rest of the file was never looked at.
        MapsVerdict::Unknown
    } else {
        MapsVerdict::Absent
    }
}

/// The result of scanning a launch tree for MangoHud renderers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RendererScan {
    /// The processes (pid, start ticks) of the user at or below the launch process that have
    /// MangoHud mapped: the candidate producers of the log.
    pub found: Vec<(u32, u64)>,
    /// Whether every process that could be a renderer was inspected to the end. `false` when the
    /// scan hit its entry limit, `/proc` could not be listed, or an in-tree process's `maps` could
    /// not be read in full: an unseen renderer cannot be ruled out, so "one found" proves nothing.
    pub complete: bool,
}

impl RendererScan {
    /// The one producer, only when the scan was exhaustive and found exactly one.
    pub fn sole(&self) -> Option<(u32, u64)> {
        (self.complete && self.found.len() == 1).then(|| self.found[0])
    }
}

/// Scans the launch tree of `launch_pid` for processes of `uid` that have MangoHud mapped.
pub fn mangohud_renderers(
    proc_root: &Path,
    launch_pid: u32,
    launch_start_ticks: u64,
    uid: u32,
) -> RendererScan {
    scan_with_limit(proc_root, launch_pid, launch_start_ticks, uid, MAX_SCAN)
}

fn scan_with_limit(
    proc_root: &Path,
    launch_pid: u32,
    launch_start_ticks: u64,
    uid: u32,
    limit: usize,
) -> RendererScan {
    let Ok(entries) = fs::read_dir(proc_root) else {
        return RendererScan::default();
    };
    let mut scan = RendererScan {
        found: Vec::new(),
        complete: true,
    };
    let mut seen = 0usize;
    for entry in entries {
        let Ok(entry) = entry else {
            // A directory entry that cannot be read is a process that cannot be ruled out.
            scan.complete = false;
            continue;
        };
        seen += 1;
        if seen > limit {
            scan.complete = false;
            break;
        }
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok())
        else {
            continue;
        };
        let Some(live) = live_process(proc_root, pid) else {
            continue;
        };
        let in_tree = (pid == launch_pid && live.start_ticks == launch_start_ticks)
            || live
                .ancestors
                .iter()
                .any(|&(ancestor, ticks)| ancestor == launch_pid && ticks == launch_start_ticks);
        if live.uid != uid || !in_tree {
            continue;
        }
        match inspect_maps(&entry.path().join("maps")) {
            MapsVerdict::Mentions => scan.found.push((pid, live.start_ticks)),
            MapsVerdict::Absent => {}
            MapsVerdict::Unknown => scan.complete = false,
        }
    }
    scan
}

/// MangoHud CSV logs (not summaries) in `dir` written within `window`.
pub fn live_log_count(dir: &Path, window: Duration) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let now = SystemTime::now();
    entries
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.ends_with(".csv") && !name.contains("summary")
        })
        .filter(|entry| {
            fs::symlink_metadata(entry.path())
                .ok()
                .filter(|meta| meta.file_type().is_file())
                .and_then(|meta| meta.modified().ok())
                .is_some_and(|modified| {
                    now.duration_since(modified)
                        .map_or(true, |age| age <= window)
                })
        })
        .count()
}

#[cfg(test)]
#[path = "renderers_tests.rs"]
mod tests;
