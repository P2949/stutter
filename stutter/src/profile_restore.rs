use std::{
    collections::BTreeMap,
    env, fs, io,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::affinity::{self, AffinityRecord, RestoreRecordStatus};

pub const PROFILE_RESTORE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProfileRestoreState {
    pub schema_version: u32,
    #[serde(default)]
    pub affinity_records: Vec<AffinityRecord>,
    #[serde(default)]
    pub nice_records: Vec<NiceRestoreRecordV2>,
    #[serde(default)]
    pub ionice_records: Vec<IoPrioRestoreRecordV2>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NiceRestoreRecordV2 {
    pub tid: u32,
    #[serde(default)]
    pub process_pid: Option<u32>,
    #[serde(default)]
    pub process_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub task_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub comm: Option<String>,
    pub original_nice: i32,
    pub applied_nice: i32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IoPrioRestoreRecordV2 {
    pub tid: u32,
    #[serde(default)]
    pub process_pid: Option<u32>,
    #[serde(default)]
    pub process_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub task_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub comm: Option<String>,
    pub original_ioprio: i32,
    pub applied_ioprio: i32,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProfileRestoreSummary {
    pub affinity: usize,
    pub nice: usize,
    pub ionice: usize,
    pub skipped_dead: usize,
    pub skipped_identity_mismatch: usize,
    pub legacy_unverified: usize,
    pub errors: usize,
}

impl ProfileRestoreSummary {
    pub fn restored_total(&self) -> usize {
        self.affinity + self.nice + self.ionice
    }
}

pub fn default_restore_path() -> PathBuf {
    let mut base = env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.push(".local");
    base.push("state");
    base.push("stutter");
    base.push("last_profile_restore.json");
    base
}

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

pub fn load_restore_state(path: &Path) -> anyhow::Result<ProfileRestoreState> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read profile restore file {}", path.display()))?;
    let state = serde_json::from_str(&data)
        .with_context(|| format!("failed to parse profile restore file {}", path.display()))?;
    Ok(state)
}

pub fn restore_saved(path: &Path) -> anyhow::Result<ProfileRestoreSummary> {
    let state = load_restore_state(path)?;
    let (summary, errors) = restore_all(&state);

    if !errors.is_empty() {
        anyhow::bail!(
            "failed to restore {} profile record(s); restore file kept at {}",
            errors.len(),
            path.display()
        );
    }

    fs::remove_file(path).ok();
    Ok(summary)
}

pub fn restore_all(state: &ProfileRestoreState) -> (ProfileRestoreSummary, Vec<anyhow::Error>) {
    restore_all_at(Path::new("/proc"), state)
}

fn restore_all_at(
    proc_root: &Path,
    state: &ProfileRestoreState,
) -> (ProfileRestoreSummary, Vec<anyhow::Error>) {
    restore_all_at_with_ops(
        proc_root,
        state,
        affinity::set_affinity_raw,
        |tid, nice| crate::actions::nice::set_task_nice(tid, nice).map_err(anyhow_to_io_error),
        |tid, ioprio| {
            crate::actions::ioprio::set_task_ioprio(tid, ioprio).map_err(anyhow_to_io_error)
        },
    )
}

fn restore_all_at_with_ops<FA, FN, FI>(
    proc_root: &Path,
    state: &ProfileRestoreState,
    mut set_affinity: FA,
    mut set_nice: FN,
    mut set_ioprio: FI,
) -> (ProfileRestoreSummary, Vec<anyhow::Error>)
where
    FA: FnMut(u32, &crate::affinity::CpuMask) -> io::Result<()>,
    FN: FnMut(u32, i32) -> io::Result<()>,
    FI: FnMut(u32, i32) -> io::Result<()>,
{
    let mut summary = ProfileRestoreSummary::default();
    let mut errors = Vec::new();

    for record in &state.affinity_records {
        match restore_record_status(
            proc_root,
            record.tid,
            record.process_pid,
            record.process_starttime_ticks,
            record.task_starttime_ticks,
        ) {
            Ok(RestoreRecordStatus::Verified) => {}
            Ok(RestoreRecordStatus::LegacyUnverified) => {
                summary.legacy_unverified += 1;
                log::warn!(
                    "profile_restore_affinity_missing_identity tid={}; restoring by numeric TID only for legacy affinity record",
                    record.tid
                );
            }
            Ok(RestoreRecordStatus::Dead) => {
                summary.skipped_dead += 1;
                continue;
            }
            Ok(RestoreRecordStatus::IdentityMismatch) => {
                summary.skipped_identity_mismatch += 1;
                continue;
            }
            Err(err) => {
                summary.errors += 1;
                errors.push(err.into());
                continue;
            }
        }

        match set_affinity(record.tid, &record.original_mask) {
            Ok(()) => summary.affinity += 1,
            Err(err) if err.raw_os_error() == Some(libc::ESRCH) => summary.skipped_dead += 1,
            Err(err) => {
                summary.errors += 1;
                errors.push(anyhow::anyhow!(
                    "failed to restore affinity for TID {}: {err}",
                    record.tid
                ));
            }
        }
    }

    for record in &state.nice_records {
        if !restore_priority_identity(
            proc_root,
            record.tid,
            record.identity_tuple(),
            &mut summary,
            &mut errors,
        ) {
            continue;
        }

        match set_nice(record.tid, record.original_nice) {
            Ok(()) => summary.nice += 1,
            Err(err) if err.raw_os_error() == Some(libc::ESRCH) => summary.skipped_dead += 1,
            Err(err) => {
                summary.errors += 1;
                errors.push(anyhow::anyhow!(
                    "failed to restore nice={} for TID {}: {err}",
                    record.original_nice,
                    record.tid
                ));
            }
        }
    }

    for record in &state.ionice_records {
        if !restore_priority_identity(
            proc_root,
            record.tid,
            record.identity_tuple(),
            &mut summary,
            &mut errors,
        ) {
            continue;
        }

        match set_ioprio(record.tid, record.original_ioprio) {
            Ok(()) => summary.ionice += 1,
            Err(err) if err.raw_os_error() == Some(libc::ESRCH) => summary.skipped_dead += 1,
            Err(err) => {
                summary.errors += 1;
                errors.push(anyhow::anyhow!(
                    "failed to restore I/O priority={} for TID {}: {err}",
                    record.original_ioprio,
                    record.tid
                ));
            }
        }
    }

    (summary, errors)
}

fn restore_priority_identity(
    proc_root: &Path,
    tid: u32,
    identity: RestoreIdentity,
    summary: &mut ProfileRestoreSummary,
    errors: &mut Vec<anyhow::Error>,
) -> bool {
    match restore_record_status(
        proc_root,
        tid,
        identity.process_pid,
        identity.process_starttime_ticks,
        identity.task_starttime_ticks,
    ) {
        Ok(RestoreRecordStatus::Verified) => true,
        Ok(RestoreRecordStatus::Dead) => {
            summary.skipped_dead += 1;
            false
        }
        Ok(RestoreRecordStatus::IdentityMismatch | RestoreRecordStatus::LegacyUnverified) => {
            summary.skipped_identity_mismatch += 1;
            false
        }
        Err(err) => {
            summary.errors += 1;
            errors.push(err.into());
            false
        }
    }
}

fn restore_record_status(
    proc_root: &Path,
    tid: u32,
    process_pid: Option<u32>,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
) -> io::Result<RestoreRecordStatus> {
    affinity::restore_identity_status_at(
        proc_root,
        tid,
        process_pid,
        process_starttime_ticks,
        task_starttime_ticks,
    )
}

#[derive(Clone, Copy)]
struct RestoreIdentity {
    process_pid: Option<u32>,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
}

impl NiceRestoreRecordV2 {
    fn identity_tuple(&self) -> RestoreIdentity {
        RestoreIdentity {
            process_pid: self.process_pid,
            process_starttime_ticks: self.process_starttime_ticks,
            task_starttime_ticks: self.task_starttime_ticks,
        }
    }
}

impl IoPrioRestoreRecordV2 {
    fn identity_tuple(&self) -> RestoreIdentity {
        RestoreIdentity {
            process_pid: self.process_pid,
            process_starttime_ticks: self.process_starttime_ticks,
            task_starttime_ticks: self.task_starttime_ticks,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct RestoreMergeKey {
    tid: u32,
    process_pid: Option<u32>,
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

fn anyhow_to_io_error(err: anyhow::Error) -> io::Error {
    if let Some(io_err) = err.downcast_ref::<io::Error>() {
        if let Some(raw) = io_err.raw_os_error() {
            return io::Error::from_raw_os_error(raw);
        }
        return io::Error::new(io_err.kind(), io_err.to_string());
    }
    io::Error::other(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restore_nice_with_matching_identity() {
        let dir = temp_dir("profile-restore-nice-match");
        write_fake_task_stat(&dir, 10, 11, 100, 111);
        let state = ProfileRestoreState {
            schema_version: PROFILE_RESTORE_SCHEMA_VERSION,
            nice_records: vec![nice_record(11, 10, 100, 111, 5, 10)],
            ..ProfileRestoreState::default()
        };
        let mut restored = Vec::new();

        let (summary, errors) = restore_all_at_with_ops(
            &dir,
            &state,
            |_, _| Ok(()),
            |tid, nice| {
                restored.push((tid, nice));
                Ok(())
            },
            |_, _| Ok(()),
        );

        assert!(errors.is_empty());
        assert_eq!(summary.nice, 1);
        assert_eq!(restored, vec![(11, 5)]);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn skip_nice_restore_on_tid_reuse() {
        let dir = temp_dir("profile-restore-nice-reuse");
        write_fake_task_stat(&dir, 10, 11, 100, 222);
        let state = ProfileRestoreState {
            schema_version: PROFILE_RESTORE_SCHEMA_VERSION,
            nice_records: vec![nice_record(11, 10, 100, 111, 5, 10)],
            ..ProfileRestoreState::default()
        };
        let mut restored = Vec::new();

        let (summary, errors) = restore_all_at_with_ops(
            &dir,
            &state,
            |_, _| Ok(()),
            |tid, nice| {
                restored.push((tid, nice));
                Ok(())
            },
            |_, _| Ok(()),
        );

        assert!(errors.is_empty());
        assert_eq!(summary.nice, 0);
        assert_eq!(summary.skipped_identity_mismatch, 1);
        assert!(restored.is_empty());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn restore_ionice_with_matching_identity() {
        let dir = temp_dir("profile-restore-ionice-match");
        write_fake_task_stat(&dir, 10, 11, 100, 111);
        let state = ProfileRestoreState {
            schema_version: PROFILE_RESTORE_SCHEMA_VERSION,
            ionice_records: vec![ionice_record(11, 10, 100, 111, 0, 16388)],
            ..ProfileRestoreState::default()
        };
        let mut restored = Vec::new();

        let (summary, errors) = restore_all_at_with_ops(
            &dir,
            &state,
            |_, _| Ok(()),
            |_, _| Ok(()),
            |tid, ioprio| {
                restored.push((tid, ioprio));
                Ok(())
            },
        );

        assert!(errors.is_empty());
        assert_eq!(summary.ionice, 1);
        assert_eq!(restored, vec![(11, 0)]);
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn skip_ionice_restore_on_tid_reuse() {
        let dir = temp_dir("profile-restore-ionice-reuse");
        write_fake_task_stat(&dir, 10, 11, 999, 111);
        let state = ProfileRestoreState {
            schema_version: PROFILE_RESTORE_SCHEMA_VERSION,
            ionice_records: vec![ionice_record(11, 10, 100, 111, 0, 16388)],
            ..ProfileRestoreState::default()
        };

        let (summary, errors) =
            restore_all_at_with_ops(&dir, &state, |_, _| Ok(()), |_, _| Ok(()), |_, _| Ok(()));

        assert!(errors.is_empty());
        assert_eq!(summary.ionice, 0);
        assert_eq!(summary.skipped_identity_mismatch, 1);
        fs::remove_dir_all(dir).ok();
    }

    fn nice_record(
        tid: u32,
        process_pid: u32,
        process_starttime_ticks: u64,
        task_starttime_ticks: u64,
        original_nice: i32,
        applied_nice: i32,
    ) -> NiceRestoreRecordV2 {
        NiceRestoreRecordV2 {
            tid,
            process_pid: Some(process_pid),
            process_starttime_ticks: Some(process_starttime_ticks),
            task_starttime_ticks: Some(task_starttime_ticks),
            comm: Some("task".to_owned()),
            original_nice,
            applied_nice,
        }
    }

    fn ionice_record(
        tid: u32,
        process_pid: u32,
        process_starttime_ticks: u64,
        task_starttime_ticks: u64,
        original_ioprio: i32,
        applied_ioprio: i32,
    ) -> IoPrioRestoreRecordV2 {
        IoPrioRestoreRecordV2 {
            tid,
            process_pid: Some(process_pid),
            process_starttime_ticks: Some(process_starttime_ticks),
            task_starttime_ticks: Some(task_starttime_ticks),
            comm: Some("task".to_owned()),
            original_ioprio,
            applied_ioprio,
        }
    }

    fn write_fake_task_stat(
        proc_root: &Path,
        process_pid: u32,
        tid: u32,
        process_starttime: u64,
        task_starttime: u64,
    ) {
        let process_dir = proc_root.join(process_pid.to_string());
        fs::create_dir_all(process_dir.join("task").join(tid.to_string())).unwrap();
        fs::write(
            process_dir.join("stat"),
            fake_stat("process", process_starttime),
        )
        .unwrap();
        fs::write(
            process_dir.join("task").join(tid.to_string()).join("stat"),
            fake_stat("task", task_starttime),
        )
        .unwrap();
    }

    fn fake_stat(comm: &str, starttime: u64) -> String {
        let mut fields = vec!["0".to_owned(); 18];
        fields.push(starttime.to_string());
        format!("1 ({comm}) S {}\n", fields.join(" "))
    }

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
