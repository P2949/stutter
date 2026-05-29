use super::*;

#[test]
fn validates_irq_tracepoint_offsets() {
    let format = r#"
field:unsigned short common_type; offset:0; size:2; signed:0;
field:int irq; offset:8; size:4; signed:1;
"#;

    validate_tracepoint_format(format, &[TracepointFieldSpec::new("irq", 8)]).unwrap();
}

#[test]
fn rejects_bad_irq_tracepoint_offsets() {
    let format = "field:int irq; offset:12; size:4; signed:1;";

    let err =
        validate_tracepoint_format(format, &[TracepointFieldSpec::new("irq", 8)]).unwrap_err();
    assert!(err.to_string().contains("expected offset 8, got 12"));
}

#[test]
fn validates_irq_tracepoint_without_ret() {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    let dir = temp_dir("irq-without-ret");
    fs::create_dir_all(&dir).unwrap();

    let irq_entry_dir = dir.join("irq/irq_handler_entry");
    fs::create_dir_all(&irq_entry_dir).unwrap();
    fs::write(
        irq_entry_dir.join("format"),
        "field:int irq; offset:8; size:4; signed:1;",
    )
    .unwrap();

    let irq_exit_dir = dir.join("irq/irq_handler_exit");
    fs::create_dir_all(&irq_exit_dir).unwrap();
    fs::write(
        irq_exit_dir.join("format"),
        "field:int irq; offset:8; size:4; signed:1;",
    )
    .unwrap();

    // Add required sched/sched_wakeup and sched/sched_switch
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

    let mut config =
        match crate::cli::parse_app_command_from(["stutter", "monitor", "--pid", "42"]).unwrap() {
            crate::commands::AppCommand::Monitor(c) => (*c.config).clone(),
            _ => unreachable!(),
        };

    config.probes.irq_latency = true;

    let availability = validate_tracepoint_formats(&dir, &config).unwrap();
    assert!(availability.irq_handler);

    let preflight =
        crate::ebpf::preflight::tracepoint_preflight(&dir, false, false, true, false, false);
    assert_eq!(preflight.irq_handler, "ok");

    fs::remove_dir_all(dir).ok();
}

#[test]
fn irq_preflight_disables_irq_latency_when_exit_tracepoint_is_missing() {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    let dir = temp_dir("irq-missing-exit");
    fs::create_dir_all(&dir).unwrap();

    let irq_entry_dir = dir.join("irq/irq_handler_entry");
    fs::create_dir_all(&irq_entry_dir).unwrap();
    fs::write(
        irq_entry_dir.join("format"),
        "field:int irq; offset:8; size:4; signed:1;",
    )
    .unwrap();

    // Do NOT create irq/irq_handler_exit/format

    // Add required sched/sched_wakeup and sched/sched_switch
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

    let mut config =
        match crate::cli::parse_app_command_from(["stutter", "monitor", "--pid", "42"]).unwrap() {
            crate::commands::AppCommand::Monitor(c) => (*c.config).clone(),
            _ => unreachable!(),
        };

    config.probes.irq_latency = true;

    let availability = validate_tracepoint_formats(&dir, &config).unwrap();
    assert!(!availability.irq_handler);

    let preflight =
        crate::ebpf::preflight::tracepoint_preflight(&dir, false, false, true, false, false);
    assert_eq!(preflight.irq_handler, "missing");

    fs::remove_dir_all(dir).ok();
}

#[test]
fn irq_preflight_rejects_bad_irq_handler_exit_offset() {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    let dir = temp_dir("irq-bad-exit-offset");
    fs::create_dir_all(&dir).unwrap();

    let irq_entry_dir = dir.join("irq/irq_handler_entry");
    fs::create_dir_all(&irq_entry_dir).unwrap();
    fs::write(
        irq_entry_dir.join("format"),
        "field:int irq; offset:8; size:4; signed:1;",
    )
    .unwrap();

    let irq_exit_dir = dir.join("irq/irq_handler_exit");
    fs::create_dir_all(&irq_exit_dir).unwrap();
    fs::write(
        irq_exit_dir.join("format"),
        "field:int irq; offset:12; size:4; signed:1;",
    )
    .unwrap();

    // Add required sched/sched_wakeup and sched/sched_switch
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

    let mut config =
        match crate::cli::parse_app_command_from(["stutter", "monitor", "--pid", "42"]).unwrap() {
            crate::commands::AppCommand::Monitor(c) => (*c.config).clone(),
            _ => unreachable!(),
        };

    config.probes.irq_latency = true;

    let result = validate_tracepoint_formats(&dir, &config);
    assert!(result.is_err());
    let err = result.unwrap_err();
    let err_str = format!("{err:#}");
    assert!(err_str.contains("irq_handler_exit"));
    assert!(err_str.contains("expected offset 8, got 12"));

    let preflight =
        crate::ebpf::preflight::tracepoint_preflight(&dir, false, false, true, false, false);
    assert_eq!(preflight.irq_handler, "mismatch");

    fs::remove_dir_all(dir).ok();
}
