use std::{
    fs::{self, OpenOptions},
    io,
    path::{Component, Path, PathBuf},
};

use anyhow::Context;

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
        anyhow::bail!("required cgroup file does not exist: {}", path.display());
    }

    OpenOptions::new()
        .write(true)
        .open(path)
        .with_context(|| format!("required cgroup file is not writable: {}", path.display()))?;

    Ok(())
}

pub(super) fn ensure_path_under_root(root: &Path, path: &Path) -> anyhow::Result<()> {
    if !path.starts_with(root) {
        anyhow::bail!(
            "target cgroup {} is outside cgroup root {}",
            path.display(),
            root.display()
        );
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
                anyhow::bail!(
                    "cgroup path must not contain parent traversal: {}",
                    path.display()
                )
            }
            Component::Prefix(_) => {
                anyhow::bail!(
                    "cgroup path must not contain platform prefix: {}",
                    path.display()
                )
            }
        }
    }

    Ok(normalized)
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
