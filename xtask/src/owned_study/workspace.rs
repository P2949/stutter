//! The study-owned scratch the cold builds run in, and the checks that make cleanup safe.
//!
//! The harness never deletes anything it cannot prove it created. Builds write to
//! `<study>/workspace/target` (via `CARGO_TARGET_DIR`), a directory inside a `0700` study root that
//! carries an ownership marker made with `create_new`. The source tree is only ever READ: it must be
//! a clean git checkout whose tree object equals the frozen identity, reached without symlink
//! aliases and outside the study directory. Anything else is `UnsafeStudyWorkspace` before a hog or
//! a build starts.
use std::{
    fmt, fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};
use stutter::api::workload_progress::FixedWorkIdentity;

#[derive(Debug, PartialEq, Eq)]
pub struct UnsafeStudyWorkspace(pub String);

impl fmt::Display for UnsafeStudyWorkspace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UnsafeStudyWorkspace: {}", self.0)
    }
}

impl std::error::Error for UnsafeStudyWorkspace {}

fn unsafe_because<T>(why: impl Into<String>) -> Result<T, UnsafeStudyWorkspace> {
    Err(UnsafeStudyWorkspace(why.into()))
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Marker {
    study_id: String,
    manifest_sha256: String,
    uid: u32,
}

#[derive(Debug)]
pub struct Workspace {
    pub source: PathBuf,
    /// The verified git tree object of the source, bound into every record's context.
    pub tree: String,
    root: PathBuf,
    marker: Marker,
}

fn git(source: &Path, args: &[&str]) -> Result<String, UnsafeStudyWorkspace> {
    match Command::new("git")
        .arg("-C")
        .arg(source)
        .args(args)
        .output()
    {
        Ok(out) if out.status.success() => {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
        }
        Ok(out) => unsafe_because(format!(
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
        Err(error) => unsafe_because(format!("git is unavailable: {error}")),
    }
}

fn is_plain_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir() && !meta.file_type().is_symlink())
}

fn effective_uid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// The source must be exactly the frozen tree and unmodified, so a build cannot silently differ.
fn verify_source(source: &Path, expected_tree: &str) -> Result<(), UnsafeStudyWorkspace> {
    if git(source, &["rev-parse", "HEAD^{tree}"])? != expected_tree {
        return unsafe_because("the source tree is not the frozen identity's tree");
    }
    if !git(source, &["status", "--porcelain", "--untracked-files=no"])?.is_empty() {
        return unsafe_because("the source tree has modified tracked files");
    }
    Ok(())
}

pub fn prepare(
    out: &Path,
    study_id: &str,
    manifest_sha256: &str,
    identity: &FixedWorkIdentity,
) -> Result<Workspace, UnsafeStudyWorkspace> {
    let Ok(meta) = fs::symlink_metadata(out) else {
        return unsafe_because("the study directory does not exist");
    };
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return unsafe_because("the study directory is not a plain directory");
    }
    if meta.uid() != effective_uid() || meta.permissions().mode() & 0o777 != 0o700 {
        return unsafe_because("the study directory is not private to this user (0700)");
    }
    let source = PathBuf::from(&identity.working_directory);
    let tree = match identity.source_tree.split_once(':') {
        Some(("git", tree)) if tree.len() == 40 => tree.to_owned(),
        _ => return unsafe_because("the identity must name a git tree (git:<40 hex>)"),
    };
    match fs::canonicalize(&source) {
        Ok(real) if real == source && source.is_absolute() => {}
        _ => {
            return unsafe_because(
                "the source path is relative, missing or reached through a symlink",
            );
        }
    }
    let real_out = fs::canonicalize(out).map_err(|e| UnsafeStudyWorkspace(e.to_string()))?;
    if real_out.starts_with(&source) || source.starts_with(&real_out) {
        return unsafe_because("the study directory and the source tree overlap");
    }
    if !source.join("Cargo.toml").is_file() {
        return unsafe_because("the source is not a cargo tree");
    }
    verify_source(&source, &tree)?;
    let root = real_out.join("workspace");
    let marker = Marker {
        study_id: study_id.to_owned(),
        manifest_sha256: manifest_sha256.to_owned(),
        uid: effective_uid(),
    };
    if fs::symlink_metadata(&root).is_err() {
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .map_err(|e| UnsafeStudyWorkspace(format!("cannot create the workspace: {e}")))?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.join(".owned-study"))
            .map_err(|e| {
                UnsafeStudyWorkspace(format!("cannot create the ownership marker: {e}"))
            })?;
        file.write_all(&serde_json::to_vec(&marker).unwrap_or_default())
            .and_then(|()| file.sync_all())
            .map_err(|e| UnsafeStudyWorkspace(e.to_string()))?;
    }
    let workspace = Workspace {
        source,
        tree,
        root,
        marker,
    };
    workspace.check_ownership()?;
    Ok(workspace)
}

impl Workspace {
    pub fn target_dir(&self) -> PathBuf {
        self.root.join("target")
    }

    fn check_ownership(&self) -> Result<(), UnsafeStudyWorkspace> {
        if !is_plain_dir(&self.root) {
            return unsafe_because("the workspace is not a plain directory");
        }
        let marker_path = self.root.join(".owned-study");
        let plain_file = fs::symlink_metadata(&marker_path).is_ok_and(|m| m.is_file());
        let found: Option<Marker> = plain_file
            .then(|| fs::read(&marker_path).ok())
            .flatten()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok());
        if found.as_ref() != Some(&self.marker) {
            return unsafe_because(
                "the workspace ownership marker is missing or belongs to another study",
            );
        }
        Ok(())
    }

    /// Removes the previous build for a cold start, and nothing else.
    pub fn reset_target(&self) -> Result<(), UnsafeStudyWorkspace> {
        self.check_ownership()?;
        let target = self.target_dir();
        match fs::symlink_metadata(&target) {
            Err(_) => return Ok(()),
            Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {}
            Ok(_) => return unsafe_because("the workspace target is a symlink or not a directory"),
        }
        match (fs::canonicalize(&target), fs::canonicalize(&self.root)) {
            (Ok(real), Ok(root)) if real.parent() == Some(root.as_path()) => {}
            _ => {
                return unsafe_because(
                    "the workspace target is not directly inside the study workspace",
                );
            }
        }
        fs::remove_dir_all(&target)
            .map_err(|e| UnsafeStudyWorkspace(format!("cleanup failed: {e}")))
    }

    /// The source is still the frozen, clean tree after a slot (a build must only read it).
    pub fn verify_unchanged_source(&self) -> Result<(), UnsafeStudyWorkspace> {
        verify_source(&self.source, &self.tree)
    }
}
