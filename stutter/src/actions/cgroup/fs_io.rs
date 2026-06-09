use std::{
    fs::{self, OpenOptions},
    io,
    path::{Component, Path, PathBuf},
};

use anyhow::Context;

use crate::actions::ActionBoundaryError;

pub(super) trait CgroupFileWriter {
    fn write_trimmed(&mut self, path: &Path, value: &str) -> anyhow::Result<()>;
}

pub(super) struct FsCgroupFileWriter;

impl CgroupFileWriter for FsCgroupFileWriter {
    fn write_trimmed(&mut self, path: &Path, value: &str) -> anyhow::Result<()> {
        write_trimmed(path, value)
    }
}

pub(super) fn ensure_writable_file(path: &Path) -> anyhow::Result<()> {
    if !path.is_file() {
        return Err(ActionBoundaryError::MissingPath {
            action_kind: "cgroup",
            path: path.to_path_buf(),
        }
        .into());
    }

    OpenOptions::new()
        .write(true)
        .open(path)
        .with_context(|| format!("required cgroup file is not writable: {}", path.display()))?;

    Ok(())
}

pub(super) fn ensure_path_under_root(root: &Path, path: &Path) -> anyhow::Result<()> {
    if !path.starts_with(root) {
        return Err(ActionBoundaryError::PathNotAllowed {
            action_kind: "cgroup",
            path: path.to_path_buf(),
            reason: format!("target cgroup is outside cgroup root {}", root.display()),
        }
        .into());
    }

    Ok(())
}

pub(super) fn normalize_cgroup_path(path: &Path) -> anyhow::Result<PathBuf> {
    let mut normalized = PathBuf::from("/");

    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(ActionBoundaryError::PathNotAllowed {
                    action_kind: "cgroup",
                    path: path.to_path_buf(),
                    reason: "cgroup path must not contain parent traversal".to_owned(),
                }
                .into());
            }
            Component::Prefix(_) => {
                return Err(ActionBoundaryError::PathNotAllowed {
                    action_kind: "cgroup",
                    path: path.to_path_buf(),
                    reason: "cgroup path must not contain platform prefix".to_owned(),
                }
                .into());
            }
        }
    }

    Ok(normalized)
}

pub(crate) fn resolve_cgroup_fs_path(
    cgroup_root: &Path,
    cgroup_path: &Path,
) -> anyhow::Result<PathBuf> {
    if cgroup_path.starts_with(cgroup_root) {
        let cgroup_relative = cgroup_path.strip_prefix(cgroup_root).with_context(|| {
            format!(
                "failed to strip cgroup root {} from {}",
                cgroup_root.display(),
                cgroup_path.display()
            )
        })?;
        let normalized = normalize_cgroup_path(cgroup_relative)?;
        return Ok(cgroup_root.join(strip_cgroup_leading_slash(&normalized)));
    }

    let normalized = normalize_cgroup_path(cgroup_path)?;
    Ok(cgroup_root.join(strip_cgroup_leading_slash(&normalized)))
}

pub(super) fn strip_cgroup_leading_slash(path: &Path) -> &Path {
    path.strip_prefix("/").unwrap_or(path)
}

pub(super) fn read_trimmed(path: &Path) -> anyhow::Result<String> {
    fs::read_to_string(path)
        .map(|value| value.trim().to_owned())
        .with_context(|| format!("failed to read {}", path.display()))
}

pub(super) fn read_optional_trimmed(path: &Path) -> anyhow::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(value) => Ok(Some(value.trim().to_owned())),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err).with_context(|| format!("failed to read {}", path.display())),
    }
}

pub(super) fn write_trimmed(path: &Path, value: &str) -> anyhow::Result<()> {
    fs::write(path, value.trim())
        .with_context(|| format!("failed to write {} to {}", value.trim(), path.display()))
}
