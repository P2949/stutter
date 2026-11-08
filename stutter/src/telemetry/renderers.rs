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

fn maps_mention_mangohud(path: &Path) -> bool {
    let Ok(file) = fs::File::open(path) else {
        return false;
    };
    let mut text = String::new();
    // `maps` is text; a read error or a truncated read just yields what was seen.
    let _ = file.take(MAX_MAPS_BYTES).read_to_string(&mut text);
    text.contains("libMangoHud")
}

/// Processes of `uid` at or below the launch process that have MangoHud mapped.
pub fn count_mangohud_renderers(
    proc_root: &Path,
    launch_pid: u32,
    launch_start_ticks: u64,
    uid: u32,
) -> usize {
    let Ok(entries) = fs::read_dir(proc_root) else {
        return 0;
    };
    let mut count = 0;
    for entry in entries.flatten().take(MAX_SCAN) {
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
        if live.uid == uid && in_tree && maps_mention_mangohud(&entry.path().join("maps")) {
            count += 1;
        }
    }
    count
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
