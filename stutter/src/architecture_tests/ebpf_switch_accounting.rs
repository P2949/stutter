use std::fs;

#[test]
fn post_consumption_read_failures_use_correct_drop_counter() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let scheduler_rs = workspace_root.join("stutter-ebpf/src/scheduler.rs");

    let content = fs::read_to_string(&scheduler_rs)
        .expect("failed to read scheduler.rs");

    let try_sched_switch_start = content
        .find("pub(crate) fn try_sched_switch")
        .expect("try_sched_switch not found");
    let try_sched_migrate_task_start = content
        .find("pub(crate) fn try_sched_migrate_task")
        .expect("try_sched_migrate_task not found");

    let try_sched_switch_body = &content[try_sched_switch_start..try_sched_migrate_task_start];

    let consume_start = try_sched_switch_body
        .find("consume_pending_wakeup")
        .expect("consume_pending_wakeup not found");

    let post_consume_body = &try_sched_switch_body[consume_start..];

    let mut inside_read_check = false;
    let mut has_increment = false;
    let mut is_return_1 = false;
    let lines: Vec<&str> = post_consume_body.lines().collect();

    let mut read_failure_blocks_found = 0;

    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with("if !read_") {
            inside_read_check = true;
            has_increment = false;
            is_return_1 = false;
        } else if inside_read_check {
            if trimmed.contains("increment_drop_counter") {
                assert!(
                    trimmed.contains("DROP_WAKEUP_DATA_CONSUMED_READ_FAILED"),
                    "Read failure after wakeup consumption must increment DROP_WAKEUP_DATA_CONSUMED_READ_FAILED"
                );
                has_increment = true;
            } else if trimmed.starts_with("return 1;") {
                is_return_1 = true;
            } else if trimmed == "}" {
                if is_return_1 {
                    assert!(
                        has_increment,
                        "Read failure returning 1 after wakeup consumption must increment a drop counter"
                    );
                    read_failure_blocks_found += 1;
                }
                inside_read_check = false;
            }
        }
    }

    assert!(
        read_failure_blocks_found >= 4,
        "Expected at least 4 read failure checks in try_sched_switch post consumption"
    );
}
