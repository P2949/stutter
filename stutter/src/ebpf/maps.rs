use serde::Serialize;
use stutter_common::{BPF_DEFAULT_EVENTS_RINGBUF_BYTES, BPF_MAX_TRACKED_CPUS};

use crate::{
    config::TARGET_PIDS_MAX,
    ebpf::{
        memlock::locked_memory_limit_bytes,
        memory::{available_memory_bytes, system_page_size},
        model::{EbpfMapSizing, MemlockPolicyReport},
    },
};

/// Fallback when `/proc/meminfo` cannot provide MemAvailable.
///
/// Keeps auto-sizing conservative on unusual systems instead of assuming
/// unlimited memory.
const DEFAULT_AVAILABLE_MEMORY_BYTES: u64 = 1 << 30;

/// Use at most 1/64 of available system memory for automatic eBPF map sizing.
///
/// Manual CLI overrides remain available for recordings that show drops.
const AVAILABLE_MEMORY_BUDGET_DIVISOR: u64 = 64;

/// When RLIMIT_MEMLOCK is finite, reserve only 75% of it for stutter maps so
/// page rounding, other BPF objects, and kernel accounting overhead have margin.
const MEMLOCK_BUDGET_NUMERATOR: u64 = 3;
const MEMLOCK_BUDGET_DENOMINATOR: u64 = 4;

// Conservative userspace budgeting estimate for one logical wakeup-state slot.
// Each slot now reserves both WAKEUP_DATA and WAKEUP_CONSUMED entries, so this
// is intentionally larger than the raw eBPF-private WakeupData struct size and
// includes kernel map metadata, alignment, hash storage overhead, and safety
// margin when splitting the available map-memory budget.
pub(crate) const WAKEUP_DATA_MAP_ENTRY_BUDGET_BYTES: u64 = 128;
pub(crate) const MIN_WAKEUP_DATA_ENTRIES: u32 = 4_096;
pub(crate) const MAX_WAKEUP_DATA_ENTRIES: u32 = 1_048_576;
pub(crate) const MIN_EVENTS_RINGBUF_BYTES: u32 = 64 * 1024;
pub(crate) const MAX_EVENTS_RINGBUF_BYTES: u32 = 16 * 1024 * 1024;

/// Split the computed budget so roughly 40% is reserved for the ring buffer and
/// the rest is available for wakeup/cursor maps.
const EVENTS_BUDGET_NUMERATOR: u64 = 2;
const EVENTS_BUDGET_DENOMINATOR: u64 = 5;

#[derive(Debug, Clone, Serialize)]
pub struct EbpfMapSizingReport {
    pub locked_memory_limit_bytes: Option<u64>,
    pub available_memory_bytes: Option<u64>,
    pub events_ringbuf_bytes: u32,
    pub min_events_ringbuf_bytes: u32,
    pub max_events_ringbuf_bytes: u32,
    pub default_events_ringbuf_bytes: u32,
    pub target_pids_max: usize,
    pub max_tracked_cpus: u32,
    pub wakeup_data_entries: u32,
    pub wakeup_data_map_entry_budget_bytes: u64,
    pub min_wakeup_data_entries: u32,
    pub max_wakeup_data_entries: u32,
}

pub(crate) fn wakeup_data_entries_for_config(
    computed_entries: u32,
    max_tasks: usize,
    wakeup_map_factor: Option<u32>,
) -> u32 {
    if let Some(factor) = wakeup_map_factor {
        return u32::try_from(max_tasks)
            .unwrap_or(u32::MAX)
            .saturating_mul(factor)
            .clamp(MIN_WAKEUP_DATA_ENTRIES, MAX_WAKEUP_DATA_ENTRIES);
    }

    computed_entries
        .max(wakeup_data_entries_floor_for_max_tasks(max_tasks))
        .clamp(MIN_WAKEUP_DATA_ENTRIES, MAX_WAKEUP_DATA_ENTRIES)
}

fn wakeup_data_entries_floor_for_max_tasks(max_tasks: usize) -> u32 {
    u32::try_from(max_tasks)
        .unwrap_or(u32::MAX)
        .min(MAX_WAKEUP_DATA_ENTRIES)
}

#[cfg(test)]
pub(crate) fn map_sizing_for_config(config: &crate::config::model::MonitorConfig) -> EbpfMapSizing {
    map_sizing_for_config_from_memory(config, current_memory_snapshot())
}

pub(crate) fn map_sizing_for_config_after_memlock(
    config: &crate::config::model::MonitorConfig,
    memlock_report: &MemlockPolicyReport,
) -> EbpfMapSizing {
    map_sizing_for_config_from_memory(
        config,
        MemorySnapshot {
            locked_memory_limit_bytes: memlock_report.after_limit_bytes,
            available_memory_bytes: available_memory_bytes(),
            page_size: system_page_size(),
        },
    )
}

