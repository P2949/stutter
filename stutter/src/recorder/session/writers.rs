use std::{fs, io::Write, path::PathBuf};

use anyhow::Context;
use serde::Serialize;

use crate::recorder::{NdjsonWriter, SyncTracker};

pub(super) fn write_json<T: ?Sized + Serialize>(
    path: PathBuf,
    value: &T,
    sync_tracker: &mut SyncTracker,
) -> anyhow::Result<()> {
    let file_name = path
        .file_name()
        .and_then(|name| {
            if name.is_empty() {
                None
            } else {
                Some(name.to_string_lossy())
            }
        })
        .ok_or_else(|| anyhow::anyhow!("JSON destination has no file name: {}", path.display()))?;
    let tmp_path = path.with_file_name(format!("{file_name}.tmp"));
    let mut file = fs::File::create(&tmp_path)
        .with_context(|| format!("failed to create temp JSON {}", tmp_path.display()))?;
    file.write_all(&serde_json::to_vec_pretty(value)?)
        .with_context(|| format!("failed to write temp JSON {}", tmp_path.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("failed to finalize temp JSON {}", tmp_path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to sync temp JSON {}", tmp_path.display()))?;
    drop(file);
    fs::rename(&tmp_path, &path)
        .with_context(|| format!("failed to rename temp JSON {}", tmp_path.display()))?;

    sync_tracker.sync_parent_once(&path)?;

    Ok(())
}

pub(super) fn write_json_stream<T: Serialize>(
    path: PathBuf,
    values: &[T],
    sync_tracker: &mut SyncTracker,
) -> anyhow::Result<()> {
    let mut writer = NdjsonWriter::create(path.clone())?;
    for value in values {
        writer.push(value)?;
    }
    writer.finish()?;

    sync_tracker.sync_parent_once(&path)?;

    Ok(())
}
