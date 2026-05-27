use std::collections::BTreeSet;

use super::{RollingWindow, RollingWindowScore};
use crate::autotune::quality::{OnlineDataQualityInput, OnlineDataQualityPolicy};

pub(crate) fn compute_rolling_window_score(
    window: &RollingWindow,
    quality_policy: &OnlineDataQualityPolicy,
) -> RollingWindowScore {
    let interval_count = window.interval_count();
    let scored_samples = window.scored_samples();
    let over_1ms = window
        .intervals()
        .iter()
        .map(|record| record.over_1ms)
        .fold(0_u64, u64::saturating_add);
    let over_2ms = window
        .intervals()
        .iter()
        .map(|record| record.over_2ms)
        .fold(0_u64, u64::saturating_add);
    let over_5ms = window
        .intervals()
        .iter()
        .map(|record| record.over_5ms)
        .fold(0_u64, u64::saturating_add);
    let max_latency_ns = window
        .intervals()
        .iter()
        .map(|record| record.max_ns)
        .max()
        .unwrap_or(0);
    let diagnostic_score_total = over_5ms
        .saturating_mul(100)
        .saturating_add(over_2ms.saturating_mul(20))
        .saturating_add(over_1ms);
    let frame_count = window.frame_count();
    let frame_p99_ms = window.frame_p99_ms();
    let frame_max_ms = window.frame_max_ms();

    let scored_task_count = window
        .intervals()
        .iter()
        .filter(|record| record.samples > 0)
        .map(|record| record.task)
        .collect::<BTreeSet<_>>()
        .len();

    let drop_counter_total = window
        .intervals()
        .iter()
        .map(|record| record.drop_counters.total())
        .fold(0_u64, u64::saturating_add);

    let data_quality = OnlineDataQualityInput {
        scored_intervals: interval_count,
        scored_samples,
        scored_task_count,
        drop_counter_total,
        target_identity_shifted: false,
        target_present: scored_task_count > 0,
        frame_data_required: false,
        frame_count,
        baseline_frame_count: None,
        candidate_frame_count: None,
        baseline_scored_identity_counts: &[],
        candidate_scored_identity_counts: &[],
    }
    .evaluate_with_policy(quality_policy);

    RollingWindowScore {
        duration_ms: window.duration_ms(),
        interval_count,
        scored_task_count,
        scored_samples,
        diagnostic_score_total,
        over_1ms,
        over_2ms,
        over_5ms,
        max_latency_ns,
        frame_count,
        frame_p99_ms,
        frame_max_ms,
        dropped_invalid_frames: window.dropped_invalid_frame_count(),
        data_quality,
    }
}
