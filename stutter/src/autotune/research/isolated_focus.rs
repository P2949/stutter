//! Whether the game is the focused window of the isolated session, asked of that session's own Sway-protocol compositor.
//! A route's keys go to whatever has focus in the session they are sent to, so a route that starts (or enters a phase) while
//! something else is focused would play into the wrong window. The tree is read with the isolated session's socket and
//! nothing else of the environment, parsed by the foreground module's own Sway parser, and must show a focused window
//! whose process id is the attested game's; anything less is a refusal, never an assumption.

use std::{io::Read, path::Path, time::Duration};

use crate::foreground::{
    model::{CONFIDENCE_HIGH, ForegroundProviderStatus},
    providers::sway::SwayForegroundProvider,
};

/// The longest the compositor may take to answer, and the largest tree read.
const QUERY_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_TREE_BYTES: u64 = 4 * 1024 * 1024;

/// Whether `tree_json` (a `swaymsg -t get_tree` answer) shows the focused window to be process `game_pid`.
pub fn game_is_focused(tree_json: &str, game_pid: u32) -> Result<(), &'static str> {
    let snapshot = SwayForegroundProvider::new().sample_from_tree_json(0, tree_json);
    if snapshot.status != ForegroundProviderStatus::Available {
        return Err("IsolatedFocusUnreadable");
    }
    let target = snapshot.decision.target.ok_or("IsolatedFocusUnreadable")?;
    if snapshot.decision.confidence < CONFIDENCE_HIGH || target.pid.is_none() {
        return Err("IsolatedFocusUnattributed");
    }
    if target.pid == Some(game_pid) {
        Ok(())
    } else {
        Err("GameNotFocusedInTheIsolatedSession")
    }
}

/// Ask the isolated session's compositor for its tree: `swaymsg -t get_tree` with only `SWAYSOCK` set to that session's
/// socket, bounded in time and size. `swaymsg` and the socket are absolute paths the operator named.
pub fn query_tree(swaymsg: &Path, socket: &Path) -> Result<String, &'static str> {
    if !swaymsg.is_absolute() || !socket.is_absolute() || !swaymsg.is_file() {
        return Err("IsolatedFocusProbeUnusable");
    }
    let mut child = std::process::Command::new(swaymsg)
        .args(["-t", "get_tree"])
        .env_clear()
        .env("SWAYSOCK", socket)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "IsolatedFocusProbeUnusable")?;
    let mut stdout = child.stdout.take().ok_or("IsolatedFocusProbeUnusable")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = (&mut stdout)
            .take(MAX_TREE_BYTES + 1)
            .read_to_end(&mut bytes);
        bytes
    });
    let deadline = std::time::Instant::now() + QUERY_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return Err("IsolatedFocusUnreadable"),
            Ok(None) if std::time::Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("IsolatedFocusUnreadable");
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
    let bytes = reader.join().map_err(|_| "IsolatedFocusUnreadable")?;
    if bytes.len() as u64 > MAX_TREE_BYTES {
        return Err("IsolatedFocusUnreadable");
    }
    String::from_utf8(bytes).map_err(|_| "IsolatedFocusUnreadable")
}

#[cfg(test)]
#[path = "isolated_focus_tests.rs"]
mod tests;
