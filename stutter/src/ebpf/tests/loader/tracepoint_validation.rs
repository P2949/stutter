use super::*;

#[test]
fn validates_expected_tracepoint_offsets() {
    let format = r#"
field:char next_comm[16]; offset:40; size:16; signed:1;
field:pid_t next_pid; offset:56; size:4; signed:1;
field:int next_prio; offset:60; size:4; signed:1;
"#;

    validate_tracepoint_format(
        format,
        &[
            TracepointFieldSpec::new("next_comm", 40),
            TracepointFieldSpec::new("next_pid", 56),
            TracepointFieldSpec::new("next_prio", 60),
        ],
    )
    .unwrap();
}

#[test]
fn shared_tracepoint_offset_tables_expose_reader_offsets() {
    use stutter_common::tracepoint_offsets as offsets;

    assert_eq!(
        offsets::SCHED_WAKEUP_FIELDS,
        &[
            TracepointFieldSpec::new("pid", offsets::SCHED_WAKEUP_PID_OFFSET),
            TracepointFieldSpec::new("prio", offsets::SCHED_WAKEUP_PRIO_OFFSET),
            TracepointFieldSpec::new("target_cpu", offsets::SCHED_WAKEUP_TARGET_CPU_OFFSET),
        ],
    );
    assert_eq!(
        offsets::SCHED_SWITCH_FIELDS,
        &[
            TracepointFieldSpec::new("prev_pid", offsets::SCHED_SWITCH_PREV_PID_OFFSET),
            TracepointFieldSpec::new("prev_state", offsets::SCHED_SWITCH_PREV_STATE_OFFSET),
            TracepointFieldSpec::new("next_comm", offsets::SCHED_SWITCH_NEXT_COMM_OFFSET),
            TracepointFieldSpec::new("next_pid", offsets::SCHED_SWITCH_NEXT_PID_OFFSET),
            TracepointFieldSpec::new("next_prio", offsets::SCHED_SWITCH_NEXT_PRIO_OFFSET),
        ],
    );
    assert_eq!(
        offsets::SCHED_MIGRATE_TASK_FIELDS,
        &[
            TracepointFieldSpec::new("pid", offsets::SCHED_MIGRATE_TASK_PID_OFFSET),
            TracepointFieldSpec::new("orig_cpu", offsets::SCHED_MIGRATE_TASK_ORIG_CPU_OFFSET),
            TracepointFieldSpec::new("dest_cpu", offsets::SCHED_MIGRATE_TASK_DEST_CPU_OFFSET),
        ],
    );
    assert_eq!(
        offsets::CPU_FREQUENCY_FIELDS,
        &[
            TracepointFieldSpec::new("state", offsets::CPU_FREQUENCY_STATE_OFFSET),
            TracepointFieldSpec::new("cpu_id", offsets::CPU_FREQUENCY_CPU_ID_OFFSET),
        ],
    );
    assert_eq!(
        offsets::SCHED_STAT_WAIT_FIELDS,
        &[
            TracepointFieldSpec::new("pid", offsets::SCHED_STAT_WAIT_PID_OFFSET),
            TracepointFieldSpec::new("delay", offsets::SCHED_STAT_WAIT_DELAY_OFFSET),
        ],
    );
    assert_eq!(
        offsets::IRQ_HANDLER_FIELDS,
        &[TracepointFieldSpec::new(
            "irq",
            offsets::IRQ_HANDLER_IRQ_OFFSET
        )],
    );
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

    let err = validate_tracepoint_format_named(
        TracepointName::new("sched_switch"),
        format,
        &[TracepointFieldSpec::new("next_pid", 56)],
    )
    .unwrap_err();
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

    let err = validate_tracepoint_format_named(
        TracepointName::new("sched_switch"),
        format,
        &[TracepointFieldSpec::new("next_pid", 56)],
    )
    .unwrap_err();
    let text = err.to_string();

    assert!(text.contains("missing expected field"));
    assert!(text.contains("next_pid"));
    assert!(text.contains("prev_pid"));
    assert!(text.contains("prev_state"));
}

#[test]
fn map_initialization_context_names_missing_and_failed_maps() {
    assert_eq!(
        missing_map_context("EVENTS"),
        "eBPF load failed: EVENTS map not found"
    );
    assert_eq!(
        missing_map_context("DROP_COUNTERS"),
        "eBPF load failed: DROP_COUNTERS map not found"
    );
    assert_eq!(
        map_init_context("TARGET_PIDS"),
        "eBPF load failed: TARGET_PIDS map init"
    );
}

#[test]
fn native_cgroup_filter_is_rejected_until_runtime_verification_exists() {
    let source = include_str!("../../../ebpf/load.rs");

    assert!(source.contains("NativeCgroupFilterUnsupported"));
    assert!(source.contains("Refuse to start a requested-but-inactive native cgroup mode"));
    assert!(!source.contains("NativeCgroupFilterStatus::unverified_directory_inode("));
    assert!(!source.contains("native_cgroup_filter_not_activated"));
}

#[test]
fn sched_switch_uses_consumed_cursor_instead_of_lookup_delete() {
    let source = include_str!("../../../../../stutter-ebpf/src/wakeup_data.rs");

    assert!(source.contains("static WAKEUP_CONSUMED"));
    assert!(source.contains("consume_pending_wakeup"));
    assert!(source.contains("without deleting WAKEUP_DATA"));
}

#[test]
fn wakeup_consumed_cursor_uses_sequence_identity() {
    let source = include_str!("../../../../../stutter-ebpf/src/wakeup_data.rs");

    assert!(source.contains("pub(crate) seq: u32"));
    assert!(source.contains("consumed.seq == data.seq"));
    assert!(source.contains("static WAKEUP_SEQ"));
    assert!(source.contains("WAKEUP_SEQ.remove"));
}

#[test]
fn sched_switch_reads_previous_task_context_after_relevance_filters() {
    let source = include_str!("../../../../../stutter-ebpf/src/scheduler.rs");
    let start = source.find("fn try_sched_switch").unwrap();
    let end = source[start..].find("fn try_sched_migrate_task").unwrap() + start;
    let body = &source[start..end];

    let wakeup_data = body
        .find("wakeup_data::consume_pending_wakeup(pid")
        .unwrap();
    let target_filter = body.find("if !is_target_pid(pid)").unwrap();
    let prev_pid = body.find("let mut prev_pid_raw").unwrap();
    let prev_state = body.find("let mut prev_state").unwrap();

    assert!(prev_pid > wakeup_data);
    assert!(prev_pid > target_filter);
    assert!(prev_state > wakeup_data);
    assert!(prev_state > target_filter);
}

#[test]
fn irq_key_documents_full_u32_irq_cpu_packing_without_overlap_guard() {
    let source = include_str!("../../../../../stutter-ebpf/src/irq.rs");
    let start = source.find("pub(crate) fn irq_key").unwrap();
    let end = source[start..]
        .find("pub(crate) fn try_irq_handler_entry")
        .unwrap()
        + start;
    let body = &source[start..end];

    assert!(body.contains("High 32 bits: IRQ number. Low 32 bits: CPU ID."));
    assert!(body.contains("CPU IDs such as 65_536"));
    assert!(body.contains("((irq as u64) << 32) | cpu as u64"));
    assert!(!body.contains("cpu >= 65536"));
    assert!(!body.contains("cpu > 65535"));
}
