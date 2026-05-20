//! Generic tests for eBPF loader configuration and object loading behavior.
//!
//! Owns loader regression tests and test-only fixtures. Does not own production object loading,
//! tracepoint attach, map sizing, or preflight logic.

use std::{
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

// tokio::time::sleep removed as unused
use super::*;

#[test]
fn parses_tracepoint_field_offsets() {
    let format = r#"
field:unsigned short common_type; offset:0; size:2; signed:0;
field:char prev_comm[16]; offset:8; size:16; signed:1;
field:pid_t prev_pid; offset:24; size:4; signed:1;
field:int prev_prio; offset:28; size:4; signed:1;
field:long prev_state; offset:32; size:8; signed:1;
field:char next_comm[16]; offset:40; size:16; signed:1;
field:pid_t next_pid; offset:56; size:4; signed:1;
field:int next_prio; offset:60; size:4; signed:1;
"#;

    let offsets = parse_tracepoint_offsets(format);

    assert_eq!(offsets.get("next_comm").map(|f| f.offset), Some(40));
    assert_eq!(offsets.get("next_pid").map(|f| f.offset), Some(56));
    assert_eq!(offsets.get("next_prio").map(|f| f.offset), Some(60));
    assert_eq!(offsets.get("next_comm").map(|f| f.size), Some(16));
    assert_eq!(offsets.get("next_pid").map(|f| f.size), Some(4));
    assert_eq!(offsets.get("next_prio").map(|f| f.size), Some(4));
}

#[test]
fn parse_tracepoint_fields_preserves_original_declaration() {
    let format = "    field:char next_comm[16]; offset:40; size:16; signed:1;\n";

    let fields = parse_tracepoint_offsets(format);
    let field = fields.get("next_comm").unwrap();

    assert_eq!(field.name, "next_comm");
    assert_eq!(field.offset, 40);
    assert_eq!(field.size, 16);
    assert!(field.signed);
    assert_eq!(
        field.declaration,
        "field:char next_comm[16]; offset:40; size:16; signed:1;",
    );
}

#[test]
fn request_pointer_key_requires_matching_issue_and_complete_offsets() {
    let issue_offsets =
        parse_tracepoint_offsets("field:struct request *rq; offset:40; size:8; signed:0;");
    let complete_offsets =
        parse_tracepoint_offsets("field:struct request *rq; offset:40; size:8; signed:0;");

    assert_eq!(
        matching_request_key_offset(&issue_offsets, &complete_offsets),
        Some(40),
    );
}

#[test]
fn request_pointer_key_rejects_mismatched_or_missing_complete_offset() {
    let issue_offsets =
        parse_tracepoint_offsets("field:struct request *rq; offset:40; size:8; signed:0;");
    let mismatched_complete_offsets =
        parse_tracepoint_offsets("field:struct request *rq; offset:48; size:8; signed:0;");
    let missing_complete_offsets =
        parse_tracepoint_offsets("field:dev_t dev; offset:8; size:4; signed:0;");

    assert_eq!(
        matching_request_key_offset(&issue_offsets, &mismatched_complete_offsets),
        None,
    );
    assert_eq!(
        matching_request_key_offset(&issue_offsets, &missing_complete_offsets),
        None,
    );
}

#[test]
fn request_pointer_key_rejects_wrong_size() {
    let issue_offsets = parse_tracepoint_offsets("field:u32 rq; offset:40; size:4; signed:0;");
    let complete_offsets = parse_tracepoint_offsets("field:u32 rq; offset:40; size:4; signed:0;");

    assert_eq!(
        matching_request_key_offset(&issue_offsets, &complete_offsets),
        None,
    );
}

#[test]
fn validates_expected_tracepoint_offsets() {
    let format = r#"
field:char next_comm[16]; offset:40; size:16; signed:1;
field:pid_t next_pid; offset:56; size:4; signed:1;
field:int next_prio; offset:60; size:4; signed:1;
"#;

    validate_tracepoint_format(
        format,
        &[("next_comm", 40), ("next_pid", 56), ("next_prio", 60)],
    )
    .unwrap();
}

#[test]
fn tracepoint_mismatch_error_includes_declaration_and_sched_switch_hint() {
    let format = r#"
    field:char prev_comm[16]; offset:8; size:16; signed:1;
    field:pid_t prev_pid; offset:24; size:4; signed:1;
    field:int prev_prio; offset:28; size:4; signed:1;
    field:int prev_state; offset:32; size:4; signed:1;
    field:char next_comm[16]; offset:36; size:16; signed:1;
    field:pid_t next_pid; offset:52; size:4; signed:1;
    field:int next_prio; offset:56; size:4; signed:1;
"#;

    let err =
        validate_tracepoint_format_named("sched_switch", format, &[("next_pid", 56)]).unwrap_err();
    let text = err.to_string();

    assert!(text.contains("sched_switch"));
    assert!(text.contains("next_pid"));
    assert!(text.contains("expected offset 56"));
    assert!(text.contains("got 52"));
    assert!(text.contains("field:pid_t next_pid; offset:52; size:4; signed:1;"));
    assert!(text.contains("prev_state"));
    assert!(text.contains("rejects this layout"));
}

#[test]
fn tracepoint_missing_field_error_lists_available_fields() {
    let format = r#"
field:pid_t prev_pid; offset:24; size:4; signed:1;
field:long prev_state; offset:32; size:8; signed:1;
"#;

    let err =
        validate_tracepoint_format_named("sched_switch", format, &[("next_pid", 56)]).unwrap_err();
    let text = err.to_string();

    assert!(text.contains("missing expected field"));
    assert!(text.contains("next_pid"));
    assert!(text.contains("prev_pid"));
    assert!(text.contains("prev_state"));
}

#[test]
fn validates_irq_tracepoint_offsets() {
    let format = r#"
field:unsigned short common_type; offset:0; size:2; signed:0;
field:int irq; offset:8; size:4; signed:1;
"#;

    validate_tracepoint_format(format, &[("irq", 8)]).unwrap();
}

#[test]
fn rejects_bad_irq_tracepoint_offsets() {
    let format = "field:int irq; offset:12; size:4; signed:1;";

    let err = validate_tracepoint_format(format, &[("irq", 8)]).unwrap_err();
    assert!(err.to_string().contains("expected offset 8, got 12"));
}

const IRQ_HANDLER_EXIT_FORMAT_WITH_RET: &str = r#"
name: irq_handler_exit
ID: 1234
format:
	field:unsigned short common_type;	offset:0;	size:2;	signed:0;
	field:unsigned char common_flags;	offset:2;	size:1;	signed:0;
	field:unsigned char common_preempt_count;	offset:3;	size:1;	signed:0;
	field:int common_pid;	offset:4;	size:4;	signed:1;

	field:int irq;	offset:8;	size:4;	signed:1;
	field:int ret;	offset:12;	size:4;	signed:1;

print fmt: "irq=%d ret=%d", REC->irq, REC->ret
"#;

const IRQ_HANDLER_EXIT_FORMAT_MISSING_RET: &str = r#"
name: irq_handler_exit
ID: 1234
format:
	field:unsigned short common_type;	offset:0;	size:2;	signed:0;
	field:int common_pid;	offset:4;	size:4;	signed:1;
	field:int irq;	offset:8;	size:4;	signed:1;
"#;

#[test]
fn parse_tracepoint_field_offset_finds_irq_and_ret() {
    assert_eq!(
        parse_tracepoint_field_offset(IRQ_HANDLER_EXIT_FORMAT_WITH_RET, "irq").unwrap(),
        8
    );

    assert_eq!(
        parse_tracepoint_field_offset(IRQ_HANDLER_EXIT_FORMAT_WITH_RET, "ret").unwrap(),
        12
    );
}

#[test]
fn parse_tracepoint_field_offset_errors_when_ret_missing() {
    let err =
        parse_tracepoint_field_offset(IRQ_HANDLER_EXIT_FORMAT_MISSING_RET, "ret").unwrap_err();

    assert!(err.to_string().contains("ret"));
}

#[test]
fn parse_tracepoint_field_offset_matches_exact_field_name() {
    let format = r#"
	field:int return_code;	offset:8;	size:4;	signed:1;
	field:int ret;	offset:12;	size:4;	signed:1;
"#;

    assert_eq!(parse_tracepoint_field_offset(format, "ret").unwrap(), 12);
}

#[test]
fn optional_tracepoint_format_missing_is_not_an_error() {
    let dir = temp_dir("optional-tracepoint");
    fs::create_dir_all(&dir).unwrap();

    let available = validate_optional_tracepoint_format_at(
        &dir.join("missing/format"),
        "missing",
        &[("pid", 24)],
        true,
    )
    .unwrap();

    assert!(!available);
    fs::remove_dir_all(dir).ok();
}

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
    assert_eq!(sizing.wakeup_data_entries, 8_192);
}

