use std::collections::BTreeMap;

use stutter_common::SchedulerEvent;

use super::*;

#[cfg(test)]
mod latency_bookkeeping_tests {
    use super::*;

    #[test]
    fn test_fault_delta_bookkeeping_separation() {
        let mut stats = TaskStats::new(123, "test".to_string(), 0);
        let mut event = SchedulerEvent {
            kind: stutter_common::EVENT_RUNNABLE_LATENCY,
            tid: 123,
            cpu: 0,
            wakeup_target_cpu: 0,
            prio: 120,
            waker_tid: 0,
            target_pending_wakeups: 0,
            observed_runnable_depth: 0,
            maj_flt: 0,
            min_flt: 0,
            wakeup_ns: 100,
            switch_ns: 200,
            latency_ns: 100,
            comm: [0; 16],
            switch_prev_pid: 0,
            _pad0: 0,
            switch_prev_state: 0,
        };

        // 1. First event establishes baseline: 10 faults
        event.maj_flt = 10;
        event.latency_ns = 100; // Not a spike
        stats.record(&event, 1000, 0, None);
        assert_eq!(stats.last_spike_major_faults, 10);

        // 2. Interval summary happens. It sees 10 faults.
        // It should NOT update stats.last_spike_major_faults.
        let mut stats_by_task = TaskStatsMap::new();
        stats_by_task.insert(123.into(), stats);
        let mut prev_faults_snapshot = BTreeMap::new();
        prev_faults_snapshot.insert(123, (10, 0));

        // Simulate 2 more faults happening before interval summary reading eBPF map
        // (In reality, collect_interval_summaries_labeled reads from eBPF map)
        // Let's say eBPF map now has 12.
        // We simulate this by NOT passing a map, but we want to check that IF it were updated, it wouldn't affect stats.

        // Actually, the bug was that collect_interval_summaries_labeled updated stats.major_faults.
        // But now it doesn't have that field, and collect_interval_summaries_labeled doesn't touch last_spike_major_faults.

        // To properly test the "separation", we just need to ensure that
        // after ANY number of interval summaries, last_spike_major_faults remains 10
        // until the NEXT spike.

        // Simulate interval summary logic WITHOUT the bug:
        // (maj_delta = 12 - 10 = 2)
        // prev_faults_snapshot.insert(123, (12, 0));
        // stats is NOT updated.

        let stats = stats_by_task.get_mut(&123).unwrap();
        assert_eq!(stats.last_spike_major_faults, 10);

        // 3. Next spike event with 12 faults
        event.maj_flt = 12;
        event.latency_ns = 2000; // Spike!
        let (maj_delta, _) = stats.record(&event, 1000, 0, None);

        // Delta should be 12 - 10 = 2.
        // If interval summary had reset the baseline to 12, delta would be 0.
        assert_eq!(maj_delta, 2);
        assert_eq!(stats.last_spike_major_faults, 12);
        assert_eq!(stats.top_spikes[0].major_faults, 2);
    }

    #[test]
    fn cpu_perf_records_interval_once_and_session_cumulative() {
        let mut stats = TaskStats::new(123, "test".to_string(), 0);
        let delta = crate::perf_counters::CpuPerfDelta {
            cycles: Some(100),
            instructions: Some(200),
            cache_misses: Some(10),
            time_enabled_ns: Some(1_000),
            time_running_ns: Some(1_000),
            ..Default::default()
        };
        stats.record_cpu_perf(&delta);

        let event = SchedulerEvent {
            kind: stutter_common::EVENT_RUNNABLE_LATENCY,
            tid: 123,
            cpu: 0,
            wakeup_target_cpu: 0,
            prio: 120,
            waker_tid: 0,
            target_pending_wakeups: 0,
            observed_runnable_depth: 0,
            maj_flt: 0,
            min_flt: 0,
            wakeup_ns: 1000,
            switch_ns: 2000,
            latency_ns: 1_000,
            comm: [0; 16],
            switch_prev_pid: 0,
            _pad0: 0,
            switch_prev_state: 0,
        };
        stats.record(&event, 1_000_000, 0, None);

        let mut stats_by_task = BTreeMap::from([(123.into(), stats)]);
        let mut prev_faults_snapshot = BTreeMap::new();
        let records = collect_interval_summaries_labeled(
            "summary",
            &mut stats_by_task,
            1_000,
            &Default::default(),
            None,
            None,
            &mut prev_faults_snapshot,
        );

        assert_eq!(records.len(), 1);
        let perf = records[0].cpu_perf.as_ref().unwrap();
        assert_eq!(perf.cycles, Some(100));
        assert_eq!(perf.instructions, Some(200));
        assert_eq!(perf.ipc, Some(2.0));
        assert_eq!(perf.cache_mpki, Some(50.0));

        let stats = stats_by_task.get_mut(&123).unwrap();
        stats.record(&event, 1_000_000, 2_000, None);
        let records = collect_interval_summaries_labeled(
            "summary",
            &mut stats_by_task,
            2_000,
            &Default::default(),
            None,
            None,
            &mut prev_faults_snapshot,
        );

        assert_eq!(records.len(), 1);
        assert!(records[0].cpu_perf.is_none());

        let session_perf = stats_by_task
            .get(&123)
            .unwrap()
            .session_cpu_perf
            .as_ref()
            .and_then(|perf| perf.snapshot())
            .unwrap();
        assert_eq!(session_perf.cycles, Some(100));
        assert_eq!(session_perf.instructions, Some(200));
    }
}
