use super::*;

#[test]
fn tracepoint_preflight_required_mismatch_includes_diagnostic_dump_hint() {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    let dir = temp_dir("tracepoint-diagnostic-hint");

    let sched_wakeup = dir.join("sched/sched_wakeup");
    fs::create_dir_all(&sched_wakeup).unwrap();
    fs::write(
        sched_wakeup.join("format"),
        "field:pid_t pid; offset:99; size:4; signed:1;\n\
         field:int prio; offset:28; size:4; signed:1;\n\
         field:int target_cpu; offset:32; size:4; signed:1;\n",
    )
    .unwrap();

    let sched_switch = dir.join("sched/sched_switch");
    fs::create_dir_all(&sched_switch).unwrap();
    fs::write(
        sched_switch.join("format"),
        "field:pid_t prev_pid; offset:24; size:4; signed:1;\n\
         field:long prev_state; offset:32; size:8; signed:1;\n\
         field:char next_comm[16]; offset:40; size:16; signed:1;\n\
         field:pid_t next_pid; offset:56; size:4; signed:1;\n\
         field:int next_prio; offset:60; size:4; signed:1;\n",
    )
    .unwrap();

    let report =
        crate::ebpf::preflight::tracepoint_preflight(&dir, false, false, false, false, false);

    assert_eq!(report.sched_wakeup, "mismatch");
    assert!(
        report
            .errors
            .iter()
            .any(|error| error.contains("doctor tracepoints --dump --json")),
        "preflight errors should tell users how to collect bug-report diagnostics: {report:#?}"
    );

    fs::remove_dir_all(dir).ok();
}

#[test]
fn tracepoint_preflight_optional_mismatch_includes_diagnostic_dump_hint() {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    let dir = temp_dir("tracepoint-optional-diagnostic-hint");

    let sched_wakeup = dir.join("sched/sched_wakeup");
    fs::create_dir_all(&sched_wakeup).unwrap();
    fs::write(
        sched_wakeup.join("format"),
        "field:pid_t pid; offset:24; size:4; signed:1;\n\
         field:int prio; offset:28; size:4; signed:1;\n\
         field:int target_cpu; offset:32; size:4; signed:1;\n",
    )
    .unwrap();

    let sched_switch = dir.join("sched/sched_switch");
    fs::create_dir_all(&sched_switch).unwrap();
    fs::write(
        sched_switch.join("format"),
        "field:pid_t prev_pid; offset:24; size:4; signed:1;\n\
         field:long prev_state; offset:32; size:8; signed:1;\n\
         field:char next_comm[16]; offset:40; size:16; signed:1;\n\
         field:pid_t next_pid; offset:56; size:4; signed:1;\n\
         field:int next_prio; offset:60; size:4; signed:1;\n",
    )
    .unwrap();

    let sched_wakeup_new = dir.join("sched/sched_wakeup_new");
    fs::create_dir_all(&sched_wakeup_new).unwrap();
    fs::write(
        sched_wakeup_new.join("format"),
        "field:pid_t pid; offset:99; size:4; signed:1;\n\
         field:int prio; offset:28; size:4; signed:1;\n\
         field:int target_cpu; offset:32; size:4; signed:1;\n",
    )
    .unwrap();

    let report =
        crate::ebpf::preflight::tracepoint_preflight(&dir, false, false, false, false, false);

    assert_eq!(report.sched_wakeup_new, "mismatch");
    assert!(
        report.warnings.iter().any(|warning| {
            warning.contains("sched_wakeup_new")
                && warning.contains("doctor tracepoints --dump --json")
        }),
        "preflight optional mismatch warnings should tell users how to collect bug-report diagnostics: {report:#?}"
    );

    fs::remove_dir_all(dir).ok();
}

#[test]
fn optional_tracepoint_format_missing_is_not_an_error() {
    let dir = temp_dir("optional-tracepoint");
    fs::create_dir_all(&dir).unwrap();

    let available = validate_optional_tracepoint_format_at(
        &dir.join("missing/format"),
        TracepointName::new("missing"),
        &[TracepointFieldSpec::new("pid", 24)],
        true,
    )
    .unwrap();

    assert!(!available);
    fs::remove_dir_all(dir).ok();
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
