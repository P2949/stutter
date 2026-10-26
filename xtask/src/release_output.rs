//! Where `release-check` leaves its package, and how it gets there.
//!
//! The package is always built and verified in a unique staging directory beside the destination
//! (same filesystem, so the final step is a rename) and only then moved into place. A failed
//! attempt therefore never disturbs an earlier verified result. `release-package` itself keeps its
//! no-clobber contract: it only ever sees the fresh staging path.
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, bail, ensure};

/// The xtask-managed output, relative to the repository root.
pub const MANAGED_OUTPUT: &str = "target/release-check";

const STAGING_MARKER: &str = ".staging-";
const BACKUP_MARKER: &str = ".previous-";

/// Who owns the destination.
#[derive(Debug, Eq, PartialEq)]
pub enum Destination {
    /// No `--out`: `target/release-check`, which a rerun may replace once the new package verified.
    Managed(PathBuf),
    /// `--out`: the caller's directory, which must not exist yet.
    Explicit(PathBuf),
}

impl Destination {
    pub fn resolve(root: &Path, out: Option<&Path>) -> Self {
        match out {
            Some(out) => Self::Explicit(root.join(out)),
            None => Self::Managed(root.join(MANAGED_OUTPUT)),
        }
    }

    pub fn path(&self) -> &Path {
        match self {
            Self::Managed(path) | Self::Explicit(path) => path,
        }
    }

    /// Refuse an explicit destination that already exists, before any work is done.
    pub fn preflight(&self) -> anyhow::Result<()> {
        if let Self::Explicit(path) = self {
            ensure!(
                fs::symlink_metadata(path).is_err(),
                "{} already exists; release-check will not overwrite a caller-owned destination",
                path.display()
            );
        }
        Ok(())
    }
}

/// A unique directory beside the destination, removed on drop unless it was installed.
#[derive(Debug)]
pub struct Staging {
    path: PathBuf,
}

fn unique_suffix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    format!("{}-{nanos}", std::process::id())
}

fn sibling(destination: &Path, marker: &str) -> anyhow::Result<PathBuf> {
    let name = destination
        .file_name()
        .with_context(|| format!("{} has no final component", destination.display()))?;
    Ok(destination.with_file_name(format!(
        "{}{marker}{}",
        name.to_string_lossy(),
        unique_suffix()
    )))
}

/// Staging directories left by a process that no longer exists (a killed run).
fn sweep_abandoned(destination: &Path) -> anyhow::Result<()> {
    let (Some(parent), Some(name)) = (destination.parent(), destination.file_name()) else {
        return Ok(());
    };
    let prefix = format!("{}{STAGING_MARKER}", name.to_string_lossy());
    let Ok(entries) = fs::read_dir(parent) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let entry_name = entry.file_name().to_string_lossy().into_owned();
        let Some(rest) = entry_name.strip_prefix(&prefix) else {
            continue;
        };
        let owner_alive = rest
            .split('-')
            .next()
            .and_then(|pid| pid.parse::<u32>().ok())
            .is_some_and(|pid| Path::new("/proc").join(pid.to_string()).exists());
        if !owner_alive {
            fs::remove_dir_all(entry.path()).ok();
        }
    }
    Ok(())
}

