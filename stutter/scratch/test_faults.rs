#[cfg(test)]
mod tests {
    use super::*;
    use stutter_common::SchedulerEvent;
    use std::collections::BTreeMap;

    #[test]
    fn test_fault_delta_mixing() {
        let mut stats = TaskStats::new(123, "test".to_string(), 0);
        let mut event = SchedulerEvent::default();
        event.pid = 123;
        
        // 1. Initial spike event with 10 faults
        event.maj_flt = 10;
        event.latency_ns = 10_000_000; // Above threshold
        stats.record(&event, 1_000_000, 0);
        
        assert_eq!(stats.major_faults, 10);
        assert_eq!(stats.top_spikes[0].major_faults, 10); // First delta is 10-0 = 10

        // 2. Interval summary reads 10 (no change yet)
        let mut stats_map = BTreeMap::new();
        stats_map.insert(123, stats.clone());
        let mut prev_faults_snapshot = BTreeMap::new();
        // (Simplified collect_interval_summaries_labeled logic for testing)
        // In the real code, it reads from eBPF map. Let's simulate that.
        let current_maj = 10;
        let prev = prev_faults_snapshot.get(&123).copied().unwrap_or((current_maj, 0));
        let maj_delta = current_maj - prev.0;
        // In the real code, it updates stats.major_faults = current_maj;
        // And inserts into prev_faults_snapshot.
        
        // 3. 2 more faults happen. Total = 12.
        // Interval summary happens BEFORE next spike.
        let current_maj = 12;
        // collect_interval_summaries_labeled would do:
        let stats_in_map = stats_map.get_mut(&123).unwrap();
        stats_in_map.major_faults = current_maj; 
        prev_faults_snapshot.insert(123, (current_maj, 0));

        // 4. Next spike event with 12 faults.
        event.maj_flt = 12;
        event.latency_ns = 10_000_000;
        stats_in_map.record(&event, 1_000_000, 0);

        // EXPECTED: spike delta should be 12 - 10 = 2.
        // ACTUAL (current code): delta = 12 - 12 = 0.
        let last_spike = &stats_in_map.top_spikes[0]; // top_spikes is sorted, so index 0 might be the first one if latency is same.
        // Let's check all spikes or use different latencies.
        
        println!("Spike deltas: {:?}", stats_in_map.top_spikes.iter().map(|s| s.major_faults).collect::<Vec<_>>());
        
        // If it's 0, then the bug is reproduced.
    }
}
