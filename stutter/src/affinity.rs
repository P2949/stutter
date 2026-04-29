use std::{
    collections::BTreeMap,
    env, fmt, fs, io,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error as DeError, Visitor},
};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct CpuMask {
    words: Vec<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AffinityRecord {
    pub tid: u32,
    pub original_mask: CpuMask,
    pub applied_mask: CpuMask,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RestoreState {
    pub schema_version: u32,
    pub records: Vec<AffinityRecord>,
}

pub const RESTORE_SCHEMA_VERSION: u32 = 2;

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

pub fn read_allowed_mask(tid: u32) -> anyhow::Result<CpuMask> {
    read_allowed_mask_raw(tid).map_err(|err| {
        anyhow::anyhow!("failed to read CPU affinity for TID {tid}: {err}")
    })
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
    pub errors: usize,
}

pub fn restore_all(records: &[AffinityRecord]) -> (RestoreSummary, Vec<anyhow::Error>) {
    let mut summary = RestoreSummary::default();
    let mut errors = Vec::new();

    for record in records {
        match set_affinity_raw(record.tid, &record.original_mask) {
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
        merged.insert(record.tid, record);
    }

    for record in records {
        merged
            .entry(record.tid)
            .and_modify(|existing| {
                existing.applied_mask = record.applied_mask.clone();
            })
            .or_insert_with(|| record.clone());
    }

    let records = merged.into_values().collect::<Vec<_>>();
    save_restore_state(path, &records)
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

fn affinity_set_error(tid: u32, err: io::Error) -> anyhow::Error {
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
    fn merged_restore_state_preserves_earliest_original_mask() {
        let dir = temp_dir("affinity-merge");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("restore.json");
        save_restore_state(
            &path,
            &[AffinityRecord {
                tid: 7,
                original_mask: CpuMask::parse("0-3").unwrap(),
                applied_mask: CpuMask::parse("0-1").unwrap(),
            }],
        )
        .unwrap();

        save_merged_restore_state(
            &path,
            &[AffinityRecord {
                tid: 7,
                original_mask: CpuMask::parse("0-1").unwrap(),
                applied_mask: CpuMask::parse("0").unwrap(),
            }],
            false,
        )
        .unwrap();

        let state = load_restore_state(&path).unwrap();
        assert_eq!(state.schema_version, RESTORE_SCHEMA_VERSION);
        assert_eq!(state.records.len(), 1);
        assert_eq!(state.records[0].original_mask.to_range_string(), "0-3");
        assert_eq!(state.records[0].applied_mask.to_range_string(), "0");

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn restore_all_skips_dead_tids() {
        let (summary, errors) = restore_all(&[AffinityRecord {
            tid: i32::MAX as u32,
            original_mask: CpuMask::parse("0").unwrap(),
            applied_mask: CpuMask::parse("0").unwrap(),
        }]);

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
        save_restore_state(
            &path,
            &[AffinityRecord {
                tid: i32::MAX as u32,
                original_mask: CpuMask::parse("0").unwrap(),
                applied_mask: CpuMask::parse("0").unwrap(),
            }],
        )
        .unwrap();

        let summary = restore_saved(&path).unwrap();

        assert_eq!(summary.skipped_dead, 1);
        assert!(!path.exists());
        fs::remove_dir_all(dir).ok();
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
}
