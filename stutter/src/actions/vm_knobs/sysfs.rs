use std::{fs, fs::OpenOptions, path::Path};

use anyhow::Context;

use crate::actions::ActionBoundaryError;

pub(super) fn ensure_writable_file(path: &Path) -> anyhow::Result<()> {
    if !path.is_file() {
        return Err(ActionBoundaryError::MissingPath {
            action_kind: "vm_knobs",
            path: path.to_path_buf(),
        }
        .into());
    }

    OpenOptions::new()
        .write(true)
        .open(path)
        .with_context(|| format!("required VM knob file is not writable: {}", path.display()))?;

    Ok(())
}

pub(super) fn read_trimmed(path: &Path) -> anyhow::Result<String> {
    fs::read_to_string(path)
        .map(|value| value.trim().to_owned())
        .with_context(|| format!("failed to read {}", path.display()))
}

pub(super) fn write_trimmed(path: &Path, value: &str) -> anyhow::Result<()> {
    fs::write(path, value.trim())
        .with_context(|| format!("failed to write {:?} to {}", value.trim(), path.display()))
}
