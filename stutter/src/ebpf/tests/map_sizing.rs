//! Tests for eBPF map sizing and memory-limit behavior.
//!
//! Owns map sizing regression tests. Does not own production eBPF map sizing, loader, preflight,
//! or tracepoint validation logic.

use super::*;

const TEST_PAGE_SIZE: u64 = 4096;

fn memory_snapshot(
    locked_memory_limit_bytes: Option<u64>,
    available_memory_bytes: Option<u64>,
) -> MemorySnapshot {
    MemorySnapshot {
        locked_memory_limit_bytes,
        available_memory_bytes,
        page_size: TEST_PAGE_SIZE,
    }
}

#[test]
fn low_memlock_budget_clamps_wakeup_entries_to_minimum() {
    let sizing = map_sizing_from_memory(memory_snapshot(Some(128 * 1024), Some(1 << 30)));

    assert_eq!(sizing.events_ringbuf_bytes, MIN_EVENTS_RINGBUF_BYTES);
    assert_eq!(sizing.wakeup_data_entries, MIN_WAKEUP_DATA_ENTRIES);
    assert_eq!(sizing.locked_memory_limit_bytes, Some(128 * 1024));
    assert_eq!(sizing.available_memory_bytes, Some(1 << 30));
}

#[test]
fn unknown_or_unlimited_memory_uses_default_available_memory_budget() {
    let sizing = map_sizing_from_memory(memory_snapshot(None, None));

    assert_eq!(sizing.events_ringbuf_bytes, 4 * 1024 * 1024);
    assert_eq!(sizing.wakeup_data_entries, 196_608);
    assert_eq!(sizing.locked_memory_limit_bytes, None);
    assert_eq!(sizing.available_memory_bytes, None);
}

#[test]
fn memlock_limit_bytes_treats_rlim_infinity_as_unknown_or_unlimited() {
    assert_eq!(memlock_limit_bytes_from_rlim(libc::RLIM_INFINITY), None);
}

#[test]
fn map_sizing_report_includes_target_and_wakeup_capacities() {
    let report = ebpf_map_sizing_report();
    let value = serde_json::to_value(&report).unwrap();

    assert_eq!(
        value
            .get("target_pids_max")
            .and_then(serde_json::Value::as_u64),
        Some(TARGET_PIDS_MAX as u64)
    );
    assert_eq!(
        value
            .get("min_events_ringbuf_bytes")
            .and_then(serde_json::Value::as_u64),
        Some(MIN_EVENTS_RINGBUF_BYTES as u64)
    );
    assert_eq!(
        value
            .get("max_events_ringbuf_bytes")
            .and_then(serde_json::Value::as_u64),
        Some(MAX_EVENTS_RINGBUF_BYTES as u64)
    );
    assert_eq!(
        value
            .get("default_events_ringbuf_bytes")
            .and_then(serde_json::Value::as_u64),
        Some(stutter_common::BPF_DEFAULT_EVENTS_RINGBUF_BYTES as u64)
    );
    assert_eq!(
        value
            .get("max_tracked_cpus")
            .and_then(serde_json::Value::as_u64),
        Some(stutter_common::BPF_MAX_TRACKED_CPUS as u64)
    );
    assert!(
        value
            .get("wakeup_data_entries")
            .and_then(serde_json::Value::as_u64)
            .is_some()
    );
    assert_eq!(
        value
            .get("wakeup_data_map_entry_budget_bytes")
            .and_then(serde_json::Value::as_u64),
        Some(WAKEUP_DATA_MAP_ENTRY_BUDGET_BYTES)
    );
    assert_eq!(
        value
            .get("min_wakeup_data_entries")
            .and_then(serde_json::Value::as_u64),
        Some(MIN_WAKEUP_DATA_ENTRIES as u64)
    );
    assert_eq!(
        value
            .get("max_wakeup_data_entries")
            .and_then(serde_json::Value::as_u64),
        Some(MAX_WAKEUP_DATA_ENTRIES as u64)
    );
}

