use super::*;

#[test]
fn sway_provider_detection_uses_swaysock_environment() {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    let previous = std::env::var_os("SWAYSOCK");

    // SAFETY: TEST_MUTEX serializes process environment mutation in this test.
    unsafe {
        std::env::remove_var("SWAYSOCK");
    }
    assert!(!SwayForegroundProvider::is_detected());

    // SAFETY: TEST_MUTEX serializes process environment mutation in this test.
    unsafe {
        std::env::set_var("SWAYSOCK", "/tmp/sway-ipc.sock");
    }
    assert!(SwayForegroundProvider::is_detected());

    // SAFETY: TEST_MUTEX is still held and previous was captured before mutation.
    unsafe {
        if let Some(previous) = previous {
            std::env::set_var("SWAYSOCK", previous);
        } else {
            std::env::remove_var("SWAYSOCK");
        }
    }
}
