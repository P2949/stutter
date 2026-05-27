use std::{collections::BTreeMap, fs, path::Path};

use stutter_core::ids::{Pid, Tid};

use super::{
    load::load_restore_state,
    model::{
        IoPrioRestoreRecordV2, NiceRestoreRecordV2, PROFILE_RESTORE_SCHEMA_VERSION,
        ProfileRestoreState,
    },
};
use crate::affinity::AffinityRecord;

pub fn save_merged_restore_state(
    path: &Path,
    affinity_records: &[AffinityRecord],
    nice_records: &[NiceRestoreRecordV2],
    ionice_records: &[IoPrioRestoreRecordV2],
    force_overwrite: bool,
) -> anyhow::Result<()> {
    let new_state = ProfileRestoreState {
        schema_version: PROFILE_RESTORE_SCHEMA_VERSION,
        affinity_records: affinity_records.to_vec(),
        nice_records: nice_records.to_vec(),
        ionice_records: ionice_records.to_vec(),
    };

    if force_overwrite || !path.exists() {
        return save_restore_state(path, &new_state);
    }

    let existing = load_restore_state(path)?;
    save_restore_state(path, &merge_restore_states(existing, new_state))
}

pub fn save_restore_state(path: &Path, state: &ProfileRestoreState) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut state = state.clone();
    state.schema_version = PROFILE_RESTORE_SCHEMA_VERSION;
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, serde_json::to_vec_pretty(&state)?)?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RestoreMergeKey {
    tid: Tid,
    process_pid: Option<Pid>,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
}

fn merge_restore_states(
    existing: ProfileRestoreState,
    new_state: ProfileRestoreState,
) -> ProfileRestoreState {
    ProfileRestoreState {
        schema_version: PROFILE_RESTORE_SCHEMA_VERSION,
        affinity_records: merge_affinity_records(
            existing.affinity_records,
            new_state.affinity_records,
        ),
        nice_records: merge_nice_records(existing.nice_records, new_state.nice_records),
        ionice_records: merge_ionice_records(existing.ionice_records, new_state.ionice_records),
    }
}

fn merge_affinity_records(
    existing: Vec<AffinityRecord>,
    new_records: Vec<AffinityRecord>,
) -> Vec<AffinityRecord> {
    let mut merged = BTreeMap::new();
    for record in existing {
        merged.insert(affinity_key(&record), record);
    }

    for record in new_records {
        merged
            .entry(affinity_key(&record))
            .and_modify(|existing: &mut AffinityRecord| {
                if record.applied_mask == existing.original_mask {
                    existing.original_mask = record.original_mask.clone();
                } else {
                    existing.applied_mask = record.applied_mask.clone();
                }
            })
            .or_insert(record);
    }

    merged.into_values().collect()
}

fn merge_nice_records(
    existing: Vec<NiceRestoreRecordV2>,
    new_records: Vec<NiceRestoreRecordV2>,
) -> Vec<NiceRestoreRecordV2> {
    let mut merged = BTreeMap::new();
    for record in existing {
        merged.insert(nice_key(&record), record);
    }

    for record in new_records {
        merged
            .entry(nice_key(&record))
            .and_modify(|existing: &mut NiceRestoreRecordV2| {
                if record.applied_nice == existing.original_nice {
                    existing.original_nice = record.original_nice;
                } else {
                    existing.applied_nice = record.applied_nice;
                }
            })
            .or_insert(record);
    }

    merged.into_values().collect()
}

fn merge_ionice_records(
    existing: Vec<IoPrioRestoreRecordV2>,
    new_records: Vec<IoPrioRestoreRecordV2>,
) -> Vec<IoPrioRestoreRecordV2> {
    let mut merged = BTreeMap::new();
    for record in existing {
        merged.insert(ionice_key(&record), record);
    }

    for record in new_records {
        merged
            .entry(ionice_key(&record))
            .and_modify(|existing: &mut IoPrioRestoreRecordV2| {
                if record.applied_ioprio == existing.original_ioprio {
                    existing.original_ioprio = record.original_ioprio;
                } else {
                    existing.applied_ioprio = record.applied_ioprio;
                }
            })
            .or_insert(record);
    }

    merged.into_values().collect()
}

fn affinity_key(record: &AffinityRecord) -> RestoreMergeKey {
    RestoreMergeKey {
        tid: record.tid,
        process_pid: record.process_pid,
        process_starttime_ticks: record.process_starttime_ticks,
        task_starttime_ticks: record.task_starttime_ticks,
    }
}

fn nice_key(record: &NiceRestoreRecordV2) -> RestoreMergeKey {
    RestoreMergeKey {
        tid: record.tid,
        process_pid: record.process_pid,
        process_starttime_ticks: record.process_starttime_ticks,
        task_starttime_ticks: record.task_starttime_ticks,
    }
}

fn ionice_key(record: &IoPrioRestoreRecordV2) -> RestoreMergeKey {
    RestoreMergeKey {
        tid: record.tid,
        process_pid: record.process_pid,
        process_starttime_ticks: record.process_starttime_ticks,
        task_starttime_ticks: record.task_starttime_ticks,
    }
}
