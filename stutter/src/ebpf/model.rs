//! eBPF loader handle and report model types.

use aya::{
    Ebpf,
    maps::{HashMap as AyaHashMap, MapData, PerCpuArray, RingBuf},
};
use serde::{Deserialize, Serialize};
use stutter_common::{
    DROP_BLOCK_FALLBACK_KEY_COLLISION, DROP_BLOCK_START_INSERT_FAILED,
    DROP_CPU_ACCOUNTING_UNTRACKED, DROP_IRQ_START_TIMES_INSERT_FAILED, DROP_RINGBUF_RESERVE_FAILED,
    DROP_WAKEUP_DATA_CONSUMED_READ_FAILED, DROP_WAKEUP_DATA_INSERT_FAILED,
    DROP_WAKEUP_DATA_REPLACED_ENTRY, DROP_WAKEUP_DATA_STALE_ENTRY,
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
    pub native_cgroup_filter: NativeCgroupFilterStatus,
    pub activation_plan: ProbeActivationPlan,
    pub(crate) drop_counters: PerCpuArray<MapData, u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeCgroupFilterStatus {
    /// The user requested native cgroup filtering for this run.
    pub enabled: bool,
    /// The eBPF cgroup-id filter was actually populated and active.
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub resolver: Option<String>,
    #[serde(default)]
    pub cgroup_path: Option<String>,
    #[serde(default)]
    pub cgroup_id: Option<u64>,
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub warning: Option<String>,
}

impl NativeCgroupFilterStatus {
    pub fn disabled() -> Self {
        Self::default()
    }

    pub fn is_disabled(&self) -> bool {
        !self.enabled
            && !self.active
            && self.resolver.is_none()
            && self.cgroup_path.is_none()
            && self.cgroup_id.is_none()
            && !self.verified
            && self.warning.is_none()
    }

    pub fn unverified_directory_inode(cgroup_path: String, cgroup_id: u64) -> Self {
        Self {
            enabled: true,
            active: false,
            resolver: Some("directory_inode".to_owned()),
            cgroup_path: Some(cgroup_path),
            cgroup_id: Some(cgroup_id),
            verified: false,
            warning: Some(
                "native cgroup filtering was requested but not activated because directory-inode cgroup id resolution is not runtime-verified; PID expansion remains the authoritative scheduler-wakeup targeting path"
                    .to_owned(),
            ),
        }
    }

    pub fn verified_directory_inode(cgroup_path: String, cgroup_id: u64) -> Self {
        Self {
            enabled: true,
            active: true,
            resolver: Some("directory_inode".to_owned()),
            cgroup_path: Some(cgroup_path),
            cgroup_id: Some(cgroup_id),
            verified: true,
            warning: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockIoCorrelationBasis {
    Disabled,
    DevSector,
    RequestPointer,
}

impl BlockIoCorrelationBasis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "unavailable",
            Self::DevSector => "dev+sector",
            Self::RequestPointer => "request-pointer",
        }
    }

    pub fn confidence(self) -> &'static str {
        match self {
            Self::RequestPointer => "high",
            Self::DevSector => "medium",
            Self::Disabled => "none",
        }
    }

    pub fn warning(self) -> Option<&'static str> {
        match self {
            Self::RequestPointer => None,
            Self::DevSector => Some(
                "Block I/O correlation is approximate (dev+sector fallback); concurrent same-sector requests may collide. Ambiguous fallback samples are dropped, so block I/O latency coverage may be incomplete.",
            ),
            Self::Disabled => Some(
                "Block I/O correlation is unavailable because block_rq tracepoints were not requested, unavailable, or incompatible on this kernel.",
            ),
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "request-pointer" => Self::RequestPointer,
            "unavailable" | "not_requested" => Self::Disabled,
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
    #[serde(default)]
    pub wakeup_data_replaced_entries: u64,
    #[serde(default)]
    pub wakeup_data_consumed_read_failed: u64,
    pub ringbuf_reserve_failed: u64,
    #[serde(default)]
    pub irq_start_times_insert_failed: u64,
    #[serde(default)]
    pub block_start_insert_failed: u64,
    #[serde(default)]
    pub block_fallback_key_collisions: u64,
    #[serde(default)]
    pub cpu_accounting_untracked: u64,
}

impl DropCountersSnapshot {
    pub fn total(&self) -> u64 {
        self.wakeup_data_insert_failed
            .saturating_add(self.wakeup_data_stale_entries)
            .saturating_add(self.wakeup_data_replaced_entries)
            .saturating_add(self.wakeup_data_consumed_read_failed)
            .saturating_add(self.ringbuf_reserve_failed)
            .saturating_add(self.irq_start_times_insert_failed)
            .saturating_add(self.block_start_insert_failed)
            .saturating_add(self.block_fallback_key_collisions)
            .saturating_add(self.cpu_accounting_untracked)
    }

    pub fn total_excluding_block_io(&self) -> u64 {
        self.wakeup_data_insert_failed
            .saturating_add(self.wakeup_data_stale_entries)
            .saturating_add(self.wakeup_data_replaced_entries)
            .saturating_add(self.wakeup_data_consumed_read_failed)
            .saturating_add(self.ringbuf_reserve_failed)
            .saturating_add(self.irq_start_times_insert_failed)
            .saturating_add(self.cpu_accounting_untracked)
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
            wakeup_data_consumed_read_failed: drop_counter_value(
                &self.drop_counters,
                DROP_WAKEUP_DATA_CONSUMED_READ_FAILED,
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
            wakeup_data_replaced_entries: drop_counter_value(
                &self.drop_counters,
                DROP_WAKEUP_DATA_REPLACED_ENTRY,
            ),
            cpu_accounting_untracked: drop_counter_value(
                &self.drop_counters,
                DROP_CPU_ACCOUNTING_UNTRACKED,
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

    #[test]
    fn disabled_block_io_basis_reports_unavailable_correlation() {
        assert_eq!(BlockIoCorrelationBasis::Disabled.as_str(), "unavailable");
        assert_eq!(BlockIoCorrelationBasis::Disabled.confidence(), "none");
        assert!(
            BlockIoCorrelationBasis::Disabled
                .warning()
                .unwrap()
                .contains("unavailable")
        );
        assert_eq!(
            BlockIoCorrelationBasis::from_str("unavailable"),
            BlockIoCorrelationBasis::Disabled
        );
    }

    #[test]
    fn unverified_native_cgroup_filter_is_requested_but_not_active() {
        let status = NativeCgroupFilterStatus::unverified_directory_inode(
            "/sys/fs/cgroup/game.slice".to_owned(),
            42,
        );

        assert!(status.enabled);
        assert!(!status.active);
        assert!(!status.verified);
        assert!(
            status
                .warning
                .as_deref()
                .is_some_and(|warning| warning.contains("not activated"))
        );
    }

    #[test]
    fn verified_native_cgroup_filter_is_active_without_warning() {
        let status = NativeCgroupFilterStatus::verified_directory_inode(
            "/sys/fs/cgroup/game.slice".to_owned(),
            42,
        );

        assert!(status.enabled);
        assert!(status.active);
        assert!(status.verified);
        assert_eq!(status.warning, None);
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
