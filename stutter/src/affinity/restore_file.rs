#[cfg(test)]
use std::collections::BTreeMap;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
#[cfg(test)]
use stutter_core::ids::{Pid, Tid};

#[cfg(test)]
use super::restore_record::RESTORE_SCHEMA_VERSION;
use super::{AffinityRecord, RestoreState, RestoreSummary, restore_record::restore_all};

pub fn default_restore_path() -> PathBuf {
    let mut base = env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.push(".local");
    base.push("state");
    base.push("stutter");
    base.push("last_affinity_restore.json");
    base
}

#[cfg(test)]
pub fn save_restore_state(path: &Path, records: &[AffinityRecord]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let state = RestoreState {
        schema_version: RESTORE_SCHEMA_VERSION,
        records: records.to_vec(),
    };
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, serde_json::to_vec_pretty(&state)?)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(test)]
pub fn save_merged_restore_state(
    path: &Path,
    records: &[AffinityRecord],
    force_overwrite: bool,
) -> anyhow::Result<()> {
    if force_overwrite || !path.exists() {
        return save_restore_state(path, records);
    }

    let existing = load_restore_state(path)?;
    let mut merged = BTreeMap::new();

    for record in existing.records {
        merged.insert(restore_merge_key(&record), record);
    }

    for record in records {
        let mut record = record.clone();
        if record.has_identity() {
            let legacy_key = RestoreMergeKey {
                tid: record.tid,
                process_pid: None,
                process_starttime_ticks: None,
                task_starttime_ticks: None,
            };
            if let Some(legacy) = merged.remove(&legacy_key)
                && legacy.applied_mask == record.original_mask
            {
                record.original_mask = legacy.original_mask;
            }
        }

        merged
            .entry(restore_merge_key(&record))
            .and_modify(|existing| {
                if record.applied_mask == existing.original_mask {
                    existing.original_mask = record.original_mask.clone();
                } else {
                    existing.applied_mask = record.applied_mask.clone();
                }
            })
            .or_insert(record);
    }

    let records = merged.into_values().collect::<Vec<_>>();
    save_restore_state(path, &records)
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RestoreMergeKey {
    tid: Tid,
    process_pid: Option<Pid>,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
}

#[cfg(test)]
fn restore_merge_key(record: &AffinityRecord) -> RestoreMergeKey {
    RestoreMergeKey {
        tid: record.tid,
        process_pid: record.process_pid,
        process_starttime_ticks: record.process_starttime_ticks,
        task_starttime_ticks: record.task_starttime_ticks,
    }
}

pub fn read_restore_records(path: &Path) -> anyhow::Result<Vec<AffinityRecord>> {
    let state = load_restore_state(path)?;
    Ok(state.records)
}

pub fn load_restore_state(path: &Path) -> anyhow::Result<RestoreState> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read restore file {}", path.display()))?;
    let state = serde_json::from_str(&data)
        .with_context(|| format!("failed to parse restore file {}", path.display()))?;
    Ok(state)
}

pub fn restore_saved(path: &Path) -> anyhow::Result<RestoreSummary> {
    let state = load_restore_state(path)?;
    let (summary, errors) = restore_all(&state.records);

    if !errors.is_empty() {
        anyhow::bail!(
            "failed to restore {} affinity record(s); restore file kept at {}",
            errors.len(),
            path.display()
        );
    }

    fs::remove_file(path).ok();
    Ok(summary)
}
