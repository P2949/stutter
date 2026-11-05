//! Remembers which exact states of a pinned file already hashed to their digest.
//!
//! The root runner re-proves its pinned binaries on every request, which costs a hash of several
//! hundred megabytes each time. A file that has not changed since it last matched, judged by
//! device, inode, size, modification and change time (which only the kernel advances), still
//! matches; any replacement or write changes at least one of them and is hashed in full again.
use std::{
    collections::HashMap,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileState {
    dev: u64,
    ino: u64,
    size: u64,
    mtime: (i64, i64),
    ctime: (i64, i64),
}

static MATCHED: Mutex<Option<HashMap<PathBuf, (FileState, String)>>> = Mutex::new(None);

pub(crate) fn state(path: &Path) -> Option<FileState> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(FileState {
        dev: metadata.dev(),
        ino: metadata.ino(),
        size: metadata.size(),
        mtime: (metadata.mtime(), metadata.mtime_nsec()),
        ctime: (metadata.ctime(), metadata.ctime_nsec()),
    })
}

/// Whether this exact state of `path` already matched `expected_sha256`.
pub(crate) fn matched(path: &Path, current: &FileState, expected_sha256: &str) -> bool {
    MATCHED
        .lock()
        .ok()
        .and_then(|known| {
            known
                .as_ref()?
                .get(path)
                .map(|(state, digest)| state == current && digest == expected_sha256)
        })
        .unwrap_or(false)
}

/// Records a match, but only if the file did not change while it was being hashed.
pub(crate) fn remember(path: &Path, before: FileState, expected_sha256: &str) {
    if state(path).as_ref() != Some(&before) {
        return;
    }
    if let Ok(mut known) = MATCHED.lock() {
        known
            .get_or_insert_with(HashMap::new)
            .insert(path.to_owned(), (before, expected_sha256.to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use stutter_core::hash::sha256_hex;

    use crate::process::verify_pinned;

    #[test]
    fn an_unchanged_file_is_remembered_and_any_change_is_hashed_again() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pinned");
        std::fs::write(&path, b"authorized bytes").unwrap();
        let digest = sha256_hex(b"authorized bytes");
        verify_pinned(&path, &digest).unwrap();
        verify_pinned(&path, &digest).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        // A same-size rewrite changes the change time, so it cannot ride on the earlier match.
        std::fs::write(&path, b"tampered  bytes!").unwrap();
        assert!(verify_pinned(&path, &digest).is_err());
        // A different digest for the same bytes is never satisfied by the remembered one.
        std::fs::write(&path, b"authorized bytes").unwrap();
        verify_pinned(&path, &digest).unwrap();
        assert!(verify_pinned(&path, &sha256_hex(b"something else")).is_err());
    }
}
