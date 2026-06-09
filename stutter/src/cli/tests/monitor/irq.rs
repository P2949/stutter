use crate::cli::parse_app_command_from;

#[test]
fn monitor_irq_latency_without_irq_is_rejected_by_runtime_validation() {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();

    let err = parse_app_command_from(["stutter", "monitor", "--irq-latency"])
        .expect_err("--irq-latency without --irq should be rejected");

    let message = err.to_string();

    assert!(
        message.contains("--irq-latency requires at least one explicit --irq"),
        "unexpected validation error: {message}"
    );
    assert!(
        message.contains("/proc/interrupts"),
        "validation error should point users at /proc/interrupts: {message}"
    );
}
