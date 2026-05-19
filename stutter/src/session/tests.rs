use super::*;

#[test]
fn tree_tick_not_needed_for_direct_pid_only() {
    assert!(!needs_tree_tick_from_parts(false, false, false));
}

#[test]
fn tree_tick_needed_for_tree_roots() {
    assert!(needs_tree_tick_from_parts(true, false, false));
}

#[test]
fn tree_tick_needed_for_watch_process_even_without_current_root() {
    assert!(needs_tree_tick_from_parts(false, true, false));
}

#[test]
fn tree_tick_needed_for_cgroupv2() {
    assert!(needs_tree_tick_from_parts(false, false, true));
}
