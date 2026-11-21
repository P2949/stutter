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

fn private_dir(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn read_bounded(path: &Path) -> anyhow::Result<Vec<u8>> {
    let meta = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        meta.is_file() && meta.len() <= MAX_FILE_BYTES,
        "a raw native file is missing, not a regular file, or too large"
    );
    let mut bytes = Vec::new();
    File::open(path)?
        .take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() as u64 == meta.len(),
        "a raw native file changed while it was read"
    );
    Ok(bytes)
}

/// Copy every completed attempt's sealed native files from `results_for(arm)` into the study's archive.
/// Idempotent: a copy already present must be byte-identical.
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
        let source_root = results_for(native.proton_source);
        let archive_root = arm_root(study_dir, native.proton_source);
        for file in &native.files {
            let relative = safe_relative(&file.path)
                .ok_or_else(|| anyhow::anyhow!("a sealed native path escapes its folder"))?;
            let bytes = read_bounded(&source_root.join(&relative))?;
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
            let target = archive_root.join(&relative);
            if let Some(parent) = target.parent() {
                private_dir(parent)?;
            }
            match std::fs::symlink_metadata(&target) {
                Ok(_) => anyhow::ensure!(
                    read_bounded(&target)? == bytes,
                    "the archived copy differs from the sealed file"
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let parent = File::open(target.parent().unwrap())?;
                    write_new_bounded(
                        &parent,
                        &target.file_name().unwrap().to_string_lossy(),
                        &bytes,
                        MAX_FILE_BYTES,
                    )?;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "raw_archive_tests.rs"]
mod tests;
