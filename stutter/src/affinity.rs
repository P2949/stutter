#[cfg(test)]
use std::collections::BTreeMap;
use std::{
    env, fmt, fs, io,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error as DeError, Visitor},
};
use stutter_core::ids::{Pid, Tid};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct CpuMask {
    words: Vec<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AffinityRecord {
    pub tid: Tid,
    #[serde(default)]
    pub process_pid: Option<Pid>,
    #[serde(default)]
    pub process_starttime_ticks: Option<u64>,
    #[serde(default)]
    pub task_starttime_ticks: Option<u64>,
    pub original_mask: CpuMask,
    pub applied_mask: CpuMask,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RestoreState {
    pub schema_version: u32,
    pub records: Vec<AffinityRecord>,
}

#[cfg(test)]
pub const RESTORE_SCHEMA_VERSION: u32 = 3;

impl CpuMask {
    pub fn parse(value: &str) -> anyhow::Result<Self> {
        let mut mask = Self::empty();

        for part in value
            .split(',')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            let (start, end) = match part.split_once('-') {
                Some((start, end)) => (parse_cpu(start)?, parse_cpu(end)?),
                None => {
                    let cpu = parse_cpu(part)?;
                    (cpu, cpu)
                }
            };

            if start > end {
                anyhow::bail!("invalid CPU range {part}: start is greater than end");
            }

            for cpu in start..=end {
                mask.set(cpu);
            }
        }

        if mask.is_empty() {
            anyhow::bail!("CPU mask must contain at least one CPU");
        }

        Ok(mask)
    }

    pub fn online_cpus() -> anyhow::Result<Self> {
        let data = std::fs::read_to_string("/sys/devices/system/cpu/online")
            .context("failed to read /sys/devices/system/cpu/online")?;
        Self::parse(data.trim()).context("failed to parse online CPUs mask")
    }

    pub fn is_subset_of(&self, other: &Self) -> bool {
        self.words.iter().enumerate().all(|(i, word)| {
            let other_word = other.words.get(i).copied().unwrap_or(0);
            word & !other_word == 0
        })
    }

    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|word| *word == 0)
    }

    pub fn to_range_string(&self) -> String {
        let cpus = self.cpus();
        let mut ranges = Vec::new();
        let mut idx = 0;

        while idx < cpus.len() {
            let start = cpus[idx];
            let mut end = start;
            idx += 1;

            while idx < cpus.len() && cpus[idx] == end + 1 {
                end = cpus[idx];
                idx += 1;
            }

            if start == end {
                ranges.push(start.to_string());
            } else {
                ranges.push(format!("{start}-{end}"));
            }
        }

        ranges.join(",")
    }

    fn empty() -> Self {
        Self { words: Vec::new() }
    }

    fn from_legacy_bits(bits: u64) -> Self {
        if bits == 0 {
            Self::empty()
        } else {
            Self { words: vec![bits] }
        }
    }

    fn set(&mut self, cpu: u32) {
        let word_idx = cpu as usize / 64;
        if self.words.len() <= word_idx {
            self.words.resize(word_idx + 1, 0);
        }
        self.words[word_idx] |= 1u64 << (cpu % 64);
    }

    fn contains(&self, cpu: u32) -> bool {
        let word_idx = cpu as usize / 64;
        self.words
            .get(word_idx)
            .is_some_and(|word| *word & (1u64 << (cpu % 64)) != 0)
    }

    fn cpus(&self) -> Vec<u32> {
        let mut cpus = Vec::new();
        for cpu in 0..cpu_set_size() {
            if self.contains(cpu) {
                cpus.push(cpu);
            }
        }
        cpus
    }

    fn to_cpu_set(&self) -> libc::cpu_set_t {
        let mut set = unsafe { std::mem::zeroed::<libc::cpu_set_t>() };
        unsafe {
            libc::CPU_ZERO(&mut set);
        }

        for cpu in 0..cpu_set_size() {
            if self.contains(cpu) {
                unsafe {
                    libc::CPU_SET(cpu as usize, &mut set);
                }
            }
        }

        set
    }

    fn from_cpu_set(set: &libc::cpu_set_t) -> Self {
        let mut mask = Self::empty();

        for cpu in 0..cpu_set_size() {
            if unsafe { libc::CPU_ISSET(cpu as usize, set) } {
                mask.set(cpu);
            }
        }

        mask
    }
}

