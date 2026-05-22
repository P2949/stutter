//! eBPF loader handle and report model types.

use aya::{
    Ebpf,
    maps::{HashMap as AyaHashMap, MapData, PerCpuArray, RingBuf},
};
use serde::{Deserialize, Serialize};
use stutter_common::{
    DROP_BLOCK_FALLBACK_KEY_COLLISION, DROP_BLOCK_START_INSERT_FAILED,
    DROP_IRQ_START_TIMES_INSERT_FAILED, DROP_RINGBUF_RESERVE_FAILED,
    DROP_WAKEUP_DATA_INSERT_FAILED, DROP_WAKEUP_DATA_STALE_ENTRY,
};
use tokio::io::unix::AsyncFd;

use crate::probe_activation::ProbeActivationPlan;

pub struct LoadedEbpf {
    pub(crate) _ebpf: Ebpf,
    pub events: AsyncFd<RingBuf<MapData>>,
    pub target_pid_map: AyaHashMap<MapData, u32, u8>,
    pub target_irq_map: Option<AyaHashMap<MapData, u32, u8>>,
    // Stored to keep the optional native cgroup map FD alive for the session.
    pub target_cgroup_map: Option<AyaHashMap<MapData, u64, u8>>,
    pub prev_faults_map: Option<AyaHashMap<MapData, u32, [u64; 2]>>, // (tid) -> (maj, min)
    pub block_io_correlation_basis: BlockIoCorrelationBasis,
    pub activation_plan: ProbeActivationPlan,
    pub(crate) drop_counters: PerCpuArray<MapData, u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockIoCorrelationBasis {
    DevSector,
    RequestPointer,
}

impl BlockIoCorrelationBasis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DevSector => "dev+sector",
            Self::RequestPointer => "request-pointer",
        }
    }

    pub fn confidence(self) -> &'static str {
        match self {
            Self::RequestPointer => "high",
            Self::DevSector => "medium",
        }
    }

    pub fn warning(self) -> Option<&'static str> {
        match self {
            Self::RequestPointer => None,
            Self::DevSector => Some(
                "Block I/O correlation is approximate (dev+sector fallback); concurrent same-sector requests may collide. Ambiguous fallback samples are dropped, so block I/O latency coverage may be incomplete.",
            ),
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "request-pointer" => Self::RequestPointer,
            _ => Self::DevSector,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DropCountersSnapshot {
    #[serde(
        rename = "lost_wakeup_timestamp_inserts",
        alias = "wakeup_data_insert_failed"
    )]
    pub wakeup_data_insert_failed: u64,
    #[serde(default)]
    pub wakeup_data_stale_entries: u64,
    pub ringbuf_reserve_failed: u64,
    #[serde(default)]
    pub irq_start_times_insert_failed: u64,
    #[serde(default)]
    pub block_start_insert_failed: u64,
    #[serde(default)]
    pub block_fallback_key_collisions: u64,
}

impl DropCountersSnapshot {
    pub fn total(&self) -> u64 {
        self.wakeup_data_insert_failed
            .saturating_add(self.wakeup_data_stale_entries)
            .saturating_add(self.ringbuf_reserve_failed)
            .saturating_add(self.irq_start_times_insert_failed)
            .saturating_add(self.block_start_insert_failed)
            .saturating_add(self.block_fallback_key_collisions)
    }

    pub fn total_excluding_block_io(&self) -> u64 {
        self.wakeup_data_insert_failed
            .saturating_add(self.wakeup_data_stale_entries)
            .saturating_add(self.ringbuf_reserve_failed)
            .saturating_add(self.irq_start_times_insert_failed)
    }
}

impl LoadedEbpf {
    pub fn snapshot_drop_counters(&self) -> DropCountersSnapshot {
        DropCountersSnapshot {
            wakeup_data_insert_failed: drop_counter_value(
                &self.drop_counters,
                DROP_WAKEUP_DATA_INSERT_FAILED,
            ),
            wakeup_data_stale_entries: drop_counter_value(
                &self.drop_counters,
                DROP_WAKEUP_DATA_STALE_ENTRY,
            ),
            ringbuf_reserve_failed: drop_counter_value(
                &self.drop_counters,
                DROP_RINGBUF_RESERVE_FAILED,
            ),
            irq_start_times_insert_failed: drop_counter_value(
                &self.drop_counters,
                DROP_IRQ_START_TIMES_INSERT_FAILED,
            ),
            block_start_insert_failed: drop_counter_value(
                &self.drop_counters,
                DROP_BLOCK_START_INSERT_FAILED,
            ),
            block_fallback_key_collisions: drop_counter_value(
                &self.drop_counters,
                DROP_BLOCK_FALLBACK_KEY_COLLISION,
            ),
        }
    }
}

fn drop_counter_value(counters: &PerCpuArray<MapData, u64>, key: u32) -> u64 {
    counters
        .get(&key, 0)
        .map(|values| values.iter().copied().sum())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_sector_warning_describes_dropped_ambiguous_samples_not_misattribution() {
        let warning = BlockIoCorrelationBasis::DevSector.warning().unwrap();

        assert!(warning.contains("Ambiguous fallback samples are dropped"));
        assert!(warning.contains("coverage may be incomplete"));
        assert!(!warning.contains("misattribution"));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EbpfMapSizing {
    pub(crate) events_ringbuf_bytes: u32,
    pub(crate) wakeup_data_entries: u32,
    pub(crate) locked_memory_limit_bytes: Option<u64>,
    pub(crate) available_memory_bytes: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MemlockPolicyReport {
    pub before_limit_bytes: Option<u64>,
    pub after_limit_bytes: Option<u64>,
    pub raise_attempted: bool,
    pub raise_succeeded: bool,
    pub raise_error: Option<String>,
}