#[test]
fn drop_counter_serializes_wakeup_failures_as_lost_wakeup_timestamps() {
    let snapshot = DropCountersSnapshot {
        wakeup_data_insert_failed: 7,
        wakeup_data_stale_entries: 0,
        wakeup_data_replaced_entries: 0,
        wakeup_data_consumed_read_failed: 0,
        ringbuf_reserve_failed: 0,
        irq_start_times_insert_failed: 0,
        block_start_insert_failed: 0,
        block_fallback_key_collisions: 0,
        cpu_accounting_untracked: 0,
    };

    let value = serde_json::to_value(&snapshot).unwrap();

    assert_eq!(
        value
            .get("lost_wakeup_timestamp_inserts")
            .and_then(serde_json::Value::as_u64),
        Some(7)
    );
    assert!(value.get("wakeup_data_insert_failed").is_none());
}

#[test]
fn drop_counter_reads_legacy_wakeup_data_insert_failed_name() {
    let snapshot: DropCountersSnapshot = serde_json::from_value(serde_json::json!({
        "wakeup_data_insert_failed": 9,
        "ringbuf_reserve_failed": 0,
        "irq_start_times_insert_failed": 0,
        "block_start_insert_failed": 0
    }))
    .unwrap();

    assert_eq!(snapshot.wakeup_data_insert_failed, 9);
    assert_eq!(snapshot.wakeup_data_stale_entries, 0);
}

#[test]
fn drop_counter_serializes_stale_wakeup_entries() {
    let snapshot = DropCountersSnapshot {
        wakeup_data_insert_failed: 0,
        wakeup_data_stale_entries: 11,
        wakeup_data_replaced_entries: 0,
        wakeup_data_consumed_read_failed: 0,
        ringbuf_reserve_failed: 0,
        irq_start_times_insert_failed: 0,
        block_start_insert_failed: 0,
        block_fallback_key_collisions: 0,
        cpu_accounting_untracked: 0,
    };

    let value = serde_json::to_value(&snapshot).unwrap();

    assert_eq!(
        value
            .get("wakeup_data_stale_entries")
            .and_then(serde_json::Value::as_u64),
        Some(11)
    );
}

#[test]
fn drop_counter_totals_include_stale_wakeup_entries() {
    let snapshot = DropCountersSnapshot {
        wakeup_data_insert_failed: 1,
        wakeup_data_stale_entries: 2,
        wakeup_data_replaced_entries: 0,
        wakeup_data_consumed_read_failed: 0,
        ringbuf_reserve_failed: 4,
        irq_start_times_insert_failed: 8,
        block_start_insert_failed: 16,
        block_fallback_key_collisions: 32,
        cpu_accounting_untracked: 0,
    };

    assert_eq!(snapshot.total(), 63);
    assert_eq!(snapshot.total_excluding_block_io(), 15);
}

#[test]
fn drop_counter_totals_include_new_wakeup_and_cpu_accounting_counters() {
    let snapshot = DropCountersSnapshot {
        wakeup_data_insert_failed: 0,
        wakeup_data_stale_entries: 0,
        wakeup_data_replaced_entries: 3,
        wakeup_data_consumed_read_failed: 5,
        ringbuf_reserve_failed: 0,
        irq_start_times_insert_failed: 0,
        block_start_insert_failed: 0,
        block_fallback_key_collisions: 7,
        cpu_accounting_untracked: 11,
    };

    assert_eq!(snapshot.total(), 26);
    assert_eq!(snapshot.total_excluding_block_io(), 19);
}

#[test]
fn map_sizing_after_failed_memlock_raise_uses_after_limit() {
    let report = MemlockPolicyReport {
        before_limit_bytes: Some(128 * 1024),
        after_limit_bytes: Some(128 * 1024),
        raise_attempted: true,
        raise_succeeded: false,
        raise_error: Some("operation not permitted".to_owned()),
    };

    let sizing = map_sizing_from_memory(memory_snapshot(report.after_limit_bytes, Some(1 << 30)));

    assert_eq!(sizing.events_ringbuf_bytes, MIN_EVENTS_RINGBUF_BYTES);
    assert_eq!(sizing.wakeup_data_entries, MIN_WAKEUP_DATA_ENTRIES);
    assert_eq!(sizing.locked_memory_limit_bytes, Some(128 * 1024));
}