#[test]
fn ring_buffer_size_is_power_of_two_and_page_aligned() {
    let size = ring_buffer_size_from_budget(900 * 1024, 64 * 1024, 16 * 1024 * 1024, 4096);

    assert_eq!(size, 512 * 1024);
}

#[test]
fn gates_optional_tracepoint_validation_by_config() {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    let dir = temp_dir("gate-validation");

    // Required tracepoints must exist for validate_tracepoint_formats to succeed
    let sched_wakeup = dir.join("sched/sched_wakeup");
    fs::create_dir_all(&sched_wakeup).unwrap();
    fs::write(
            sched_wakeup.join("format"),
            "field:pid_t pid; offset:24; size:4; signed:1;\nfield:int prio; offset:28; size:4; signed:1;\nfield:int target_cpu; offset:32; size:4; signed:1;",
        ).unwrap();

    let sched_switch = dir.join("sched/sched_switch");
    fs::create_dir_all(&sched_switch).unwrap();
    fs::write(
            sched_switch.join("format"),
            "field:char prev_comm[16]; offset:8; size:16; signed:1;\nfield:pid_t prev_pid; offset:24; size:4; signed:1;\nfield:int prev_prio; offset:28; size:4; signed:1;\nfield:long prev_state; offset:32; size:8; signed:1;\nfield:char next_comm[16]; offset:40; size:16; signed:1;\nfield:pid_t next_pid; offset:56; size:4; signed:1;\nfield:int next_prio; offset:60; size:4; signed:1;",
        ).unwrap();

    let sched_process_exit = dir.join("sched/sched_process_exit");
    fs::create_dir_all(&sched_process_exit).unwrap();
    fs::write(
        sched_process_exit.join("format"),
        "field:pid_t pid; offset:12; size:4; signed:1;",
    )
    .unwrap();

    // Create a fake format file with WRONG offset for cpu_frequency
    let cpu_freq_dir = dir.join("power/cpu_frequency");
    fs::create_dir_all(&cpu_freq_dir).unwrap();
    fs::write(
            cpu_freq_dir.join("format"),
            "field:int state; offset:99; size:4; signed:1;\nfield:int cpu_id; offset:103; size:4; signed:1;",
        ).unwrap();

    // Create a fake format file with WRONG offset for sched_stat_wait
    let stat_wait_dir = dir.join("sched/sched_stat_wait");
    fs::create_dir_all(&stat_wait_dir).unwrap();
    fs::write(
            stat_wait_dir.join("format"),
            "field:pid_t pid; offset:99; size:4; signed:1;\nfield:u64 delay; offset:103; size:8; signed:0;",
        ).unwrap();

    // Create a fake format file with WRONG offset for IRQ
    let irq_entry_dir = dir.join("irq/irq_handler_entry");
    fs::create_dir_all(&irq_entry_dir).unwrap();
    fs::write(
        irq_entry_dir.join("format"),
        "field:int irq; offset:99; size:4; signed:1;",
    )
    .unwrap();
    let irq_exit_dir = dir.join("irq/irq_handler_exit");
    fs::create_dir_all(&irq_exit_dir).unwrap();
    fs::write(
        irq_exit_dir.join("format"),
        "field:int irq; offset:99; size:4; signed:1;",
    )
    .unwrap();

    let mut config =
        match crate::cli::parse_app_command_from(["stutter", "monitor", "--pid", "42"]).unwrap() {
            crate::commands::AppCommand::Monitor(c) => (*c.config).clone(),
            _ => unreachable!(),
        };

    // Validating with optional features DISABLED should SUCCEED even with wrong formats
    config.probes.cpu_freq = false;
    config.probes.stat_wait = false;
    config.probes.irq_latency = false;
    config.probes.block_io = false;

    let availability = validate_tracepoint_formats(&dir, &config).unwrap();
    assert!(!availability.cpu_frequency);
    assert!(!availability.sched_stat_wait);
    assert!(!availability.irq_handler);

    // Validating with cpu_freq = true should FAIL
    config.probes.cpu_freq = true;
    let err = validate_tracepoint_formats(&dir, &config).unwrap_err();
    assert!(err.to_string().contains("cpu_frequency"));
    config.probes.cpu_freq = false;

    // Validating with stat_wait = true should FAIL
    config.probes.stat_wait = true;
    let err = validate_tracepoint_formats(&dir, &config).unwrap_err();
    assert!(err.to_string().contains("sched_stat_wait"));
    config.probes.stat_wait = false;

    // Validating with irq_latency = true should FAIL
    config.probes.irq_latency = true;
    // irq_latency also requires --irq N in CLI, but validate_tracepoint_formats
    // only cares about the irq_latency flag and existence of files.
    let err = validate_tracepoint_formats(&dir, &config).unwrap_err();
    assert!(err.to_string().contains("irq_handler_entry"));
    config.probes.irq_latency = false;

    fs::remove_dir_all(dir).ok();
}

