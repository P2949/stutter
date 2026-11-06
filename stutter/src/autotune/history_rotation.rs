//! Keeps the controller's history log from growing without bound.
//!
//! An always-on controller appends a record every second, and a record is several kilobytes, so
//! an unbounded log reaches hundreds of megabytes within a day and every reader that parses it
//! (status, the TUI panel, metrics) gets slower with it. When the current file reaches
//! [`HISTORY_MAX_BYTES`] it becomes `<name>.1`, replacing the previous generation, and a new file
//! starts. Readers read the current file only, so the recent history they report stays complete.
use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::Context;

/// The size at which the current history file is set aside.
pub(crate) const HISTORY_MAX_BYTES: u64 = 128 * 1024 * 1024;

// A day of one record per second at about five kilobytes is roughly 420 MB, and the longest
// campaign history is tens of megabytes: the limit sits well between the two.
const _: () =
    assert!(HISTORY_MAX_BYTES < 400 * 1024 * 1024 && HISTORY_MAX_BYTES > 100 * 1024 * 1024);

/// The file the previous generation is kept in.
pub(crate) fn previous_generation(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".1");
    PathBuf::from(name)
}

/// Sets the history file aside when it has reached `max_bytes`. A missing file is fine, and two
/// writers racing each other are harmless: the loser finds a small or missing file.
pub(crate) fn rotate_if_full(path: &Path, max_bytes: u64) -> anyhow::Result<()> {
    let length = match fs::metadata(path) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to inspect autotune history {}", path.display()));
        }
    };
    if length < max_bytes {
        return Ok(());
    }
    match fs::rename(path, previous_generation(path)) {
        Ok(()) => Ok(()),
        // Another writer rotated it first.
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to rotate autotune history {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "stutter-history-rotation-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_file_below_the_limit_or_missing_is_left_alone() {
        let dir = scratch("below");
        let path = dir.join("history.jsonl");
        rotate_if_full(&path, 100).unwrap();
        fs::write(&path, "x".repeat(99)).unwrap();
        rotate_if_full(&path, 100).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap().len(), 99);
        assert!(!previous_generation(&path).exists());
    }

    #[test]
    fn a_full_file_becomes_the_previous_generation_and_the_oldest_is_dropped() {
        let dir = scratch("full");
        let path = dir.join("history.jsonl");
        fs::write(&path, "first".repeat(30)).unwrap();
        rotate_if_full(&path, 100).unwrap();
        assert!(!path.exists());
        assert!(
            fs::read_to_string(previous_generation(&path))
                .unwrap()
                .starts_with("first")
        );
        fs::write(&path, "second".repeat(30)).unwrap();
        rotate_if_full(&path, 100).unwrap();
        // The earlier generation was replaced, so the disk use stays bounded.
        assert!(
            fs::read_to_string(previous_generation(&path))
                .unwrap()
                .starts_with("second")
        );
        assert!(!dir.join("history.jsonl.2").exists());
    }
}
