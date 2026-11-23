//! Offline replay of one sealed off/on window pair through the production objective comparison. A research
//! campaign's action study decides keep or revert with the very function the live controller uses
//! (`compare_for_objective` for `GameRunnableLatency`): the same thresholds, the same worsening-tail rejection, the
//! same frame, foreground, power and thermal guards, over the same canonical histogram layout. Nothing here defines
//! a second policy; the one addition is that a histogram too coarse to show the claimed improvement says so.

use super::{
    ObjectiveComparisonInput, ObjectiveKind, ObjectiveSignals, compare_for_objective,
    game_runnable_latency::RunnableThresholds,
};
use crate::{
    autotune::{
        comparison::{ExperimentDataQuality, ExperimentResult},
        experiment::WindowScore,
    },
    metrics::LatencyHistogramBucket,
    scorer::StutterScore,
};

/// What one window's sealed record supplies to the comparison.
#[derive(Clone, Debug)]
pub struct ReplayWindow {
    /// The target game's merged runnable-delay population in the canonical bucket layout.
    pub histogram: Vec<LatencyHistogramBucket>,
    pub max_ns: u64,
    pub target_pid: u32,
    pub target_start_ticks: u64,
    /// The foreground's own frame-time tail and over-5-ms count over the same window, and the frames behind them.
    pub frame_p99_ms: f64,
    pub scored_samples: u64,
    pub foreground_over_5ms: u64,
    pub thermal_degraded: bool,
    pub thermal_throttle_count: u64,
    pub cpu_power_limited: bool,
    pub gpu_power_limited: bool,
}

fn signals(window: &ReplayWindow, score: &WindowScore) -> ObjectiveSignals {
    let mut signals = ObjectiveSignals::from_window_score(score);
    signals.set_runnable_latency_from_histogram(&window.histogram, window.max_ns);
    signals.runnable_target_root_pid = Some(window.target_pid);
    signals.runnable_target_starttime_ticks = Some(window.target_start_ticks);
    signals.thermal_degraded = Some(window.thermal_degraded);
    signals.thermal_throttle_count = Some(window.thermal_throttle_count);
    signals.cpu_power_limited = Some(window.cpu_power_limited);
    signals.gpu_power_limited = Some(window.gpu_power_limited);
    signals
}

fn score(window: &ReplayWindow) -> WindowScore {
    WindowScore {
        started_unix_nanos: 1,
        finished_unix_nanos: 2,
        interval_count: 5,
        scored_samples: window.scored_samples,
        scored_task_count: 1,
        score: StutterScore {
            frame_p99_ms: window.frame_p99_ms,
            over_5ms: window.foreground_over_5ms,
            ..StutterScore::default()
        },
    }
}

/// The lower edge of the canonical bucket the window's 99th-percentile bound lies in.
fn bucket_floor_ns(signals: &ObjectiveSignals, window: &ReplayWindow) -> Option<u64> {
    let p99 = signals.runnable_latency_p99_ns?;
    let mut floor = 0;
    for bucket in &window.histogram {
        if bucket.upper_bound_ns.is_none_or(|upper| p99 <= upper) {
            return Some(floor);
        }
        floor = bucket.upper_bound_ns?;
    }
    None
}

/// Why one window is not a valid runnable population (the production signal builder's own reason), if it is not.
pub fn window_problem(window: &ReplayWindow) -> Option<String> {
    let mut signals = ObjectiveSignals::default();
    signals.set_runnable_latency_from_histogram(&window.histogram, window.max_ns);
    signals.runnable_latency_unavailable_reason
}

/// The production comparison of the candidate (action on) against the baseline (action off). An improvement is
/// reported only when it survives the histogram's resolution: the candidate's bound must already clear the
/// frozen improvement from the *bottom* of the baseline's bucket, else the measurement cannot support the claim.
pub fn replay_pair(baseline: &ReplayWindow, candidate: &ReplayWindow) -> ExperimentResult {
    let (baseline_score, candidate_score) = (score(baseline), score(candidate));
    let (baseline_signals, candidate_signals) = (
        signals(baseline, &baseline_score),
        signals(candidate, &candidate_score),
    );
    let result = compare_for_objective(ObjectiveComparisonInput {
        objective: ObjectiveKind::GameRunnableLatency,
        baseline: &baseline_score,
        candidate: &candidate_score,
        baseline_signals: &baseline_signals,
        candidate_signals: &candidate_signals,
        data_quality: ExperimentDataQuality::High,
        target_disappeared: false,
    });
    if let ExperimentResult::Improved { .. } = result {
        let required = 1.0 - RunnableThresholds::default().min_improvement_percent / 100.0;
        let certain = bucket_floor_ns(&baseline_signals, baseline)
            .zip(candidate_signals.runnable_latency_p99_ns)
            .is_some_and(|(floor, upper)| (upper as f64) <= floor as f64 * required);
        if !certain {
            return ExperimentResult::Inconclusive {
                reason: "the runnable histogram's resolution cannot show the claimed improvement"
                    .to_owned(),
            };
        }
    }
    result
}

#[cfg(test)]
#[path = "runnable_replay_tests.rs"]
mod tests;
