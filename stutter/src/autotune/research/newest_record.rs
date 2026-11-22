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
    match newest_with_bytes(dir, prefix, stamp) {
        Newest::None => Newest::None,
        Newest::Invalid => Newest::Invalid,
        Newest::Found((record, _)) => Newest::Found(record),
    }
}

/// As `newest`, also returning the exact bytes of the chosen record, for a record another one commits to by digest.
pub fn newest_with_bytes<T: serde::de::DeserializeOwned>(
    dir: &Path,
    prefix: &str,
    stamp: impl Fn(&T) -> u64,
) -> Newest<(T, Vec<u8>)> {
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
    let mut best: Option<(T, Vec<u8>)> = None;
    // Two different records share the newest instant seen so far; a strictly newer one still settles it.
    let mut ambiguous = false;
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
        match best.as_ref() {
            // Two different records at the newest instant cannot be ordered: which one stands would depend on
            // directory order, so the scan fails closed instead of choosing.
            Some((current, kept)) if stamp(&candidate) == stamp(current) => {
                if *kept != bytes {
                    ambiguous = true;
                }
            }
            Some((current, _)) if stamp(&candidate) < stamp(current) => {}
            _ => {
                best = Some((candidate, bytes));
                ambiguous = false;
            }
        }
    }
    if ambiguous {
        return Newest::Invalid;
    }
    best.map_or(Newest::None, Newest::Found)
}

#[cfg(test)]
#[path = "newest_record_tests.rs"]
mod tests;
