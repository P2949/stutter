//! Bounded archive of a study's raw native evidence under the campaign's own ownership. The game
//! writes its result folders into the Proton prefix, where they accumulate without bound and can be
//! rotated, edited or deleted by anything the user runs. Once an attempt is sealed, the files its
//! completion names are copied beside the study (`raw/<arm>/<result folder>/...`, private, write-once),
//! each re-hashed against the sealed list as it is copied, so later verification replays the campaign's
//! own copy rather than a directory it does not control. The layout mirrors a results directory, so the
//! archive is simply another `results_for` root. A file that differs from what was sealed, an existing
//! archive copy that differs, a path that escapes its folder, or a size over the bound is an error;
//! nothing is ever overwritten.

use std::{
    collections::BTreeMap,
    fs::File,
    io::Read,
    os::fd::AsRawFd,
    path::{Component, Path, PathBuf},
};

use super::{ProtonSource, report::AttemptEvidenceV1, report_store::write_new_bounded};

/// One archived file is never larger than this (the game's frame file is a few hundred KiB).
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// All raw files of one study together.
const MAX_STUDY_BYTES: u64 = 512 * 1024 * 1024;
const RAW_DIR: &str = "raw";

/// A relative path made only of ordinary names.
fn safe_relative(path: &str) -> Option<PathBuf> {
    let candidate = Path::new(path);
    (!path.is_empty()
        && candidate
            .components()
            .all(|part| matches!(part, Component::Normal(_))))
    .then(|| candidate.to_path_buf())
}

/// The archive root for one arm, as a path usable by a `results_for` closure.
pub fn arm_root(study_dir: &File, source: ProtonSource) -> PathBuf {
    // The replay opens its root without following symlinks, so the study directory is named by its real
    // path rather than through /proc/self/fd.
    let study = std::fs::read_link(format!("/proc/self/fd/{}", study_dir.as_raw_fd()))
        .unwrap_or_else(|_| PathBuf::from(format!("/proc/self/fd/{}", study_dir.as_raw_fd())));
    study.join(RAW_DIR).join(source.dir_name())
}

/// Whether a study has an archive.
pub fn exists(study_dir: &File) -> bool {
    std::fs::symlink_metadata(format!("/proc/self/fd/{}/{RAW_DIR}", study_dir.as_raw_fd()))
        .is_ok_and(|meta| meta.is_dir())
}

/// Open (creating private directories as needed) the folder holding `components`, walking from `root` one
/// directory descriptor at a time without following any link. Each directory must be a private directory
/// owned by this user, whether it was just made or already there.
fn open_chain(
    root: &File,
    components: &[std::ffi::OsString],
    create: bool,
) -> anyhow::Result<File> {
    use std::os::{fd::AsFd, unix::fs::MetadataExt};
    let mut dir = root.try_clone()?;
    for component in components {
        let name = component
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("a raw path component is not text"))?;
        if create {
            match crate::syscall::mkdir_private_at(dir.as_fd(), name) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        let next = File::from(crate::syscall::openat_dir_nofollow(dir.as_fd(), name)?);
        let meta = next.metadata()?;
        anyhow::ensure!(
            meta.uid() == crate::syscall::geteuid() && meta.mode() & 0o077 == 0,
            "an archive directory is not private to this user"
        );
        dir = next;
    }
    Ok(dir)
}

/// A regular file's bytes, read through its directory descriptor without following a link, and bounded.
/// The length is taken from the open descriptor, so a swap after opening cannot change what was checked.
fn read_at(dir: &File, name: &str) -> anyhow::Result<Vec<u8>> {
    use std::os::fd::AsFd;
    let file = crate::syscall::openat_nofollow(dir.as_fd(), name)?;
    let meta = file.metadata()?;
    anyhow::ensure!(
        meta.is_file() && meta.len() <= MAX_FILE_BYTES,
        "a raw native file is missing, not a regular file, or too large"
    );
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 == meta.len(),
        "a raw native file changed while it was read"
    );
    Ok(bytes)
}

fn components_of(relative: &Path) -> Vec<std::ffi::OsString> {
    relative
        .components()
        .map(|part| part.as_os_str().to_owned())
        .collect()
}

/// Copy every completed attempt's sealed native files from `results_for(arm)` into the study's archive.
/// Idempotent: a copy already present must be byte-identical. Both trees are walked by directory
/// descriptor without following links.
pub fn archive(
    study_dir: &File,
    evidence: &BTreeMap<u32, AttemptEvidenceV1>,
    results_for: &dyn Fn(ProtonSource) -> PathBuf,
) -> anyhow::Result<()> {
    let mut total = 0u64;
    for item in evidence.values() {
        let Some(native) = item.native_completion.as_deref() else {
            continue;
        };
        let source_root = File::from(crate::syscall::open_dir_no_symlinks(&results_for(
            native.proton_source,
        ))?);
        let arm_components = vec![
            std::ffi::OsString::from(RAW_DIR),
            std::ffi::OsString::from(native.proton_source.dir_name()),
        ];
        for file in &native.files {
            let relative = safe_relative(&file.path)
                .ok_or_else(|| anyhow::anyhow!("a sealed native path escapes its folder"))?;
            let mut parts = components_of(&relative);
            let leaf = parts
                .pop()
                .and_then(|leaf| leaf.into_string().ok())
                .ok_or_else(|| anyhow::anyhow!("a native file has no folder or name"))?;
            let source_dir = open_foreign_chain(&source_root, &parts)?;
            let bytes = read_at(&source_dir, &leaf)?;
            anyhow::ensure!(
                bytes.len() as u64 == file.size_bytes
                    && stutter_core::hash::sha256_hex(&bytes) == file.sha256,
                "a native file differs from what was sealed"
            );
            total += bytes.len() as u64;
            anyhow::ensure!(
                total <= MAX_STUDY_BYTES,
                "the raw archive exceeds its bound"
            );
            let mut target = arm_components.clone();
            target.extend(parts);
            let folder = open_chain(study_dir, &target, true)?;
            match read_at(&folder, &leaf) {
                Ok(existing) => anyhow::ensure!(
                    existing == bytes,
                    "the archived copy differs from the sealed file"
                ),
                Err(error)
                    if error
                        .downcast_ref::<std::io::Error>()
                        .is_some_and(|io| io.kind() == std::io::ErrorKind::NotFound) =>
                {
                    write_new_bounded(&folder, &leaf, &bytes, MAX_FILE_BYTES)?;
                }
                Err(error) => return Err(error),
            }
        }
    }
    Ok(())
}

/// The source tree belongs to the game, not to the campaign: walk it without following links but without
/// requiring it to be private.
fn open_foreign_chain(root: &File, components: &[std::ffi::OsString]) -> anyhow::Result<File> {
    use std::os::fd::AsFd;
    let mut dir = root.try_clone()?;
    for component in components {
        let name = component
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("a raw path component is not text"))?;
        dir = File::from(crate::syscall::openat_dir_nofollow(dir.as_fd(), name)?);
    }
    Ok(dir)
}

#[cfg(test)]
#[path = "raw_archive_tests.rs"]
mod tests;
