//! Generic tests for session module boundaries.
//!
//! Owns session architecture regression tests. Does not own production monitor session
//! orchestration, tick handling, or event conversion logic.

#[test]
fn session_child_modules_are_not_public_submodules() {
    let source = include_str!("../session.rs");

    let public_child_modules: Vec<&str> = source
        .lines()
        .map(str::trim_start)
        .filter(|line| line.starts_with("pub mod "))
        .collect();

    assert!(
        public_child_modules.is_empty(),
        "session child modules must stay crate-private and be exposed intentionally through api::session: {public_child_modules:?}"
    );
}
