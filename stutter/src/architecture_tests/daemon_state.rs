//! Architecture checks for daemon-state persistence semantics.

#[test]
fn daemon_state_store_lifecycle_mutations_do_not_clone_full_state() {
    let source = include_str!("../daemon/store.rs");

    assert!(
        source.contains("fn mutate_current"),
        "DaemonStateStore should use a single in-place mutation boundary"
    );
    assert!(
        !source.contains("self.state.clone()"),
        "DaemonStateStore lifecycle mutations must not clone the full DaemonState"
    );
}
