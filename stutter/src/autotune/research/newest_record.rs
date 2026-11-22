//! The newest record of a named kind in a private directory, read the way every other piece of campaign evidence
//! is: through a directory descriptor, without following links, bounded, and with a present record that cannot be
//! read or parsed reported as invalid rather than skipped as if it were never there.

use std::{fs::File, io::Read, os::fd::AsFd, path::Path};

const MAX_BYTES: u64 = 256 * 1024;

/// What a scan of a directory found.
#[derive(Debug, PartialEq)]
pub enum Newest<T> {
    /// No file of this kind exists (or the directory itself is absent).
    None,
    /// The newest valid record; every matching file was readable and parsed.
    Found(T),
    /// A matching file is present but is a link, too large, unreadable or not a valid record.
    Invalid,
}

/// Scan `dir` for `<prefix>*.json` files and return the one `stamp` ranks highest.
pub fn newest<T: serde::de::DeserializeOwned>(
    dir: &Path,
    prefix: &str,
    stamp: impl Fn(&T) -> u64,
) -> Newest<T> {
    let folder = match crate::syscall::open_dir_no_symlinks(dir) {
        Ok(fd) => File::from(fd),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Newest::None,
        Err(_) => return Newest::Invalid,
    };
    let Ok(entries) = std::fs::read_dir(format!(
        "/proc/self/fd/{}",
        std::os::fd::AsRawFd::as_raw_fd(&folder)
    )) else {
        return Newest::Invalid;
    };
    let mut best: Option<T> = None;
    for entry in entries {
        let Ok(entry) = entry else {
            return Newest::Invalid;
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !(name.starts_with(prefix) && name.ends_with(".json")) {
            continue;
        }
        let Ok(file) = crate::syscall::openat_nofollow(folder.as_fd(), name) else {
            return Newest::Invalid;
        };
        let Ok(meta) = file.metadata() else {
            return Newest::Invalid;
        };
        if !meta.is_file() || meta.len() > MAX_BYTES {
            return Newest::Invalid;
        }
        let mut bytes = Vec::new();
        if file.take(MAX_BYTES + 1).read_to_end(&mut bytes).is_err() {
            return Newest::Invalid;
        }
        let Ok(candidate) = serde_json::from_slice::<T>(&bytes) else {
            return Newest::Invalid;
        };
        if best
            .as_ref()
            .is_none_or(|current| stamp(&candidate) > stamp(current))
        {
            best = Some(candidate);
        }
    }
    best.map_or(Newest::None, Newest::Found)
}

#[cfg(test)]
#[path = "newest_record_tests.rs"]
mod tests;
