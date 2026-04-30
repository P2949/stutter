use serde::{Deserialize, Serialize};

use crate::{metrics::IntervalRecord, process_tree::TaskClass};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct StutterScore {
    pub total: u64,
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
    pub max_latency_ns: u64,
}

pub fn score_from_interval_records(records: &[IntervalRecord]) -> StutterScore {
    let mut score = StutterScore::default();

    for record in records.iter().filter(|record| {
        matches!(
            record.class,
            TaskClass::Game | TaskClass::WineServer | TaskClass::GameScope | TaskClass::Compositor
        )
    }) {
        score.over_1ms = score.over_1ms.saturating_add(record.over_1ms);
        score.over_2ms = score.over_2ms.saturating_add(record.over_2ms);
        score.over_5ms = score.over_5ms.saturating_add(record.over_5ms);
        score.max_latency_ns = score.max_latency_ns.max(record.max_ns);
    }

    score.total = score
        .over_5ms
        .saturating_mul(100)
        .saturating_add(score.over_2ms.saturating_mul(20))
        .saturating_add(score.over_1ms);
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_threshold_counters() {
        let record = IntervalRecord {
            elapsed_ms: 0,
            task: 1,
            active: true,
            class: TaskClass::Game,
            comm: "game".into(),
            process_pid: Some(1), // This is a u32, not Arc<str>
            process_comm: "game".into(),
            samples: 1,
            stored_samples: 1,
            truncated_samples: 0,
            min_ns: 0,
            avg_ns: 0,
            p95_ns: 0,
            p99_ns: 0,
            max_ns: 7_000_000,
            over_1ms: 3,
            over_2ms: 2,
            over_5ms: 1,
            busiest_cpu: None,
            busiest_cpu_samples: 0,
            worst_cpu: None,
            worst_cpu_max_ns: 0,
            spikiest_cpu: None,
            spikiest_cpu_spikes: 0,
            major_faults: 0,
            minor_faults: 0,
            percentile_scope: "exact".to_owned(),
            histogram: Vec::new(),
            drop_counters: Default::default(),
        };

        let score = score_from_interval_records(&[record]);
        assert_eq!(score.total, 143);
        assert_eq!(score.max_latency_ns, 7_000_000);
    }
}