pub(crate) fn map_sizing_for_config_from_memory(
    config: &crate::config::model::MonitorConfig,
    snapshot: MemorySnapshot,
) -> EbpfMapSizing {
    let mut sizing = map_sizing_from_memory(snapshot);

    if let Some(kb) = config.ebpf_sizing.ringbuf_size_kb {
        let bytes = u64::from(kb).saturating_mul(1024);
        let page_size = system_page_size();
        // RingBuf requires power-of-two and page-alignment
        let rounded =
            next_power_of_two(bytes).max(next_power_of_two(u64::from(MIN_EVENTS_RINGBUF_BYTES)));
        let rounded =
            round_up_to_multiple(rounded, page_size).min(u64::from(MAX_EVENTS_RINGBUF_BYTES));
        sizing.events_ringbuf_bytes = rounded as u32;
    }

    sizing.wakeup_data_entries = wakeup_data_entries_for_config(
        sizing.wakeup_data_entries,
        config.target.max_tasks,
        config.ebpf_sizing.wakeup_map_factor,
    );

    sizing
}

pub fn ebpf_map_sizing_report() -> EbpfMapSizingReport {
    let sizing = dynamic_map_sizing();
    EbpfMapSizingReport {
        locked_memory_limit_bytes: sizing.locked_memory_limit_bytes,
        available_memory_bytes: sizing.available_memory_bytes,
        events_ringbuf_bytes: sizing.events_ringbuf_bytes,
        min_events_ringbuf_bytes: MIN_EVENTS_RINGBUF_BYTES,
        max_events_ringbuf_bytes: MAX_EVENTS_RINGBUF_BYTES,
        default_events_ringbuf_bytes: BPF_DEFAULT_EVENTS_RINGBUF_BYTES,
        target_pids_max: TARGET_PIDS_MAX,
        max_tracked_cpus: BPF_MAX_TRACKED_CPUS,
        wakeup_data_entries: sizing.wakeup_data_entries,
        wakeup_data_map_entry_budget_bytes: WAKEUP_DATA_MAP_ENTRY_BUDGET_BYTES,
        min_wakeup_data_entries: MIN_WAKEUP_DATA_ENTRIES,
        max_wakeup_data_entries: MAX_WAKEUP_DATA_ENTRIES,
    }
}

fn dynamic_map_sizing() -> EbpfMapSizing {
    map_sizing_from_memory(current_memory_snapshot())
}

fn current_memory_snapshot() -> MemorySnapshot {
    MemorySnapshot {
        locked_memory_limit_bytes: locked_memory_limit_bytes(),
        available_memory_bytes: available_memory_bytes(),
        page_size: system_page_size(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MemorySnapshot {
    pub(crate) locked_memory_limit_bytes: Option<u64>,
    pub(crate) available_memory_bytes: Option<u64>,
    pub(crate) page_size: u64,
}

pub(crate) fn map_sizing_from_memory(snapshot: MemorySnapshot) -> EbpfMapSizing {
    let available_memory = snapshot
        .available_memory_bytes
        .unwrap_or(DEFAULT_AVAILABLE_MEMORY_BYTES);
    let available_budget = available_memory / AVAILABLE_MEMORY_BUDGET_DIVISOR;
    let memlock_budget = snapshot
        .locked_memory_limit_bytes
        .map(|bytes| bytes.saturating_mul(MEMLOCK_BUDGET_NUMERATOR) / MEMLOCK_BUDGET_DENOMINATOR)
        .unwrap_or(u64::MAX);
    let budget = available_budget.min(memlock_budget);
    let events_budget = budget.saturating_mul(EVENTS_BUDGET_NUMERATOR) / EVENTS_BUDGET_DENOMINATOR;
    let page_size = snapshot.page_size.max(1);
    let min_events = u64::from(MIN_EVENTS_RINGBUF_BYTES).max(page_size);
    let max_events = u64::from(MAX_EVENTS_RINGBUF_BYTES).max(min_events);
    let events_ringbuf_bytes =
        ring_buffer_size_from_budget(events_budget, min_events, max_events, page_size);
    let wakeup_budget = budget.saturating_sub(u64::from(events_ringbuf_bytes));
    let wakeup_data_entries = wakeup_budget
        .checked_div(WAKEUP_DATA_MAP_ENTRY_BUDGET_BYTES)
        .unwrap_or(0)
        .clamp(
            u64::from(MIN_WAKEUP_DATA_ENTRIES),
            u64::from(MAX_WAKEUP_DATA_ENTRIES),
        ) as u32;

    EbpfMapSizing {
        events_ringbuf_bytes,
        wakeup_data_entries,
        locked_memory_limit_bytes: snapshot.locked_memory_limit_bytes,
        available_memory_bytes: snapshot.available_memory_bytes,
    }
}

pub(crate) fn ring_buffer_size_from_budget(
    budget: u64,
    min_size: u64,
    max_size: u64,
    page_size: u64,
) -> u32 {
    let requested = budget.clamp(min_size, max_size);
    let rounded = floor_power_of_two(requested).max(next_power_of_two(min_size));
    let rounded = round_up_to_multiple(rounded, page_size).min(max_size);
    rounded.min(u64::from(u32::MAX)) as u32
}

fn floor_power_of_two(value: u64) -> u64 {
    if value <= 1 {
        return 1;
    }
    1u64 << (u64::BITS - 1 - value.leading_zeros())
}

fn next_power_of_two(value: u64) -> u64 {
    if value <= 1 {
        return 1;
    }
    value.next_power_of_two()
}

fn round_up_to_multiple(value: u64, multiple: u64) -> u64 {
    if multiple <= 1 {
        return value;
    }
    value.div_ceil(multiple).saturating_mul(multiple)
}