#[test]
fn map_sizing_after_unlimited_memlock_uses_available_memory_budget() {
    let report = MemlockPolicyReport {
        before_limit_bytes: None,
        after_limit_bytes: None,
        raise_attempted: false,
        raise_succeeded: false,
        raise_error: None,
    };

    let sizing = map_sizing_from_memory(memory_snapshot(report.after_limit_bytes, Some(1 << 30)));

    assert_eq!(sizing.events_ringbuf_bytes, 4 * 1024 * 1024);
    assert_eq!(sizing.wakeup_data_entries, 196_608);
    assert_eq!(sizing.locked_memory_limit_bytes, None);
}

#[test]
fn map_sizing_after_unknown_memlock_uses_available_memory_budget() {
    let report = MemlockPolicyReport {
        before_limit_bytes: None,
        after_limit_bytes: None,
        raise_attempted: false,
        raise_succeeded: false,
        raise_error: Some("failed to read RLIMIT_MEMLOCK before raise".to_owned()),
    };

    let sizing = map_sizing_from_memory(memory_snapshot(report.after_limit_bytes, Some(1 << 30)));

    assert_eq!(sizing.events_ringbuf_bytes, 4 * 1024 * 1024);
    assert_eq!(sizing.wakeup_data_entries, 196_608);
    assert_eq!(sizing.locked_memory_limit_bytes, None);
}

#[test]
fn very_high_available_memory_clamps_wakeup_entries_to_maximum() {
    let sizing = map_sizing_from_memory(memory_snapshot(None, Some(1u64 << 40)));

    assert_eq!(sizing.events_ringbuf_bytes, MAX_EVENTS_RINGBUF_BYTES);
    assert_eq!(sizing.wakeup_data_entries, MAX_WAKEUP_DATA_ENTRIES);
}

#[test]
fn explicit_wakeup_map_factor_uses_max_tasks_times_factor() {
    let entries = wakeup_data_entries_for_config(MIN_WAKEUP_DATA_ENTRIES, 10_000, Some(4));

    assert_eq!(entries, 40_000);
}

#[test]
fn explicit_wakeup_map_factor_is_clamped_to_minimum() {
    let entries = wakeup_data_entries_for_config(1, 1, Some(0));

    assert_eq!(entries, MIN_WAKEUP_DATA_ENTRIES);
}

#[test]
fn explicit_wakeup_map_factor_is_clamped_to_maximum() {
    let entries = wakeup_data_entries_for_config(
        MIN_WAKEUP_DATA_ENTRIES,
        MAX_WAKEUP_DATA_ENTRIES as usize,
        Some(2),
    );

    assert_eq!(entries, MAX_WAKEUP_DATA_ENTRIES);
}

#[test]
fn automatic_sizing_uses_at_least_configured_max_tasks() {
    let entries = wakeup_data_entries_for_config(MIN_WAKEUP_DATA_ENTRIES, 200_000, None);

    assert_eq!(entries, 200_000);
}

#[test]
fn automatic_sizing_clamps_configured_max_tasks_to_maximum() {
    let entries = wakeup_data_entries_for_config(
        MIN_WAKEUP_DATA_ENTRIES,
        (MAX_WAKEUP_DATA_ENTRIES as usize).saturating_add(1),
        None,
    );

    assert_eq!(entries, MAX_WAKEUP_DATA_ENTRIES);
}

#[test]
fn automatic_sizing_still_clamps_tiny_results_to_minimum() {
    let entries = wakeup_data_entries_for_config(1, 1, None);

    assert_eq!(entries, MIN_WAKEUP_DATA_ENTRIES);
}
