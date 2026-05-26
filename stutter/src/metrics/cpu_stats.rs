use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use stutter_core::ids::CpuId;

#[derive(Clone, Debug, Copy, Default, Serialize, Deserialize)]
pub struct CpuStats {
    pub samples: u64,
    pub max_ns: u64,
    pub spikes: u64,
}

#[derive(Clone, Debug, Default)]
pub struct CpuStatsSet {
    pub by_cpu: BTreeMap<CpuId, CpuStats>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CpuPerfRecord {
    pub cycles: Option<u64>,
    pub instructions: Option<u64>,
    pub cache_references: Option<u64>,
    pub cache_misses: Option<u64>,
    pub ipc: Option<f64>,
    pub cache_miss_rate: Option<f64>,
    pub cache_mpki: Option<f64>,
    pub time_enabled_ns: Option<u64>,
    pub time_running_ns: Option<u64>,
    pub multiplexed: bool,
    pub scaled: bool,
    pub unavailable_reason: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct CpuPerfAccumulator {
    pub cycles: u128,
    pub instructions: u128,
    pub cache_references: u128,
    pub cache_misses: u128,
    pub time_enabled_ns: u128,
    pub time_running_ns: u128,
    pub samples: u64,
    pub multiplexed_samples: u64,
    pub scaled_samples: u64,
    pub unavailable_samples: u64,
    pub last_unavailable_reason: Option<String>,
}

#[derive(Clone, Debug, Copy, Serialize, Deserialize, Default)]
pub struct CpuLine {
    pub cpu: CpuId,
    pub samples: u64,
    pub max_ns: u64,
    pub spikes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct CpuSnapshot {
    pub busiest_cpu: Option<CpuId>,
    pub busiest_cpu_samples: u64,
    pub worst_cpu: Option<CpuId>,
    pub worst_cpu_max_ns: u64,
    pub spikiest_cpu: Option<CpuId>,
    pub spikiest_cpu_spikes: u64,
    pub per_cpu: Vec<CpuLine>,
}

impl CpuStats {
    pub fn record(&mut self, latency_ns: u64, spike_threshold_ns: u64) {
        self.samples += 1;
        self.max_ns = self.max_ns.max(latency_ns);

        if latency_ns >= spike_threshold_ns {
            self.spikes += 1;
        }
    }
}

impl CpuStatsSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record(&mut self, cpu: CpuId, latency_ns: u64, spike_threshold_ns: u64) {
        self.by_cpu
            .entry(cpu)
            .or_default()
            .record(latency_ns, spike_threshold_ns);
    }

    pub fn snapshot(&self) -> CpuSnapshot {
        let mut busiest_cpu = None;
        let mut busiest_cpu_samples = 0;

        let mut worst_cpu = None;
        let mut worst_cpu_max_ns = 0;

        let mut spikiest_cpu = None;
        let mut spikiest_cpu_spikes = 0;

        let mut per_cpu = Vec::with_capacity(self.by_cpu.len());

        for (cpu, stats) in &self.by_cpu {
            if stats.samples > busiest_cpu_samples {
                busiest_cpu = Some(*cpu);
                busiest_cpu_samples = stats.samples;
            }

            if stats.max_ns > worst_cpu_max_ns {
                worst_cpu = Some(*cpu);
                worst_cpu_max_ns = stats.max_ns;
            }

            if stats.spikes > spikiest_cpu_spikes {
                spikiest_cpu = Some(*cpu);
                spikiest_cpu_spikes = stats.spikes;
            }

            per_cpu.push(CpuLine {
                cpu: *cpu,
                samples: stats.samples,
                max_ns: stats.max_ns,
                spikes: stats.spikes,
            });
        }

        CpuSnapshot {
            busiest_cpu,
            busiest_cpu_samples,
            worst_cpu,
            worst_cpu_max_ns,
            spikiest_cpu,
            spikiest_cpu_spikes,
            per_cpu,
        }
    }

    pub fn snapshot_and_reset(&mut self) -> CpuSnapshot {
        let snapshot = self.snapshot();
        self.by_cpu.clear();
        snapshot
    }
}

impl CpuPerfAccumulator {
    pub fn record(&mut self, delta: &crate::perf_counters::CpuPerfDelta) {
        self.samples = self.samples.saturating_add(1);

        if let Some(cycles) = delta.cycles {
            self.cycles = self.cycles.saturating_add(cycles as u128);
        }
        if let Some(instructions) = delta.instructions {
            self.instructions = self.instructions.saturating_add(instructions as u128);
        }
        if let Some(cache_references) = delta.cache_references {
            self.cache_references = self
                .cache_references
                .saturating_add(cache_references as u128);
        }
        if let Some(cache_misses) = delta.cache_misses {
            self.cache_misses = self.cache_misses.saturating_add(cache_misses as u128);
        }
        if let Some(time_enabled_ns) = delta.time_enabled_ns {
            self.time_enabled_ns = self.time_enabled_ns.saturating_add(time_enabled_ns as u128);
        }
        if let Some(time_running_ns) = delta.time_running_ns {
            self.time_running_ns = self.time_running_ns.saturating_add(time_running_ns as u128);
        }

        if delta.multiplexed {
            self.multiplexed_samples = self.multiplexed_samples.saturating_add(1);
        }
        if delta.scaled {
            self.scaled_samples = self.scaled_samples.saturating_add(1);
        }
        if let Some(reason) = &delta.unavailable_reason {
            self.unavailable_samples = self.unavailable_samples.saturating_add(1);
            self.last_unavailable_reason = Some(reason.clone());
        }
    }

    pub fn snapshot(&self) -> Option<CpuPerfRecord> {
        if self.samples == 0 {
            return None;
        }

        let cycles = optional_u128_to_u64(self.cycles);
        let instructions = optional_u128_to_u64(self.instructions);
        let cache_references = optional_u128_to_u64(self.cache_references);
        let cache_misses = optional_u128_to_u64(self.cache_misses);

        Some(CpuPerfRecord {
            cycles,
            instructions,
            cache_references,
            cache_misses,
            ipc: ratio_u128(self.instructions, self.cycles),
            cache_miss_rate: ratio_u128(self.cache_misses, self.cache_references),
            cache_mpki: if self.instructions > 0 {
                Some(self.cache_misses as f64 * 1000.0 / self.instructions as f64)
                    .filter(|v| v.is_finite())
            } else {
                None
            },
            time_enabled_ns: optional_u128_to_u64(self.time_enabled_ns),
            time_running_ns: optional_u128_to_u64(self.time_running_ns),
            multiplexed: self.multiplexed_samples > 0,
            scaled: self.scaled_samples > 0,
            unavailable_reason: self.last_unavailable_reason.clone(),
        })
    }

    pub fn snapshot_and_reset(&mut self) -> Option<CpuPerfRecord> {
        let snapshot = self.snapshot()?;
        *self = Self::default();
        Some(snapshot)
    }
}

impl CpuLine {
    pub fn cpu_id(&self) -> CpuId {
        self.cpu
    }
}

fn optional_u128_to_u64(value: u128) -> Option<u64> {
    if value == 0 {
        None
    } else {
        Some(value.min(u64::MAX as u128) as u64)
    }
}

fn ratio_u128(numerator: u128, denominator: u128) -> Option<f64> {
    if denominator == 0 {
        return None;
    }
    Some(numerator as f64 / denominator as f64).filter(|v| v.is_finite())
}
