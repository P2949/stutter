use serde::{Deserialize, Serialize};

use crate::{metrics::IntervalRecord, process_tree::TaskClass};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct StutterScore {
    pub total: u64,
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
    pub max_latency_ns: u64,
    pub frame_max_ms: f64,
    pub frame_p99_ms: f64,
}

pub fn class_contributes_to_score(class: TaskClass) -> bool {
    matches!(
        class,
        TaskClass::Game | TaskClass::GameHelper | TaskClass::WineServer | TaskClass::GameScope
    )
}

pub fn score_from_interval_records(records: &[IntervalRecord]) -> StutterScore {
    let mut score = StutterScore::default();

    for record in records
        .iter()
        .filter(|record| class_contributes_to_score(record.class))
    {
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

pub fn score_from_interval_records_and_frames(
    records: &[IntervalRecord],
    frames: &[crate::recorder::FrameEvent],
) -> StutterScore {
    let mut score = score_from_interval_records(records);
    let (frame_max, frame_p99) = calculate_frame_metrics(frames);
    score.frame_max_ms = frame_max;
    score.frame_p99_ms = frame_p99;

    if frame_max > 50.0 {
        score.total = score.total.saturating_add(100);
    } else if frame_max > 20.0 {
        score.total = score.total.saturating_add(20);
    }

    score
}

pub fn calculate_frame_metrics(frames: &[crate::recorder::FrameEvent]) -> (f64, f64) {
    let mut times: Vec<f64> = frames
        .iter()
        .map(|f| f.frametime_ms)
        .filter(|value| value.is_finite())
        .collect();
    if times.is_empty() {
        return (0.0, 0.0);
    }
    times.sort_by(|a, b| a.total_cmp(b));
    let max = *times.last().unwrap_or(&0.0);
    let p99_idx = (times.len() * 99 / 100).min(times.len() - 1);
    let p99 = times[p99_idx];
    (max, p99)
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
            cpu_psi_some: 0.0,
            mem_psi_some: 0.0,
            mem_psi_full: 0.0,
            io_psi_some: 0.0,
            io_psi_full: 0.0,
            major_faults: 0,
            minor_faults: 0,
            percentile_scope: "all".to_owned(),
            histogram: Vec::new(),
            drop_counters: crate::ebpf_loader::DropCountersSnapshot::default(),
        };

        let score = score_from_interval_records(&[record]);
        assert_eq!(score.total, 143);
        assert_eq!(score.max_latency_ns, 7_000_000);
    }

    #[test]
    fn frame_metrics_contribute_to_total_score() {
        let frames = [
            crate::recorder::FrameEvent {
                elapsed_ms: 1_000,
                frametime_ms: 16.6,
            },
            crate::recorder::FrameEvent {
                elapsed_ms: 1_016,
                frametime_ms: 55.0,
            },
        ];

        let score = score_from_interval_records_and_frames(&[], &frames);

        assert_eq!(score.total, 100);
        assert_eq!(score.frame_max_ms, 55.0);
        assert_eq!(score.frame_p99_ms, 55.0);
    }

    #[test]
    fn frame_metrics_ignore_non_finite_values() {
        let frames = [
            crate::recorder::FrameEvent {
                elapsed_ms: 1_000,
                frametime_ms: f64::NAN,
            },
            crate::recorder::FrameEvent {
                elapsed_ms: 1_016,
                frametime_ms: 24.0,
            },
            crate::recorder::FrameEvent {
                elapsed_ms: 1_032,
                frametime_ms: f64::INFINITY,
            },
        ];

        assert_eq!(calculate_frame_metrics(&frames), (24.0, 24.0));
    }

    #[test]
    fn score_includes_game_helpers() {
        let mut record = IntervalRecord {
            elapsed_ms: 0,
            task: 1,
            active: true,
            class: TaskClass::GameHelper,
            comm: "dxvk-worker".into(),
            process_pid: Some(1),
            process_comm: "game".into(),
            samples: 1,
            stored_samples: 1,
            truncated_samples: 0,
            min_ns: 0,
            avg_ns: 0,
            p95_ns: 0,
            p99_ns: 0,
            max_ns: 2_000_000,
            over_1ms: 1,
            over_2ms: 1,
            over_5ms: 0,
            busiest_cpu: None,
            busiest_cpu_samples: 0,
            worst_cpu: None,
            worst_cpu_max_ns: 0,
            spikiest_cpu: None,
            spikiest_cpu_spikes: 0,
            cpu_psi_some: 0.0,
            mem_psi_some: 0.0,
            mem_psi_full: 0.0,
            io_psi_some: 0.0,
            io_psi_full: 0.0,
            major_faults: 0,
            minor_faults: 0,
            percentile_scope: "all".to_owned(),
            histogram: Vec::new(),
            drop_counters: crate::ebpf_loader::DropCountersSnapshot::default(),
        };

        assert_eq!(score_from_interval_records(&[record.clone()]).total, 21);

        record.class = TaskClass::Compositor;
        assert_eq!(score_from_interval_records(&[record]).total, 0);
    }
}
