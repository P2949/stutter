use std::{fs, fs::OpenOptions, path::Path};

use anyhow::Context;

use crate::actions::ActionBoundaryError;

pub(super) fn ensure_writable_file(path: &Path) -> anyhow::Result<()> {
    if !path.is_file() {
        return Err(ActionBoundaryError::MissingPath {
            action_kind: "irq_affinity",
            path: path.to_path_buf(),
        }
        .into());
    }

    OpenOptions::new().write(true).open(path).with_context(|| {
        format!(
            "required IRQ affinity file is not writable: {}",
            path.display()
        )
    })?;

    Ok(())
}

pub(super) fn read_irq_device_hint(irq_dir: &Path) -> anyhow::Result<String> {
    let actions_path = irq_dir.join("actions");
    if let Ok(actions) = read_trimmed(&actions_path)
        && !actions.is_empty()
    {
        return Ok(actions);
    }

    let name_path = irq_dir.join("name");
    if let Ok(name) = read_trimmed(&name_path)
        && !name.is_empty()
    {
        return Ok(name);
    }

    return Err(ActionBoundaryError::InvalidValue {
        action_kind: "irq_affinity",
        field: "device_hint".to_owned(),
        reason: format!(
            "neither {} nor {} contained a device hint",
            actions_path.display(),
            name_path.display()
        ),
    }
    .into());
}

pub(super) fn normalize_affinity(value: &str) -> String {
    value
        .trim()
        .split(',')
        .map(|part| {
            let trimmed = part.trim_start_matches('0');
            if trimmed.is_empty() {
                "0".to_owned()
            } else {
                trimmed.to_ascii_lowercase()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

pub(super) fn read_trimmed(path: &Path) -> anyhow::Result<String> {
    fs::read_to_string(path)
        .map(|value| value.trim().to_owned())
        .with_context(|| format!("failed to read {}", path.display()))
}

pub(super) fn write_trimmed(path: &Path, value: &str) -> anyhow::Result<()> {
    fs::write(path, value.trim())
        .with_context(|| format!("failed to write {} to {}", value.trim(), path.display()))
}
