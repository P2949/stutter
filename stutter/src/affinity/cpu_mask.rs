use std::fmt;

use anyhow::Context;
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error as DeError, Visitor},
};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct CpuMask {
    words: Vec<u64>,
}

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

    pub(super) fn empty() -> Self {
        Self { words: Vec::new() }
    }

    fn from_legacy_bits(bits: u64) -> Self {
        if bits == 0 {
            Self::empty()
        } else {
            Self { words: vec![bits] }
        }
    }

    pub(super) fn set(&mut self, cpu: u32) {
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

    pub(super) fn cpus(&self) -> Vec<u32> {
        let mut cpus = Vec::new();
        for cpu in 0..cpu_set_size() {
            if self.contains(cpu) {
                cpus.push(cpu);
            }
        }
        cpus
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

pub(super) fn cpu_set_size() -> u32 {
    libc::CPU_SETSIZE as u32
}
