use super::*;

#[test]
fn classify_switch_prev_state_zero_is_running() {
    assert_eq!(classify_switch_prev_state(0), "running");
}

#[test]
fn classify_switch_prev_state_interruptible() {
    assert_eq!(classify_switch_prev_state(1), "interruptible_sleep");
}

#[test]
fn classify_switch_prev_state_uninterruptible() {
    assert_eq!(classify_switch_prev_state(2), "uninterruptible_sleep");
}

#[test]
fn classify_switch_prev_state_other_sleep() {
    assert_eq!(classify_switch_prev_state(8), "traced");
}

#[test]
fn classify_switch_prev_state_interruptible_wins_when_multiple_bits_set() {
    assert_eq!(classify_switch_prev_state(3), "interruptible_sleep");
}