impl Staging {
    pub fn create(destination: &Destination) -> anyhow::Result<Self> {
        let final_path = destination.path();
        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
        }
        sweep_abandoned(final_path)?;
        Ok(Self {
            path: sibling(final_path, STAGING_MARKER)?,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Move the verified package to its destination.
    pub fn install(self, destination: &Destination) -> anyhow::Result<()> {
        let final_path = destination.path();
        match destination {
            Destination::Explicit(_) => {
                ensure!(
                    fs::symlink_metadata(final_path).is_err(),
                    "{} appeared while the release was being checked; not overwriting it",
                    final_path.display()
                );
                fs::rename(&self.path, final_path)
                    .with_context(|| format!("install {}", final_path.display()))
            }
            Destination::Managed(_) => {
                if fs::symlink_metadata(final_path).is_err() {
                    return fs::rename(&self.path, final_path)
                        .with_context(|| format!("install {}", final_path.display()));
                }
                let backup = sibling(final_path, BACKUP_MARKER)?;
                fs::rename(final_path, &backup)
                    .with_context(|| format!("set aside {}", final_path.display()))?;
                if let Err(error) = fs::rename(&self.path, final_path) {
                    if let Err(restore) = fs::rename(&backup, final_path) {
                        bail!(
                            "install failed ({error}) and the previous result could not be restored \
                             from {} ({restore})",
                            backup.display()
                        );
                    }
                    return Err(error).with_context(|| format!("install {}", final_path.display()));
                }
                if let Err(error) = fs::remove_dir_all(&backup) {
                    eprintln!(
                        "release-check: could not remove the superseded result {}: {error}",
                        backup.display()
                    );
                }
                Ok(())
            }
        }
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        // Absent once installed; present after any failure, where it must not linger.
        if self.path.exists() {
            fs::remove_dir_all(&self.path).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("stutter-release-output-{name}-{}", unique_suffix()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn package(path: &Path, marker: &str) {
        fs::create_dir_all(path).unwrap();
        fs::write(path.join("marker"), marker).unwrap();
    }

    fn marker(path: &Path) -> String {
        fs::read_to_string(path.join("marker")).unwrap()
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn no_out_is_managed_and_out_is_caller_owned() {
        let root = Path::new("/repo");
        assert_eq!(
            Destination::resolve(root, None),
            Destination::Managed(PathBuf::from("/repo/target/release-check"))
        );
        assert_eq!(
            Destination::resolve(root, Some(Path::new("dist/x"))),
            Destination::Explicit(PathBuf::from("/repo/dist/x"))
        );
        assert_eq!(
            Destination::resolve(root, Some(Path::new("/abs/x"))),
            Destination::Explicit(PathBuf::from("/abs/x"))
        );
    }

    #[test]
    fn a_first_managed_install_renames_staging_into_place() {
        let root = scratch("first");
        let destination = Destination::resolve(&root, None);
        destination.preflight().unwrap();
        let staging = Staging::create(&destination).unwrap();
        package(staging.path(), "new");
        staging.install(&destination).unwrap();
        assert_eq!(marker(destination.path()), "new");
        assert_eq!(entries(&root.join("target")), ["release-check"]);
    }

    #[test]
    fn an_existing_managed_output_is_replaced_after_the_new_one_verified() {
        let root = scratch("replace");
        let destination = Destination::resolve(&root, None);
        package(destination.path(), "old");
        destination.preflight().unwrap();
        let staging = Staging::create(&destination).unwrap();
        package(staging.path(), "new");
        staging.install(&destination).unwrap();
        assert_eq!(marker(destination.path()), "new");
        assert_eq!(
            entries(&root.join("target")),
            ["release-check"],
            "neither the staging directory nor the backup remains"
        );
    }

    #[test]
    fn a_failed_managed_install_restores_the_previous_output() {
        let root = scratch("restore");
        let destination = Destination::resolve(&root, None);
        package(destination.path(), "old");
        let staging = Staging::create(&destination).unwrap();
        // Nothing was staged, so moving it into place fails after the old result was set aside.
        assert!(staging.install(&destination).is_err());
        assert_eq!(marker(destination.path()), "old");
        assert_eq!(entries(&root.join("target")), ["release-check"]);
    }

    #[test]
    fn an_existing_explicit_destination_is_rejected_and_left_alone() {
        let root = scratch("explicit");
        let destination = Destination::resolve(&root, Some(Path::new("out")));
        package(destination.path(), "mine");
        assert!(destination.preflight().is_err());
        let staging = Staging::create(&destination).unwrap();
        package(staging.path(), "new");
        assert!(staging.install(&destination).is_err());
        assert_eq!(marker(destination.path()), "mine");
        assert_eq!(entries(&root), ["out"], "the staging directory is cleaned");
    }

    #[test]
    fn a_fresh_explicit_destination_is_installed() {
        let root = scratch("explicit-fresh");
        let destination = Destination::resolve(&root, Some(Path::new("nested/out")));
        destination.preflight().unwrap();
        let staging = Staging::create(&destination).unwrap();
        package(staging.path(), "new");
        staging.install(&destination).unwrap();
        assert_eq!(marker(destination.path()), "new");
    }

    #[test]
    fn dropping_staging_after_a_failure_removes_it() {
        let root = scratch("abandon");
        let destination = Destination::resolve(&root, None);
        let staging = Staging::create(&destination).unwrap();
        package(staging.path(), "half-built");
        let path = staging.path().to_owned();
        drop(staging);
        assert!(!path.exists());
        assert_eq!(entries(&root.join("target")), Vec::<String>::new());
    }

    #[test]
    fn staging_left_by_a_dead_process_is_swept_but_a_live_one_is_kept() {
        let root = scratch("sweep");
        let destination = Destination::resolve(&root, None);
        let target = root.join("target");
        // Process ids above the kernel's pid_max cannot be alive.
        package(&target.join("release-check.staging-4294967295-1"), "dead");
        let live = target.join(format!("release-check.staging-{}-1", std::process::id()));
        package(&live, "live");
        package(&target.join("unrelated"), "unrelated");
        let staging = Staging::create(&destination).unwrap();
        drop(staging);
        assert!(live.exists());
        assert!(target.join("unrelated").exists());
        assert!(!target.join("release-check.staging-4294967295-1").exists());
    }

    #[test]
    fn release_package_keeps_refusing_an_existing_destination() {
        let root = scratch("no-clobber");
        let existing = root.join("package");
        package(&existing, "previous");
        let result = crate::release_package::package_release(
            &root,
            &crate::release_package::PackageArgs {
                binary: root.join("no-such-binary"),
                bpf_object: root.join("no-such-object"),
                out: existing.clone(),
            },
        );
        let message = format!("{:#}", result.unwrap_err());
        assert!(message.contains("already exists"), "{message}");
        assert_eq!(marker(&existing), "previous");
    }
}
