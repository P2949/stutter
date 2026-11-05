//! Isolation between attempts at one scheduled round.
use std::{fs, path::Path};

use anyhow::Context;

/// A repeated round starts in an empty directory: the readiness markers, stderr files and runtime
/// receipts of the attempt before it must not be mistaken for the new attempt's. The earlier
/// attempt is kept beside it as `<index>.attempt-<n>` for the audit.
pub(super) fn archive_previous_attempt(dir: &Path) -> anyhow::Result<()> {
    if !dir.exists() {
        return Ok(());
    }
    let name = dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    for attempt in 1.. {
        let kept = dir.with_file_name(format!("{name}.attempt-{attempt}"));
        if !kept.exists() {
            fs::rename(dir, &kept).with_context(|| {
                format!("could not set aside the earlier attempt {}", dir.display())
            })?;
            return Ok(());
        }
    }
    Ok(())
}

#[cfg(test)]
mod attempt_tests {
    use super::*;

    #[test]
    fn a_repeated_round_starts_empty_and_keeps_every_earlier_attempt() {
        let root = std::env::temp_dir().join(format!("round-attempts-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let dir = root.join("rounds/0007");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("game-1.ready"), "first").unwrap();
        archive_previous_attempt(&dir).unwrap();
        assert!(
            !dir.exists(),
            "the stale marker must not be visible to the next attempt"
        );
        assert_eq!(
            fs::read_to_string(root.join("rounds/0007.attempt-1/game-1.ready")).unwrap(),
            "first"
        );
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("game-1.ready"), "second").unwrap();
        archive_previous_attempt(&dir).unwrap();
        assert!(!dir.exists());
        assert_eq!(
            fs::read_to_string(root.join("rounds/0007.attempt-2/game-1.ready")).unwrap(),
            "second"
        );
        assert!(root.join("rounds/0007.attempt-1").exists());
        // A first attempt has nothing to archive.
        archive_previous_attempt(&root.join("rounds/0009")).unwrap();
        let _ = fs::remove_dir_all(&root);
    }
}
