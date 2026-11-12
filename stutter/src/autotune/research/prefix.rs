//! Read-only preflight for a study-owned compatibility directory. This does not prove Steam
//! Cloud isolation or authorize launching a game; those remain separate physical gates.
use std::{
    fs::File,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Component, Path},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StudyPrefixIdentity {
    pub device: u64,
    pub inode: u64,
    pub owner_uid: u32,
}

fn normalized_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
}

fn overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

/// Verify Steam's effective path against a precreated private study directory. The registered
/// libraries are rediscovered from the actual Steam client; callers cannot supply exclusions.
pub fn verify_study_compatdata(
    primary_steamapps: &Path,
    study_root: &Path,
    steam_compat_data_path: &Path,
    uid: u32,
    require_empty: bool,
) -> anyhow::Result<StudyPrefixIdentity> {
    anyhow::ensure!(
        uid != 0 && normalized_absolute(study_root) && normalized_absolute(steam_compat_data_path),
        "invalid study prefix path"
    );
    let expected = study_root.join("compatdata");
    anyhow::ensure!(
        steam_compat_data_path == expected,
        "Steam compatibility data path is not the study directory"
    );
    let libraries = super::discover_steam_libraries(primary_steamapps, uid)?;
    let root = File::from(crate::syscall::open_dir_no_symlinks(study_root)?);
    let root_meta = root.metadata()?;
    anyhow::ensure!(
        root_meta.is_dir() && root_meta.uid() == uid && root_meta.mode() & 0o077 == 0,
        "untrusted study root"
    );
    let compat = File::from(crate::syscall::open_dir_no_symlinks(
        steam_compat_data_path,
    )?);
    let compat_meta = compat.metadata()?;
    anyhow::ensure!(
        compat_meta.is_dir() && compat_meta.uid() == uid && compat_meta.mode() & 0o077 == 0,
        "untrusted study compatibility data"
    );
    for library in &libraries {
        let steam_root = library
            .parent()
            .ok_or_else(|| anyhow::anyhow!("invalid Steam library"))?;
        anyhow::ensure!(
            !overlap(study_root, steam_root),
            "study root overlaps a Steam library"
        );
        for protected in [
            library.join("compatdata"),
            library.join("compatdata/1091500"),
        ] {
            match crate::syscall::open_dir_no_symlinks(&protected) {
                Ok(fd) => {
                    let meta = File::from(fd).metadata()?;
                    anyhow::ensure!(
                        meta.dev() != compat_meta.dev() || meta.ino() != compat_meta.ino(),
                        "study directory aliases Steam compatibility data"
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    if require_empty {
        let mut entries = std::fs::read_dir(format!("/proc/self/fd/{}", compat.as_raw_fd()))?;
        anyhow::ensure!(
            entries.next().is_none(),
            "study compatibility directory is not empty"
        );
    }
    let root_current = File::from(crate::syscall::open_dir_no_symlinks(study_root)?).metadata()?;
    let compat_current = File::from(crate::syscall::open_dir_no_symlinks(
        steam_compat_data_path,
    )?)
    .metadata()?;
    anyhow::ensure!(
        root_meta.dev() == root_current.dev()
            && root_meta.ino() == root_current.ino()
            && root_current.uid() == uid
            && root_current.mode() & 0o077 == 0
            && compat_meta.dev() == compat_current.dev()
            && compat_meta.ino() == compat_current.ino()
            && compat_current.uid() == uid
            && compat_current.mode() & 0o077 == 0,
        "study compatibility directory changed during preflight"
    );
    anyhow::ensure!(
        super::discover_steam_libraries(primary_steamapps, uid)? == libraries,
        "Steam libraries changed during prefix preflight"
    );
    Ok(StudyPrefixIdentity {
        device: compat_meta.dev(),
        inode: compat_meta.ino(),
        owner_uid: uid,
    })
}

#[cfg(test)]
#[path = "prefix_tests.rs"]
mod tests;
