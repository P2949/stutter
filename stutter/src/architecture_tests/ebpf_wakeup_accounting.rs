use std::fs;

#[test]
fn try_sched_wakeup_preserves_wakeup_data_and_gates_accounting() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let scheduler_rs = workspace_root.join("stutter-ebpf/src/scheduler.rs");

    let content = fs::read_to_string(&scheduler_rs).expect("failed to read scheduler.rs");

    let try_sched_wakeup_start = content
        .find("pub(crate) fn try_sched_wakeup")
        .expect("try_sched_wakeup not found");
    let try_sched_switch_start = content
        .find("pub(crate) fn try_sched_switch")
        .expect("try_sched_switch not found");

    let try_sched_wakeup_body = &content[try_sched_wakeup_start..try_sched_switch_start];

    // 1. Check that mark_task_runnable returns target_cpu_tracked
    assert!(
        try_sched_wakeup_body
            .contains("let target_cpu_tracked = mark_task_runnable(pid, target_cpu);"),
        "try_sched_wakeup must track if mark_task_runnable succeeded"
    );

    // 2. Check that wakeup_data::record_wakeup is still called unconditionally (not early returned if !target_cpu_tracked)
    assert!(
        try_sched_wakeup_body.contains("match wakeup_data::record_wakeup(pid, data, &mut old) {"),
        "try_sched_wakeup must call record_wakeup unconditionally to preserve wakeup data"
    );

    // 3. Check that increment_target_pending is gated by target_cpu_tracked
    let lines: Vec<&str> = try_sched_wakeup_body.lines().collect();
    let mut increment_count = 0;
    let mut gated_count = 0;

    for (i, line) in lines.iter().enumerate() {
        if line.contains("increment_target_pending") {
            increment_count += 1;
            // Check if previous line or two lines up contained the if guard
            let prev = if i > 0 { lines[i - 1] } else { "" };
            if prev.contains("if target_cpu_tracked {") {
                gated_count += 1;
            }
        }
    }

    assert_eq!(
        increment_count, gated_count,
        "All calls to increment_target_pending inside try_sched_wakeup must be exactly one line below `if target_cpu_tracked {{`"
    );

    assert!(
        increment_count >= 2,
        "Expected at least 2 increments of target_pending in try_sched_wakeup"
    );
}
