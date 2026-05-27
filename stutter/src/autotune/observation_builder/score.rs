use crate::{
    autotune::rolling_window::RollingWindowScore as RuntimeWindowScore, scorer::StutterScore,
};

pub(crate) fn stutter_score_from_runtime_window_score(score: &RuntimeWindowScore) -> StutterScore {
    StutterScore {
        total: score.diagnostic_score_total,
        over_1ms: score.over_1ms,
        over_2ms: score.over_2ms,
        over_5ms: score.over_5ms,
        max_latency_ns: score.max_latency_ns,
        frame_max_ms: score.frame_max_ms,
        frame_p99_ms: score.frame_p99_ms,
        frame_over_16ms: 0,
        frame_over_33ms: 0,
        frame_over_50ms: 0,
    }
}
