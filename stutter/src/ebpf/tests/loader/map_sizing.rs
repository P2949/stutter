use super::*;

#[test]
fn parses_mem_available_from_proc_meminfo() {
    let meminfo = "MemTotal:       32768000 kB\nMemAvailable:   12345 kB\n";

    assert_eq!(parse_mem_available_bytes(meminfo), Some(12_641_280));
}

#[test]
fn dynamic_map_sizing_grows_when_memory_is_plentiful() {
    let sizing = map_sizing_from_memory(MemorySnapshot {
        locked_memory_limit_bytes: None,
        available_memory_bytes: Some(128 * 1024 * 1024 * 1024),
        page_size: 4096,
    });

    assert_eq!(sizing.events_ringbuf_bytes, MAX_EVENTS_RINGBUF_BYTES);
    assert_eq!(sizing.wakeup_data_entries, MAX_WAKEUP_DATA_ENTRIES);
}

#[test]
fn dynamic_map_sizing_respects_finite_memlock_budget() {
    let sizing = map_sizing_from_memory(MemorySnapshot {
        locked_memory_limit_bytes: Some(1024 * 1024),
        available_memory_bytes: Some(128 * 1024 * 1024 * 1024),
        page_size: 4096,
    });

    assert_eq!(sizing.events_ringbuf_bytes, 256 * 1024);
    assert_eq!(sizing.wakeup_data_entries, 4_096);
}

#[test]
fn ring_buffer_size_is_power_of_two_and_page_aligned() {
    let size = ring_buffer_size_from_budget(900 * 1024, 64 * 1024, 16 * 1024 * 1024, 4096);

    assert_eq!(size, 512 * 1024);
}

#[test]
fn ringbuf_override_applies_and_rounds() {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    let config = match crate::cli::parse_app_command_from([
        "stutter",
        "monitor",
        "--pid",
        "42",
        "--ringbuf-size-kb",
        "1000",
    ])
    .unwrap()
    {
        crate::commands::AppCommand::Monitor(c) => (*c.config).clone(),
        _ => unreachable!(),
    };

    let sizing = map_sizing_for_config(&config);
    // 1000 KB = 1024000 bytes.
    // next_power_of_two(1024000) = 1048576 (1 MiB)
    assert_eq!(sizing.events_ringbuf_bytes, 1024 * 1024);
}

#[test]
fn wakeup_map_factor_applies_and_clamps() {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    let config = match crate::cli::parse_app_command_from([
        "stutter",
        "monitor",
        "--pid",
        "42",
        "--wakeup-map-factor",
        "4",
        "--max-tasks",
        "1000",
    ])
    .unwrap()
    {
        crate::commands::AppCommand::Monitor(c) => (*c.config).clone(),
        _ => unreachable!(),
    };

    let sizing = map_sizing_for_config(&config);
    // 1000 * 4 = 4000.
    // MIN_WAKEUP_DATA_ENTRIES = 4096.
    assert_eq!(sizing.wakeup_data_entries, 4096);

    let config2 = match crate::cli::parse_app_command_from([
        "stutter",
        "monitor",
        "--pid",
        "42",
        "--wakeup-map-factor",
        "10",
        "--max-tasks",
        "1024",
    ])
    .unwrap()
    {
        crate::commands::AppCommand::Monitor(c) => (*c.config).clone(),
        _ => unreachable!(),
    };
    let sizing2 = map_sizing_for_config(&config2);
    // 1024 * 10 = 10240.
    assert_eq!(sizing2.wakeup_data_entries, 10240);
}

#[test]
fn rejects_invalid_map_tuning_values() {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    // ringbuf too small
    let err = crate::cli::parse_app_command_from([
        "stutter",
        "monitor",
        "--pid",
        "42",
        "--ringbuf-size-kb",
        "63",
    ])
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("--ebpf-ringbuf-size-kb must be between 64 and 16384")
    );

    // ringbuf too large
    let err = crate::cli::parse_app_command_from([
        "stutter",
        "monitor",
        "--pid",
        "42",
        "--ringbuf-size-kb",
        "16385",
    ])
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("--ebpf-ringbuf-size-kb must be between 64 and 16384")
    );

    // wakeup factor zero
    let err = crate::cli::parse_app_command_from([
        "stutter",
        "monitor",
        "--pid",
        "42",
        "--wakeup-map-factor",
        "0",
    ])
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("--ebpf-wakeup-map-factor must be between 1 and 64")
    );

    // wakeup factor too large
    let err = crate::cli::parse_app_command_from([
        "stutter",
        "monitor",
        "--pid",
        "42",
        "--wakeup-map-factor",
        "65",
    ])
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("--ebpf-wakeup-map-factor must be between 1 and 64")
    );
}