#[test]
fn validates_sched_process_exit_availability() {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    let dir = temp_dir("process-exit");

    // Required tracepoints
    let sched_wakeup = dir.join("sched/sched_wakeup");
    fs::create_dir_all(&sched_wakeup).unwrap();
    fs::write(
            sched_wakeup.join("format"),
            "field:pid_t pid; offset:24; size:4; signed:1;\nfield:int prio; offset:28; size:4; signed:1;\nfield:int target_cpu; offset:32; size:4; signed:1;",
        ).unwrap();

    let sched_switch = dir.join("sched/sched_switch");
    fs::create_dir_all(&sched_switch).unwrap();
    fs::write(
            sched_switch.join("format"),
            "field:char prev_comm[16]; offset:8; size:16; signed:1;\nfield:pid_t prev_pid; offset:24; size:4; signed:1;\nfield:int prev_prio; offset:28; size:4; signed:1;\nfield:long prev_state; offset:32; size:8; signed:1;\nfield:char next_comm[16]; offset:40; size:16; signed:1;\nfield:pid_t next_pid; offset:56; size:4; signed:1;\nfield:int next_prio; offset:60; size:4; signed:1;",
        ).unwrap();

    let config =
        match crate::cli::parse_app_command_from(["stutter", "monitor", "--pid", "42"]).unwrap() {
            crate::commands::AppCommand::Monitor(c) => (*c.config).clone(),
            _ => unreachable!(),
        };

    // Case 1: sched/sched_process_exit/format missing
    let availability = validate_tracepoint_formats(&dir, &config).unwrap();
    assert!(!availability.sched_process_exit);

    // Case 2: sched/sched_process_exit/format present
    let sched_process_exit = dir.join("sched/sched_process_exit");
    fs::create_dir_all(&sched_process_exit).unwrap();
    fs::write(
        sched_process_exit.join("format"),
        "field:pid_t pid; offset:12; size:4; signed:1;",
    )
    .unwrap();

    let availability = validate_tracepoint_formats(&dir, &config).unwrap();
    assert!(availability.sched_process_exit);

    fs::remove_dir_all(dir).ok();
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
        "2000",
    ])
    .unwrap()
    {
        crate::commands::AppCommand::Monitor(c) => (*c.config).clone(),
        _ => unreachable!(),
    };
    let sizing2 = map_sizing_for_config(&config2);
    // 2000 * 10 = 20000.
    assert_eq!(sizing2.wakeup_data_entries, 20000);
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
            .contains("--ringbuf-size-kb must be between 64 and 16384")
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
            .contains("--ringbuf-size-kb must be between 64 and 16384")
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
            .contains("--wakeup-map-factor must be between 1 and 64")
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
            .contains("--wakeup-map-factor must be between 1 and 64")
    );
}

fn temp_dir(name: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "stutter-{name}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    dir
}

#[test]
fn read_prebuilt_bpf_object_reads_non_empty_file() {
    let dir = temp_dir("prebuilt-bpf");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("stutter.bpf.o");

    fs::write(&path, b"fake-bpf-object").unwrap();

    let bytes = read_prebuilt_bpf_object(&path).unwrap();
    assert_eq!(bytes, b"fake-bpf-object");

    fs::remove_dir_all(dir).ok();
}

#[test]
fn read_prebuilt_bpf_object_rejects_empty_file() {
    let dir = temp_dir("prebuilt-bpf-empty");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("stutter.bpf.o");

    fs::write(&path, b"").unwrap();

    let err = read_prebuilt_bpf_object(&path).unwrap_err();
    assert!(err.to_string().contains("empty"));

    fs::remove_dir_all(dir).ok();
}

#[test]
fn read_prebuilt_bpf_object_rejects_missing_file() {
    let dir = temp_dir("prebuilt-bpf-missing");
    let path = dir.join("nonexistent.bpf.o");

    let err = read_prebuilt_bpf_object(&path).unwrap_err();
    assert!(err.to_string().contains("failed to read"));
}
