//! Focus snapshot tests extracted from `focus::mod`.
//!
//! Owns tests for this focus behavior area after extraction from `focus::mod`.
//! Does not own shared fixtures or production focus behavior.

#[cfg(test)]
mod tests {
    use crate::focus::*;

    #[test]
    fn focus_counter_deltas_are_zero_on_first_seen_and_reset_on_pid_reuse() {
        let current = FocusCounters {
            starttime_ticks: Some(10),
            cpu_time_ticks: 100,
            read_bytes: 200,
            write_bytes: 300,
            voluntary_ctxt_switches: 40,
            nonvoluntary_ctxt_switches: 50,
        };

        let first_seen = counter_deltas(None, &current);
        assert_eq!(first_seen.starttime_ticks, Some(10));
        assert_eq!(first_seen.cpu_time_ticks, 0);
        assert_eq!(first_seen.read_bytes, 0);
        assert_eq!(first_seen.write_bytes, 0);
        assert_eq!(first_seen.voluntary_ctxt_switches, 0);
        assert_eq!(first_seen.nonvoluntary_ctxt_switches, 0);

        let previous = FocusCounters {
            starttime_ticks: Some(10),
            cpu_time_ticks: 70,
            read_bytes: 125,
            write_bytes: 250,
            voluntary_ctxt_switches: 35,
            nonvoluntary_ctxt_switches: 45,
        };

        let deltas = counter_deltas(Some(&previous), &current);
        assert_eq!(deltas.starttime_ticks, Some(10));
        assert_eq!(deltas.cpu_time_ticks, 30);
        assert_eq!(deltas.read_bytes, 75);
        assert_eq!(deltas.write_bytes, 50);
        assert_eq!(deltas.voluntary_ctxt_switches, 5);
        assert_eq!(deltas.nonvoluntary_ctxt_switches, 5);

        let reused_pid_previous = FocusCounters {
            starttime_ticks: Some(9),
            cpu_time_ticks: 500,
            read_bytes: 600,
            write_bytes: 700,
            voluntary_ctxt_switches: 80,
            nonvoluntary_ctxt_switches: 90,
        };

        let reused_pid_deltas = counter_deltas(Some(&reused_pid_previous), &current);
        assert_eq!(reused_pid_deltas.starttime_ticks, Some(10));
        assert_eq!(reused_pid_deltas.cpu_time_ticks, 0);
        assert_eq!(reused_pid_deltas.read_bytes, 0);
        assert_eq!(reused_pid_deltas.write_bytes, 0);
        assert_eq!(reused_pid_deltas.voluntary_ctxt_switches, 0);
        assert_eq!(reused_pid_deltas.nonvoluntary_ctxt_switches, 0);
    }
}
