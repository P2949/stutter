pub fn classify_switch_prev_state(state: i64) -> &'static str {
    if state == 0 {
        return "running";
    }

    if state < 0 {
        return "preempted";
    }

    let unsigned = state as u64;

    if unsigned & 0x0001 != 0 {
        "interruptible_sleep"
    } else if unsigned & 0x0002 != 0 {
        "uninterruptible_sleep"
    } else if unsigned & 0x0004 != 0 {
        "stopped"
    } else if unsigned & 0x0008 != 0 {
        "traced"
    } else if unsigned & 0x0010 != 0 {
        "exit_dead"
    } else if unsigned & 0x0020 != 0 {
        "exit_zombie"
    } else if unsigned & 0x0040 != 0 {
        "parked"
    } else if unsigned & 0x0080 != 0 {
        "dead"
    } else if unsigned & 0x0100 != 0 {
        "wakekill"
    } else if unsigned & 0x0200 != 0 {
        "waking"
    } else if unsigned & 0x0400 != 0 {
        "no_load"
    } else if unsigned & 0x0800 != 0 {
        "new"
    } else if unsigned & 0x1000 != 0 {
        "rtlock_wait"
    } else {
        "unknown"
    }
}

#[cfg(test)]
mod tests {
    use super::classify_switch_prev_state;

    #[test]
    fn classifies_running_state() {
        assert_eq!(classify_switch_prev_state(0), "running");
    }

    #[test]
    fn classifies_common_sleep_states() {
        assert_eq!(classify_switch_prev_state(1), "interruptible_sleep");
        assert_eq!(classify_switch_prev_state(2), "uninterruptible_sleep");
    }

    #[test]
    fn classifies_preempted_state() {
        assert_eq!(classify_switch_prev_state(-1), "preempted");
    }

    #[test]
    fn classifies_unknown_positive_state() {
        assert_eq!(classify_switch_prev_state(0x4000), "unknown");
    }
}
