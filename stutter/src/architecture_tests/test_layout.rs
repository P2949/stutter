//! Architecture checks for test-module placement in files touched by this split.

#[test]
fn split_state_and_diagnosis_keep_tests_in_child_modules() {
    for (path, source) in [
        ("src/daemon/state.rs", include_str!("../daemon/state.rs")),
        ("src/diagnosis.rs", include_str!("../diagnosis.rs")),
    ] {
        assert!(
            source.contains("#[cfg(test)]\nmod tests;"),
            "{path} should keep tests in an extracted child tests.rs module"
        );
        assert!(
            !source.contains("#[cfg(test)]\nmod tests {"),
            "{path} must not reintroduce a large inline tests block"
        );
    }
}
