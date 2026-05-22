use std::{fs, path::Path};
use anyhow::Context;
use super::types::*;

pub(super) fn write_sysfs_value(path: &Path, value: &str) -> anyhow::Result<()> {
    fs::write(path, value).with_context(|| format!("failed to write {}", path.display()))
}

pub(super) fn read_irq_device_hint(irq_dir: &Path) -> anyhow::Result<String> {
    let actions_path = irq_dir.join("actions");
    if let Ok(value) = read_trimmed(&actions_path)
        && !value.is_empty()
    {
        return Ok(value);
    }

    let name_path = irq_dir.join("name");
    if let Ok(value) = read_trimmed(&name_path)
        && !value.is_empty()
    {
        return Ok(value);
    }

    anyhow::bail!(
        "neither {} nor {} contained a device hint",
        actions_path.display(),
        name_path.display()
    )
}

pub(super) fn read_trimmed(path: &Path) -> anyhow::Result<String> {
    fs::read_to_string(path)
        .map(|value| value.trim().to_owned())
        .with_context(|| format!("failed to read {}", path.display()))
}
pub(super) fn is_missing_task_error(err: &std::io::Error) -> bool {
    err.raw_os_error() == Some(libc::ESRCH)
}

pub(super) fn restore_summary_with_missing_skips(
    rollback_kind: impl Into<String>,
    restored_items: usize,
    skipped_missing: usize,
) -> RollbackRestoreSummary {
    let mut summary = RollbackRestoreSummary {
        rollback_kind: rollback_kind.into(),
        restored_items,
        skipped_items: skipped_missing,
        skipped_missing,
        skipped_identity_mismatch: 0,
        failed_items: 0,
        messages: Vec::new(),
    };

    if skipped_missing > 0 {
        summary
            .messages
            .push(format!("skipped_missing_tasks={skipped_missing}"));
    }

    summary
}

