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
use crate::{
    config::model::MonitorConfig,
    ebpf::{
        attach::{AttachOps, FaultPerfProbe, attach_kms_tracepoints},
        load::{
            attach_optional_scheduler_tracepoints, attach_required_scheduler_tracepoints,
            map_init_context, missing_map_context,
        },
        preflight::TracepointAvailability,
    },
    probe_activation::ProbeActivationPlan,
    session::targeting::TargetPolicy,
};

#[derive(Default)]
struct FakeAttachOps {
    fail_program: Option<&'static str>,
    tracepoint_calls: Vec<(&'static str, String, String)>,
    perf_calls: Vec<(&'static str, FaultPerfProbe)>,
}

impl FakeAttachOps {
    fn fail_program(program: &'static str) -> Self {
        Self {
            fail_program: Some(program),
            ..Self::default()
        }
    }
}

impl AttachOps for FakeAttachOps {
    fn attach_tracepoint(
        &mut self,
        program_name: &'static str,
        category: &str,
        tracepoint_name: &str,
    ) -> anyhow::Result<()> {
        self.tracepoint_calls.push((
            program_name,
            category.to_owned(),
            tracepoint_name.to_owned(),
        ));

        if self.fail_program == Some(program_name) {
            anyhow::bail!("{program_name} failed for test");
        }

        Ok(())
    }

    fn attach_perf_event(
        &mut self,
        program_name: &'static str,
        probe: FaultPerfProbe,
    ) -> anyhow::Result<()> {
        self.perf_calls.push((program_name, probe));

        if self.fail_program == Some(program_name) {
            anyhow::bail!("{program_name} failed for test");
        }

        Ok(())
    }
}

fn attach_test_tracepoints() -> TracepointAvailability {
    TracepointAvailability {
        sched_wakeup_new: true,
        sched_migrate_task: true,
        cpu_frequency: false,
        sched_stat_wait: false,
        irq_handler: false,
        block_rq: false,
        block_rq_has_rwbs: false,
        block_rq_key_offset: None,
        block_rq_issue_nr_sector_offset: None,
        block_rq_issue_rwbs_offset: None,
        block_rq_complete_nr_sector_offset: None,
        block_rq_complete_rwbs_offset: None,
        kms: crate::drm_tracepoints::KmsTracepointAvailability::unavailable(),
        drm_fence: None,
        sched_process_exit: true,
        sched_process_exec: true,
    }
}

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
fn shared_tracepoint_offset_tables_expose_reader_offsets() {
    use stutter_common::tracepoint_offsets as offsets;

    assert_eq!(
        offsets::SCHED_WAKEUP_FIELDS,
        &[
            ("pid", offsets::SCHED_WAKEUP_PID_OFFSET),
            ("prio", offsets::SCHED_WAKEUP_PRIO_OFFSET),
            ("target_cpu", offsets::SCHED_WAKEUP_TARGET_CPU_OFFSET),
        ],
    );
    assert_eq!(
        offsets::SCHED_SWITCH_FIELDS,
        &[
            ("prev_pid", offsets::SCHED_SWITCH_PREV_PID_OFFSET),
            ("prev_state", offsets::SCHED_SWITCH_PREV_STATE_OFFSET),
            ("next_comm", offsets::SCHED_SWITCH_NEXT_COMM_OFFSET),
            ("next_pid", offsets::SCHED_SWITCH_NEXT_PID_OFFSET),
            ("next_prio", offsets::SCHED_SWITCH_NEXT_PRIO_OFFSET),
        ],
    );
    assert_eq!(
        offsets::SCHED_MIGRATE_TASK_FIELDS,
        &[
            ("pid", offsets::SCHED_MIGRATE_TASK_PID_OFFSET),
            ("orig_cpu", offsets::SCHED_MIGRATE_TASK_ORIG_CPU_OFFSET),
            ("dest_cpu", offsets::SCHED_MIGRATE_TASK_DEST_CPU_OFFSET),
        ],
    );
    assert_eq!(
        offsets::CPU_FREQUENCY_FIELDS,
        &[
            ("state", offsets::CPU_FREQUENCY_STATE_OFFSET),
            ("cpu_id", offsets::CPU_FREQUENCY_CPU_ID_OFFSET),
        ],
    );
    assert_eq!(
        offsets::SCHED_STAT_WAIT_FIELDS,
        &[
            ("pid", offsets::SCHED_STAT_WAIT_PID_OFFSET),
            ("delay", offsets::SCHED_STAT_WAIT_DELAY_OFFSET),
        ],
    );
    assert_eq!(
        offsets::IRQ_HANDLER_FIELDS,
        &[("irq", offsets::IRQ_HANDLER_IRQ_OFFSET)],
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
fn loader_overrides_wakeup_data_and_consumed_cursor_maps_together() {
    let source = include_str!("../../ebpf/load.rs");

    assert!(source.contains("map_max_entries(\"WAKEUP_DATA\", map_sizing.wakeup_data_entries)"));
    assert!(
        source.contains("map_max_entries(\"WAKEUP_CONSUMED\", map_sizing.wakeup_data_entries)")
    );
}

#[test]
fn scheduler_optional_tracepoint_attach_failures_degrade_through_activation_warnings() {
    let source = include_str!("../../ebpf/load.rs");

    for program in [
        "sched_wakeup_new",
        "sched_process_exit",
        "sched_migrate_task",
    ] {
        let marker = format!("\"{program}\"");
        let start = source
            .find(&marker)
            .unwrap_or_else(|| panic!("{program} attach block not found"));
        let end = source.len().min(start + 1_200);
        let body = &source[start..end];

        assert!(body.contains("activation_plan.push_tracepoint_attach_warning"));
        assert!(body.contains("ProbeKey::SchedulerRunnableLatency"));
        assert!(body.contains("optional_probe_attach_failed"));
        assert!(!body.contains("context(\"eBPF load failed: attach"));
    }
}

#[test]
fn required_sched_wakeup_attach_failure_aborts_load_plan() {
    let mut fake = FakeAttachOps::fail_program("sched_wakeup");

    let err = attach_required_scheduler_tracepoints(&mut fake).unwrap_err();

    assert!(err.to_string().contains("sched_wakeup"));
    assert_eq!(fake.tracepoint_calls.len(), 1);
    assert_eq!(fake.tracepoint_calls[0].0, "sched_wakeup");
}

#[test]
fn optional_sched_wakeup_new_attach_failure_records_warning_and_continues() {
    let config = MonitorConfig::default();
    let mut plan = ProbeActivationPlan::from_config(&config, &attach_test_tracepoints()).unwrap();
    let mut fake = FakeAttachOps::fail_program("sched_wakeup_new");

    attach_optional_scheduler_tracepoints(&mut fake, &mut plan);

    assert!(
        fake.tracepoint_calls
            .iter()
            .any(|(program, _, _)| *program == "sched_process_exit"),
    );
    assert!(plan.warnings.iter().any(|warning| {
        warning.message.contains("sched/sched_wakeup_new")
            && warning.message.contains("sched_wakeup_new")
    }));
}

#[test]
fn kms_optional_attach_warning_names_actual_tracepoint() {
    let mut plan =
        ProbeActivationPlan::from_config(&MonitorConfig::default(), &attach_test_tracepoints())
            .unwrap();
    let kms = crate::drm_tracepoints::KmsTracepointAvailability {
        pageflip_request: None,
        pageflip_done: None,
        vblank_event: Some(crate::drm_tracepoints::parse_drm_tracepoint_format(
            "drm",
            "drm_vblank_event",
            "field:unsigned int crtc_id;\toffset:8;\tsize:4;\tsigned:0;\n",
        )),
        atomic_commit: None,
        provider: crate::drm_tracepoints::KmsTracepointProvider::GenericDrm,
        generic_drm: Vec::new(),
        i915: Vec::new(),
        amdgpu: Vec::new(),
        warnings: Vec::new(),
    };
    let mut fake = FakeAttachOps::fail_program("drm_vblank_event");

    attach_kms_tracepoints(&mut fake, &mut plan, &kms);

    assert!(plan.warnings.iter().any(|warning| {
        warning.message.contains("drm/drm_vblank_event")
            && warning.message.contains("drm_vblank_event")
    }));
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
#[ignore = "requires Linux tracefs and eBPF privileges"]
fn load_and_attach_real_bpf_object_smoke_test() {
    let config = MonitorConfig::default();
    let target_policy = TargetPolicy::from_monitor_config(&config).unwrap();

    let _loaded = crate::ebpf::load::load_and_attach(&config, &target_policy).unwrap();
}

#[test]
fn native_cgroup_filter_is_rejected_until_runtime_verification_exists() {
    let source = include_str!("../../ebpf/load.rs");

    assert!(source.contains("NativeCgroupFilterUnsupported"));
    assert!(source.contains("Refuse to start a requested-but-inactive native cgroup mode"));
    assert!(!source.contains("NativeCgroupFilterStatus::unverified_directory_inode("));
    assert!(!source.contains("native_cgroup_filter_not_activated"));
}

#[test]
fn sched_switch_uses_consumed_cursor_instead_of_lookup_delete() {
    let source = include_str!("../../../../stutter-ebpf/src/wakeup_data.rs");

    assert!(source.contains("static WAKEUP_CONSUMED"));
    assert!(source.contains("consume_pending_wakeup"));
    assert!(source.contains("without deleting WAKEUP_DATA"));
}

#[test]
fn sched_switch_reads_previous_task_context_after_relevance_filters() {
    let source = include_str!("../../../../stutter-ebpf/src/main.rs");
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
    let source = include_str!("../../../../stutter-ebpf/src/main.rs");
    let start = source.find("fn irq_key").unwrap();
    let end = source[start..].find("fn increment_drop_counter").unwrap() + start;
    let body = &source[start..end];

    assert!(body.contains("High 32 bits: IRQ number. Low 32 bits: CPU ID."));
    assert!(body.contains("CPU IDs such as 65_536"));
    assert!(body.contains("((irq as u64) << 32) | cpu as u64"));
    assert!(!body.contains("cpu >= 65536"));
    assert!(!body.contains("cpu > 65535"));
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
    assert_eq!(sizing.wakeup_data_entries, 4_096);
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
