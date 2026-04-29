use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CpuMask(pub u64);

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

pub const RESTORE_SCHEMA_VERSION: u32 = 1;

impl CpuMask {
    pub fn parse(value: &str) -> anyhow::Result<Self> {
        let mut mask = 0u64;

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
                mask |= 1u64 << cpu;
            }
        }

        if mask == 0 {
            anyhow::bail!("CPU mask must contain at least one CPU");
        }

        Ok(Self(mask))
    }

    fn to_cpu_set(self) -> libc::cpu_set_t {
        let mut set = unsafe { std::mem::zeroed::<libc::cpu_set_t>() };
        unsafe {
            libc::CPU_ZERO(&mut set);
        }

        for cpu in 0..64 {
            if self.0 & (1u64 << cpu) != 0 {
                unsafe {
                    libc::CPU_SET(cpu, &mut set);
                }
            }
        }

        set
    }

    fn from_cpu_set(set: &libc::cpu_set_t) -> Self {
        let mut mask = 0u64;

        for cpu in 0..64 {
            if unsafe { libc::CPU_ISSET(cpu, set) } {
                mask |= 1u64 << cpu;
            }
        }

        Self(mask)
    }
}

pub fn read_allowed_mask(tid: u32) -> anyhow::Result<CpuMask> {
    let mut set = unsafe { std::mem::zeroed::<libc::cpu_set_t>() };
    let result = unsafe {
        libc::sched_getaffinity(
            tid as libc::pid_t,
            std::mem::size_of::<libc::cpu_set_t>(),
            &mut set,
        )
    };

    if result != 0 {
        anyhow::bail!(
            "failed to read CPU affinity for TID {tid}: {}",
            std::io::Error::last_os_error()
        );
    }

    Ok(CpuMask::from_cpu_set(&set))
}

pub fn set_affinity(tid: u32, mask: CpuMask) -> anyhow::Result<()> {
    let set = mask.to_cpu_set();
    let result = unsafe {
        libc::sched_setaffinity(
            tid as libc::pid_t,
            std::mem::size_of::<libc::cpu_set_t>(),
            &set,
        )
    };

    if result != 0 {
        anyhow::bail!(
            "failed to set CPU affinity for TID {tid}: {}",
            std::io::Error::last_os_error()
        );
    }

    Ok(())
}

pub fn restore_all(records: &[AffinityRecord]) -> Vec<anyhow::Error> {
    let mut errors = Vec::new();

    for record in records {
        if let Err(err) = set_affinity(record.tid, record.original_mask) {
            errors.push(err);
        }
    }

    errors
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

pub fn load_restore_state(path: &Path) -> anyhow::Result<RestoreState> {
    let data = fs::read_to_string(path)
        .with_context(|| format!("failed to read restore file {}", path.display()))?;
    let state = serde_json::from_str(&data)
        .with_context(|| format!("failed to parse restore file {}", path.display()))?;
    Ok(state)
}

pub fn restore_saved(path: &Path) -> anyhow::Result<usize> {
    let state = load_restore_state(path)?;
    let errors = restore_all(&state.records);

    if !errors.is_empty() {
        anyhow::bail!(
            "failed to restore {} affinity record(s); restore file kept at {}",
            errors.len(),
            path.display()
        );
    }

    fs::remove_file(path).ok();
    Ok(state.records.len())
}

fn parse_cpu(value: &str) -> anyhow::Result<u32> {
    let cpu = value
        .trim()
        .parse::<u32>()
        .with_context(|| format!("invalid CPU id {value:?}"))?;
    if cpu >= 64 {
        anyhow::bail!("CPU id {cpu} is outside the supported 0..63 range");
    }
    Ok(cpu)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cpu_mask_ranges_and_lists() {
        assert_eq!(CpuMask::parse("0-2,5").unwrap(), CpuMask(0b100111));
    }

    #[test]
    fn rejects_empty_cpu_mask() {
        assert!(CpuMask::parse("").is_err());
    }
}