impl Serialize for CpuMask {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_range_string())
    }
}

impl<'de> Deserialize<'de> for CpuMask {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct CpuMaskVisitor;

        impl Visitor<'_> for CpuMaskVisitor {
            type Value = CpuMask;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a CPU range string or legacy numeric CPU mask")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: DeError,
            {
                CpuMask::parse(value).map_err(E::custom)
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: DeError,
            {
                Ok(CpuMask::from_legacy_bits(value))
            }
        }

        deserializer.deserialize_any(CpuMaskVisitor)
    }
}

pub fn read_allowed_mask(tid: Tid) -> io::Result<CpuMask> {
    read_allowed_mask_raw(tid.as_u32())
}

pub fn read_allowed_mask_raw(tid: u32) -> io::Result<CpuMask> {
    let mut set = unsafe { std::mem::zeroed::<libc::cpu_set_t>() };
    let result = unsafe {
        libc::sched_getaffinity(
            tid as libc::pid_t,
            std::mem::size_of::<libc::cpu_set_t>(),
            &mut set,
        )
    };

    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(CpuMask::from_cpu_set(&set))
}

pub fn set_affinity(tid: Tid, mask: &CpuMask) -> io::Result<()> {
    set_affinity_raw(tid.as_u32(), mask)
}

pub fn set_affinity_raw(tid: u32, mask: &CpuMask) -> io::Result<()> {
    let set = mask.to_cpu_set();
    let result = unsafe {
        libc::sched_setaffinity(
            tid as libc::pid_t,
            std::mem::size_of::<libc::cpu_set_t>(),
            &set,
        )
    };

    if result != 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct RestoreSummary {
    pub restored: usize,
    pub skipped_dead: usize,
    pub skipped_identity_mismatch: usize,
    pub legacy_unverified: usize,
    pub errors: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreRecordStatus {
    Verified,
    LegacyUnverified,
    Dead,
    IdentityMismatch,
}

pub fn restore_all(records: &[AffinityRecord]) -> (RestoreSummary, Vec<anyhow::Error>) {
    restore_all_at(Path::new("/proc"), records)
}

fn restore_all_at(
    proc_root: &Path,
    records: &[AffinityRecord],
) -> (RestoreSummary, Vec<anyhow::Error>) {
    let mut summary = RestoreSummary::default();
    let mut errors = Vec::new();

    for record in records {
        match restore_record_status_at(proc_root, record) {
            Ok(RestoreRecordStatus::Verified) => {}
            Ok(RestoreRecordStatus::LegacyUnverified) => {
                summary.legacy_unverified += 1;
                log::warn!(
                    "restore_record_missing_identity tid={}; restoring by numeric TID only for legacy restore file",
                    record.tid
                );
            }
            Ok(RestoreRecordStatus::Dead) => {
                summary.skipped_dead += 1;
                continue;
            }
            Ok(RestoreRecordStatus::IdentityMismatch) => {
                summary.skipped_identity_mismatch += 1;
                log::warn!(
                    "restore_record_identity_mismatch tid={}; skipping affinity restore to avoid TID reuse damage",
                    record.tid
                );
                continue;
            }
            Err(err) => {
                summary.errors += 1;
                errors.push(err.into());
                continue;
            }
        }

        match set_affinity(record.tid, &record.original_mask) {
            Ok(()) => summary.restored += 1,
            Err(err) if err.raw_os_error() == Some(libc::ESRCH) => {
                summary.skipped_dead += 1;
            }
            Err(err) => {
                summary.errors += 1;
                errors.push(affinity_set_error(record.tid, err));
            }
        }
    }

    (summary, errors)
}

// restore_record_status removed as unused

fn restore_record_status_at(
    proc_root: &Path,
    record: &AffinityRecord,
) -> io::Result<RestoreRecordStatus> {
    restore_identity_status_at(
        proc_root,
        record.tid,
        record.process_pid,
        record.process_starttime_ticks,
        record.task_starttime_ticks,
    )
}

pub(crate) fn restore_identity_status_at(
    proc_root: &Path,
    tid: Tid,
    process_pid: Option<Pid>,
    process_starttime_ticks: Option<u64>,
    task_starttime_ticks: Option<u64>,
) -> io::Result<RestoreRecordStatus> {
    // No identity at all -> legacy restore by numeric TID (back-compat).
    if process_pid.is_none() && process_starttime_ticks.is_none() && task_starttime_ticks.is_none()
    {
        return Ok(RestoreRecordStatus::LegacyUnverified);
    }

    // If any identity field is present but the trio is incomplete, treat
    // this as an identity mismatch rather than falling back to a numeric
    // TID restore. This prevents accidental restores when schema v3
    // records include partial identity data.
    let (Some(process_pid), Some(process_starttime_ticks), Some(task_starttime_ticks)) =
        (process_pid, process_starttime_ticks, task_starttime_ticks)
    else {
        return Ok(RestoreRecordStatus::IdentityMismatch);
    };

    let process_stat_path = proc_root.join(process_pid.to_string()).join("stat");
    let process_starttime = match stat_starttime_at(&process_stat_path) {
        Ok(starttime) => starttime,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(RestoreRecordStatus::Dead),
        Err(err) => {
            return Err(io::Error::new(
                err.kind(),
                format!(
                    "failed to read process identity for TID {} via {}: {err}",
                    tid,
                    process_stat_path.display()
                ),
            ));
        }
    };
    if process_starttime != Some(process_starttime_ticks) {
        return Ok(RestoreRecordStatus::IdentityMismatch);
    }

    let task_stat_path = proc_root
        .join(process_pid.to_string())
        .join("task")
        .join(tid.to_string())
        .join("stat");
    let task_starttime = match stat_starttime_at(&task_stat_path) {
        Ok(starttime) => starttime,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(RestoreRecordStatus::Dead),
        Err(err) => {
            return Err(io::Error::new(
                err.kind(),
                format!(
                    "failed to read task identity for TID {} via {}: {err}",
                    tid,
                    task_stat_path.display()
                ),
            ));
        }
    };
    if task_starttime != Some(task_starttime_ticks) {
        return Ok(RestoreRecordStatus::IdentityMismatch);
    }

    Ok(RestoreRecordStatus::Verified)
}

#[cfg(test)]
impl AffinityRecord {
    fn has_identity(&self) -> bool {
        self.process_pid.is_some()
            || self.process_starttime_ticks.is_some()
            || self.task_starttime_ticks.is_some()
    }
}

fn stat_starttime_at(path: &Path) -> io::Result<Option<u64>> {
    let stat = fs::read_to_string(path)?;
    Ok(crate::process_tree::parse_proc_stat_starttime(&stat))
}

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

fn parse_cpu(value: &str) -> anyhow::Result<u32> {
    let cpu = value
        .trim()
        .parse::<u32>()
        .with_context(|| format!("invalid CPU id {value:?}"))?;
    let max_cpus = cpu_set_size();
    if cpu >= max_cpus {
        anyhow::bail!(
            "CPU id {cpu} is outside the supported 0..{} range",
            max_cpus.saturating_sub(1)
        );
    }
    Ok(cpu)
}

fn cpu_set_size() -> u32 {
    libc::CPU_SETSIZE as u32
}

fn affinity_set_error(tid: Tid, err: io::Error) -> anyhow::Error {
    anyhow::anyhow!("failed to set CPU affinity for TID {tid}: {err}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cpu_mask_ranges_and_lists() {
        assert_eq!(CpuMask::parse("0-2,5").unwrap().to_range_string(), "0-2,5");
    }

    #[test]
    fn rejects_empty_cpu_mask() {
        assert!(CpuMask::parse("").is_err());
    }

    #[test]
    fn parses_cpu_ids_above_63() {
        let mask = CpuMask::parse("0,64").unwrap();

        assert_eq!(mask.to_range_string(), "0,64");
    }

    #[test]
    fn serializes_ranges_and_deserializes_legacy_numeric_masks() {
        let mask = CpuMask::parse("0-2,5").unwrap();
        assert_eq!(serde_json::to_string(&mask).unwrap(), r#""0-2,5""#);

        let legacy: CpuMask = serde_json::from_str("39").unwrap();
        assert_eq!(legacy.to_range_string(), "0-2,5");
    }

    #[test]
    fn affinity_record_typed_ids_preserve_numeric_json_shape() {
        let record = AffinityRecord {
            tid: Tid::new(7),
            process_pid: Some(Pid::new(42)),
            process_starttime_ticks: Some(100),
            task_starttime_ticks: Some(200),
            original_mask: CpuMask::parse("0-3").unwrap(),
            applied_mask: CpuMask::parse("0-1").unwrap(),
        };

        let json = serde_json::to_value(&record).unwrap();
        assert_eq!(json["tid"], 7);
        assert_eq!(json["process_pid"], 42);

        let decoded: AffinityRecord = serde_json::from_value(json).unwrap();
        assert_eq!(decoded.tid, Tid::new(7));
        assert_eq!(decoded.process_pid, Some(Pid::new(42)));
    }

    #[test]
    fn merged_restore_state_preserves_earliest_original_mask() {
        let dir = temp_dir("affinity-merge");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("restore.json");
        save_restore_state(&path, &[affinity_record(7, "0-3", "0-1")]).unwrap();

        save_merged_restore_state(&path, &[affinity_record(7, "0-1", "0")], false).unwrap();

        let state = load_restore_state(&path).unwrap();
        assert_eq!(state.schema_version, RESTORE_SCHEMA_VERSION);
        assert_eq!(state.records.len(), 1);
        assert_eq!(state.records[0].original_mask.to_range_string(), "0-3");
        assert_eq!(state.records[0].applied_mask.to_range_string(), "0");

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn restore_all_skips_dead_tids() {
        let (summary, errors) = restore_all(&[affinity_record(i32::MAX as u32, "0", "0")]);

        assert_eq!(summary.restored, 0);
        assert_eq!(summary.skipped_dead, 1);
        assert_eq!(summary.errors, 0);
        assert!(errors.is_empty());
    }

    #[test]
    fn restore_saved_deletes_file_when_only_dead_tids_are_skipped() {
        let dir = temp_dir("affinity-restore-dead");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("restore.json");
        save_restore_state(&path, &[affinity_record(i32::MAX as u32, "0", "0")]).unwrap();

        let summary = restore_saved(&path).unwrap();

        assert_eq!(summary.skipped_dead, 1);
        assert!(!path.exists());
        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn restore_record_status_verifies_saved_identity() {
        let dir = temp_dir("affinity-identity");
        write_fake_task_stat(&dir, 10, 11, 100, 111);

        let mut record = affinity_record(11, "0", "1");
        record.process_pid = Some(10.into());
        record.process_starttime_ticks = Some(100);
        record.task_starttime_ticks = Some(111);
        assert_eq!(
            restore_record_status_at(&dir, &record).unwrap(),
            RestoreRecordStatus::Verified
        );

        record.task_starttime_ticks = Some(222);
        assert_eq!(
            restore_record_status_at(&dir, &record).unwrap(),
            RestoreRecordStatus::IdentityMismatch
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn merged_restore_state_keys_by_task_identity() {
        let dir = temp_dir("affinity-merge-identity");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("restore.json");

        let mut original = affinity_record(7, "0-3", "0-1");
        original.process_pid = Some(7.into());
        original.process_starttime_ticks = Some(70);
        original.task_starttime_ticks = Some(70);
        save_restore_state(&path, &[original]).unwrap();

        let mut same_identity = affinity_record(7, "0-1", "0");
        same_identity.process_pid = Some(7.into());
        same_identity.process_starttime_ticks = Some(70);
        same_identity.task_starttime_ticks = Some(70);
        save_merged_restore_state(&path, &[same_identity], false).unwrap();

        let state = load_restore_state(&path).unwrap();
        assert_eq!(state.records.len(), 1);
        assert_eq!(state.records[0].original_mask.to_range_string(), "0-3");
        assert_eq!(state.records[0].applied_mask.to_range_string(), "0");

        let mut new_identity = affinity_record(7, "1-3", "1");
        new_identity.process_pid = Some(7.into());
        new_identity.process_starttime_ticks = Some(70);
        new_identity.task_starttime_ticks = Some(71);
        save_merged_restore_state(&path, &[new_identity], false).unwrap();

        let state = load_restore_state(&path).unwrap();
        assert_eq!(state.records.len(), 2);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn merged_restore_state_replaces_legacy_same_tid_record() {
        let dir = temp_dir("affinity-merge-legacy");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("restore.json");

        save_restore_state(&path, &[affinity_record(7, "0-3", "0-1")]).unwrap();

        let mut identity_record = affinity_record(7, "1-3", "1");
        identity_record.process_pid = Some(7.into());
        identity_record.process_starttime_ticks = Some(70);
        identity_record.task_starttime_ticks = Some(70);
        save_merged_restore_state(&path, &[identity_record], false).unwrap();

        let state = load_restore_state(&path).unwrap();
        assert_eq!(state.records.len(), 1);
        assert_eq!(state.records[0].original_mask.to_range_string(), "1-3");
        assert_eq!(state.records[0].task_starttime_ticks, Some(70));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn can_round_trip_current_thread_affinity_when_allowed() {
        let Ok(current) = read_allowed_mask_raw(0) else {
            return;
        };
        if current.is_empty() {
            return;
        }

        let Ok(()) = set_affinity_raw(0, &current) else {
            return;
        };

        let reread = read_allowed_mask_raw(0).unwrap();
        assert_eq!(reread, current);
    }

    fn affinity_record(tid: u32, original_mask: &str, applied_mask: &str) -> AffinityRecord {
        AffinityRecord {
            tid: tid.into(),
            process_pid: None,
            process_starttime_ticks: None,
            task_starttime_ticks: None,
            original_mask: CpuMask::parse(original_mask).unwrap(),
            applied_mask: CpuMask::parse(applied_mask).unwrap(),
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
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        dir
    }

    #[test]
    fn merged_restore_state_preserves_earliest_original_mask_even_if_merge_order_is_swapped() {
        let dir = temp_dir("affinity-merge-swapped");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("restore.json");

        // File has LATER record
        save_restore_state(&path, &[affinity_record(7, "0-1", "0")]).unwrap();

        // New has EARLIER record
        save_merged_restore_state(&path, &[affinity_record(7, "0-3", "0-1")], false).unwrap();

        let state = load_restore_state(&path).unwrap();
        assert_eq!(state.records.len(), 1);
        assert_eq!(state.records[0].original_mask.to_range_string(), "0-3");
        assert_eq!(state.records[0].applied_mask.to_range_string(), "0");

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn merged_restore_state_preserves_earliest_original_mask_during_legacy_upgrade() {
        let dir = temp_dir("affinity-legacy-upgrade");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("restore.json");

        // File has legacy record (earliest)
        save_restore_state(&path, &[affinity_record(7, "0-3", "0-1")]).unwrap();

        // New has identity record (later)
        let mut identity = affinity_record(7, "0-1", "0");
        identity.process_pid = Some(7.into());
        identity.process_starttime_ticks = Some(70);
        identity.task_starttime_ticks = Some(70);

        save_merged_restore_state(&path, &[identity], false).unwrap();

        let state = load_restore_state(&path).unwrap();
        assert_eq!(state.records.len(), 1);
        assert_eq!(state.records[0].original_mask.to_range_string(), "0-3");
        assert_eq!(state.records[0].applied_mask.to_range_string(), "0");

        fs::remove_dir_all(dir).ok();
    }
}
