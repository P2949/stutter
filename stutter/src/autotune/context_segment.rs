#![allow(dead_code)]
use std::collections::BTreeMap;

use crate::{
    autotune::rolling_window::RollingWindow,
    tune::comparability::{
        ScoredIdentityCount, TaskIdentity, scored_identity_counts_to_map,
        scored_identity_map_to_counts, scored_identity_overlap,
    },
};

#[derive(Debug, Clone, Default)]
pub struct ContextSignals {
    pub gpu_busy_min_percent: Option<f64>,
    pub gpu_busy_max_percent: Option<f64>,
    pub route_marker: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ContextSegment {
    pub duration_ms: u64,
    pub active_target_min: usize,
    pub active_target_max: usize,
    pub scored_samples: u64,
    pub scored_sample_rate: f64,
    pub frame_count: usize,
    pub frame_rate: f64,
    pub gpu_busy_min_percent: Option<f64>,
    pub gpu_busy_max_percent: Option<f64>,
    pub cpu_psi_some_min: Option<f64>,
    pub cpu_psi_some_max: Option<f64>,
    pub cpu_psi_some_avg: Option<f64>,
    pub scored_identity_counts: Vec<ScoredIdentityCount>,
    pub route_marker: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContextComparisonConfig {
    pub max_active_target_ratio: f64,
    pub max_scored_sample_rate_ratio: f64,
    pub max_frame_rate_ratio: f64,
    pub min_task_identity_overlap: f64,
    pub max_gpu_busy_range_delta_percent: f64,
    pub max_cpu_psi_some_delta: f64,
    pub require_matching_route_marker: bool,
}

impl Default for ContextComparisonConfig {
    fn default() -> Self {
        Self {
            max_active_target_ratio: 2.0,
            max_scored_sample_rate_ratio: 2.0,
            max_frame_rate_ratio: 1.5,
            min_task_identity_overlap: 0.75,
            max_gpu_busy_range_delta_percent: 25.0,
            max_cpu_psi_some_delta: 10.0,
            require_matching_route_marker: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContextComparability {
    Comparable,
    NotComparable { reasons: Vec<String> },
}

impl ContextComparability {
    pub fn is_comparable(&self) -> bool {
        matches!(self, Self::Comparable)
    }

    pub fn reasons(&self) -> &[String] {
        match self {
            Self::Comparable => &[],
            Self::NotComparable { reasons } => reasons,
        }
    }
}

impl ContextSegment {
    pub fn from_window(window: &RollingWindow, signals: ContextSignals) -> Self {
        let duration_ms = window.duration_ms();
        let duration_seconds = duration_ms as f64 / 1000.0;
        let scored_samples = window.scored_samples();
        let frame_count = window.frame_count();
        let scored_sample_rate = rate_per_second(scored_samples, duration_seconds);
        let frame_rate = rate_per_second(frame_count as u64, duration_seconds);

        let active_counts_by_elapsed = active_counts_by_elapsed(window);
        let active_target_min = active_counts_by_elapsed
            .values()
            .copied()
            .min()
            .unwrap_or(0);
        let active_target_max = active_counts_by_elapsed
            .values()
            .copied()
            .max()
            .unwrap_or(0);

        let cpu_psi_values = window
            .intervals
            .iter()
            .map(|record| record.cpu_psi_some)
            .filter(|value| value.is_finite())
            .collect::<Vec<_>>();

        let cpu_psi_some_min = finite_min(&cpu_psi_values);
        let cpu_psi_some_max = finite_max(&cpu_psi_values);
        let cpu_psi_some_avg = finite_avg(&cpu_psi_values);

        let mut scored_identity_counts = BTreeMap::new();
        for record in window.intervals.iter().filter(|record| record.samples > 0) {
            let identity = TaskIdentity {
                class: record.class,
                process_comm: record.process_comm.to_string(),
                comm: record.comm.clone(),
                process_starttime_ticks: None,
                task_starttime_ticks: None,
                exe_dev: None,
                exe_ino: None,
            };
            *scored_identity_counts.entry(identity).or_default() += 1;
        }

        Self {
            duration_ms,
            active_target_min,
            active_target_max,
            scored_samples,
            scored_sample_rate,
            frame_count,
            frame_rate,
            gpu_busy_min_percent: signals
                .gpu_busy_min_percent
                .filter(|value| value.is_finite()),
            gpu_busy_max_percent: signals
                .gpu_busy_max_percent
                .filter(|value| value.is_finite()),
            cpu_psi_some_min,
            cpu_psi_some_max,
            cpu_psi_some_avg,
            scored_identity_counts: scored_identity_map_to_counts(scored_identity_counts),
            route_marker: signals
                .route_marker
                .map(|marker| marker.trim().to_owned())
                .filter(|marker| !marker.is_empty()),
        }
    }

    pub fn task_identity_overlap_ratio(&self, other: &Self) -> f64 {
        task_identity_overlap_ratio(&self.scored_identity_counts, &other.scored_identity_counts)
    }
}

pub fn compare_context_segments(
    baseline: &ContextSegment,
    candidate: &ContextSegment,
    config: &ContextComparisonConfig,
) -> ContextComparability {
    if let Err(reason) = validate_context_comparison_config(config) {
        return ContextComparability::NotComparable {
            reasons: vec![reason],
        };
    }

    let mut reasons = Vec::new();

    push_ratio_reason(
        &mut reasons,
        "active target count",
        baseline.active_target_max,
        candidate.active_target_max,
        config.max_active_target_ratio,
    );

    push_f64_ratio_reason(
        &mut reasons,
        "scored sample rate",
        baseline.scored_sample_rate,
        candidate.scored_sample_rate,
        config.max_scored_sample_rate_ratio,
    );

    push_f64_ratio_reason(
        &mut reasons,
        "frame rate",
        baseline.frame_rate,
        candidate.frame_rate,
        config.max_frame_rate_ratio,
    );

    let identity_overlap = baseline.task_identity_overlap_ratio(candidate);
    if identity_overlap < config.min_task_identity_overlap {
        reasons.push(format!(
            "task identity overlap below threshold: overlap={:.1}% threshold={:.1}%",
            identity_overlap * 100.0,
            config.min_task_identity_overlap * 100.0
        ));
    }

    push_optional_range_delta_reason(
        &mut reasons,
        "GPU busy range",
        baseline.gpu_busy_min_percent,
        baseline.gpu_busy_max_percent,
        candidate.gpu_busy_min_percent,
        candidate.gpu_busy_max_percent,
        config.max_gpu_busy_range_delta_percent,
    );

    push_optional_value_delta_reason(
        &mut reasons,
        "CPU PSI some average",
        baseline.cpu_psi_some_avg,
        candidate.cpu_psi_some_avg,
        config.max_cpu_psi_some_delta,
    );

    if config.require_matching_route_marker && baseline.route_marker != candidate.route_marker {
        reasons.push(format!(
            "route/scenario marker mismatch: baseline={:?} candidate={:?}",
            baseline.route_marker, candidate.route_marker
        ));
    }

    if reasons.is_empty() {
        ContextComparability::Comparable
    } else {
        ContextComparability::NotComparable { reasons }
    }
}

fn validate_context_comparison_config(config: &ContextComparisonConfig) -> Result<(), String> {
    if !config.max_active_target_ratio.is_finite() || config.max_active_target_ratio < 1.0 {
        return Err(format!(
            "max_active_target_ratio must be finite and >= 1.0: {}",
            config.max_active_target_ratio
        ));
    }

    if !config.max_scored_sample_rate_ratio.is_finite() || config.max_scored_sample_rate_ratio < 1.0
    {
        return Err(format!(
            "max_scored_sample_rate_ratio must be finite and >= 1.0: {}",
            config.max_scored_sample_rate_ratio
        ));
    }

    if !config.max_frame_rate_ratio.is_finite() || config.max_frame_rate_ratio < 1.0 {
        return Err(format!(
            "max_frame_rate_ratio must be finite and >= 1.0: {}",
            config.max_frame_rate_ratio
        ));
    }

    if !config.min_task_identity_overlap.is_finite()
        || !(0.0..=1.0).contains(&config.min_task_identity_overlap)
    {
        return Err(format!(
            "min_task_identity_overlap must be finite and within 0.0..=1.0: {}",
            config.min_task_identity_overlap
        ));
    }

    if !config.max_gpu_busy_range_delta_percent.is_finite()
        || config.max_gpu_busy_range_delta_percent < 0.0
    {
        return Err(format!(
            "max_gpu_busy_range_delta_percent must be finite and non-negative: {}",
            config.max_gpu_busy_range_delta_percent
        ));
    }

    if !config.max_cpu_psi_some_delta.is_finite() || config.max_cpu_psi_some_delta < 0.0 {
        return Err(format!(
            "max_cpu_psi_some_delta must be finite and non-negative: {}",
            config.max_cpu_psi_some_delta
        ));
    }

    Ok(())
}

fn active_counts_by_elapsed(window: &RollingWindow) -> BTreeMap<u64, usize> {
    let mut counts = BTreeMap::new();

    for record in window.intervals.iter().filter(|record| record.active) {
        *counts.entry(record.elapsed_ms).or_default() += 1;
    }

    counts
}

fn task_identity_overlap_ratio(
    baseline_counts: &[ScoredIdentityCount],
    candidate_counts: &[ScoredIdentityCount],
) -> f64 {
    let baseline = scored_identity_counts_to_map(baseline_counts);
    let candidate = scored_identity_counts_to_map(candidate_counts);

    let common = scored_identity_overlap(&baseline, &candidate, usize::min);
    let total = scored_identity_overlap(&baseline, &candidate, usize::max);

    if total > 0 {
        common as f64 / total as f64
    } else {
        1.0
    }
}

fn push_ratio_reason(
    reasons: &mut Vec<String>,
    label: &str,
    baseline: usize,
    candidate: usize,
    max_ratio: f64,
) {
    match ratio_pair(baseline as f64, candidate as f64) {
        RatioPair::BothZero => {}
        RatioPair::ZeroMismatch => reasons.push(format!(
            "{label} is zero in one segment and nonzero in the other: baseline={baseline} candidate={candidate}"
        )),
        RatioPair::Ratio(ratio) if ratio > max_ratio => reasons.push(format!(
            "{label} differs beyond ratio threshold: baseline={baseline} candidate={candidate} ratio={ratio:.2} max_ratio={max_ratio:.2}"
        )),
        RatioPair::Ratio(_) => {}
    }
}

fn push_f64_ratio_reason(
    reasons: &mut Vec<String>,
    label: &str,
    baseline: f64,
    candidate: f64,
    max_ratio: f64,
) {
    match ratio_pair(baseline, candidate) {
        RatioPair::BothZero => {}
        RatioPair::ZeroMismatch => reasons.push(format!(
            "{label} is zero in one segment and nonzero in the other: baseline={baseline:.3} candidate={candidate:.3}"
        )),
        RatioPair::Ratio(ratio) if ratio > max_ratio => reasons.push(format!(
            "{label} differs beyond ratio threshold: baseline={baseline:.3} candidate={candidate:.3} ratio={ratio:.2} max_ratio={max_ratio:.2}"
        )),
        RatioPair::Ratio(_) => {}
    }
}

fn push_optional_range_delta_reason(
    reasons: &mut Vec<String>,
    label: &str,
    baseline_min: Option<f64>,
    baseline_max: Option<f64>,
    candidate_min: Option<f64>,
    candidate_max: Option<f64>,
    max_delta: f64,
) {
    match (baseline_min, baseline_max, candidate_min, candidate_max) {
        (None, None, None, None) => {}
        (Some(base_min), Some(base_max), Some(candidate_min), Some(candidate_max)) => {
            let min_delta = (base_min - candidate_min).abs();
            let max_delta_seen = (base_max - candidate_max).abs();
            let observed_delta = min_delta.max(max_delta_seen);

            if observed_delta > max_delta {
                reasons.push(format!(
                    "{label} differs beyond delta threshold: baseline={base_min:.1}..{base_max:.1} candidate={candidate_min:.1}..{candidate_max:.1} delta={observed_delta:.1} max_delta={max_delta:.1}"
                ));
            }
        }
        _ => reasons.push(format!(
            "{label} is present in one segment and missing or incomplete in the other"
        )),
    }
}

fn push_optional_value_delta_reason(
    reasons: &mut Vec<String>,
    label: &str,
    baseline: Option<f64>,
    candidate: Option<f64>,
    max_delta: f64,
) {
    match (baseline, candidate) {
        (None, None) => {}
        (Some(baseline), Some(candidate)) => {
            let delta = (baseline - candidate).abs();
            if delta > max_delta {
                reasons.push(format!(
                    "{label} differs beyond delta threshold: baseline={baseline:.3} candidate={candidate:.3} delta={delta:.3} max_delta={max_delta:.3}"
                ));
            }
        }
        _ => reasons.push(format!(
            "{label} is present in one segment and missing in the other"
        )),
    }
}

enum RatioPair {
    BothZero,
    ZeroMismatch,
    Ratio(f64),
}

fn ratio_pair(left: f64, right: f64) -> RatioPair {
    let left = if left.is_finite() && left > 0.0 {
        left
    } else {
        0.0
    };
    let right = if right.is_finite() && right > 0.0 {
        right
    } else {
        0.0
    };

    if left == 0.0 && right == 0.0 {
        RatioPair::BothZero
    } else if left == 0.0 || right == 0.0 {
        RatioPair::ZeroMismatch
    } else {
        RatioPair::Ratio(left.max(right) / left.min(right))
    }
}

fn rate_per_second(count: u64, duration_seconds: f64) -> f64 {
    if count == 0 || !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        0.0
    } else {
        count as f64 / duration_seconds
    }
}

fn finite_min(values: &[f64]) -> Option<f64> {
    values.iter().copied().reduce(f64::min)
}

fn finite_max(values: &[f64]) -> Option<f64> {
    values.iter().copied().reduce(f64::max)
}

fn finite_avg(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        None
    } else {
        Some(values.iter().copied().sum::<f64>() / values.len() as f64)
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use super::*;
    use crate::{metrics::IntervalRecord, process_tree::TaskClass, recorder::FrameEvent};

    #[allow(clippy::too_many_arguments)]
    fn record(
        elapsed_ms: u64,
        task: u32,
        active: bool,
        samples: u64,
        class: TaskClass,
        process_comm: &str,
        comm: &str,
        cpu_psi_some: f64,
    ) -> IntervalRecord {
        IntervalRecord {
            elapsed_ms,
            task,
            active,
            class,
            process_comm: Arc::from(process_comm),
            comm: comm.to_owned(),
            samples,
            cpu_psi_some,
            ..Default::default()
        }
    }

    fn frame(elapsed_ms: u64) -> FrameEvent {
        FrameEvent {
            elapsed_ms,
            frametime_ms: 16.0,
        }
    }

    fn window_with_game_context() -> RollingWindow {
        let mut window = RollingWindow::new(Duration::from_secs(10));
        window.push_interval(record(
            1000,
            10,
            true,
            50,
            TaskClass::Game,
            "Game.exe",
            "RenderThread",
            3.0,
        ));
        window.push_interval(record(
            1000,
            11,
            true,
            50,
            TaskClass::GameHelper,
            "Game.exe",
            "Worker",
            4.0,
        ));
        window.push_interval(record(
            2000,
            10,
            true,
            50,
            TaskClass::Game,
            "Game.exe",
            "RenderThread",
            5.0,
        ));
        window.push_interval(record(
            2000,
            11,
            true,
            50,
            TaskClass::GameHelper,
            "Game.exe",
            "Worker",
            6.0,
        ));

        for elapsed_ms in [1000, 2000, 3000, 4000, 5000] {
            window.push_frame(frame(elapsed_ms));
        }

        window
    }

    fn segment(marker: Option<&str>) -> ContextSegment {
        ContextSegment::from_window(
            &window_with_game_context(),
            ContextSignals {
                gpu_busy_min_percent: Some(70.0),
                gpu_busy_max_percent: Some(95.0),
                route_marker: marker.map(str::to_owned),
            },
        )
    }

    #[test]
    fn segment_from_window_tracks_active_targets_rates_gpu_cpu_psi_and_identities() {
        let segment = segment(Some("combat-arena"));

        assert_eq!(segment.duration_ms, 10_000);
        assert_eq!(segment.active_target_min, 2);
        assert_eq!(segment.active_target_max, 2);
        assert_eq!(segment.scored_samples, 200);
        assert_eq!(segment.scored_sample_rate, 20.0);
        assert_eq!(segment.frame_count, 5);
        assert_eq!(segment.frame_rate, 0.5);
        assert_eq!(segment.gpu_busy_min_percent, Some(70.0));
        assert_eq!(segment.gpu_busy_max_percent, Some(95.0));
        assert_eq!(segment.cpu_psi_some_min, Some(3.0));
        assert_eq!(segment.cpu_psi_some_max, Some(6.0));
        assert_eq!(segment.cpu_psi_some_avg, Some(4.5));
        assert_eq!(segment.route_marker.as_deref(), Some("combat-arena"));
        assert_eq!(segment.scored_identity_counts.len(), 2);
    }

    #[test]
    fn identical_contexts_are_comparable() {
        let baseline = segment(Some("combat-arena"));
        let candidate = segment(Some("combat-arena"));

        let result =
            compare_context_segments(&baseline, &candidate, &ContextComparisonConfig::default());

        assert!(result.is_comparable());
    }

    #[test]
    fn route_marker_mismatch_is_not_comparable() {
        let baseline = segment(Some("quiet-menu"));
        let candidate = segment(Some("combat-arena"));

        let result =
            compare_context_segments(&baseline, &candidate, &ContextComparisonConfig::default());

        assert!(!result.is_comparable());
        assert!(
            result
                .reasons()
                .iter()
                .any(|reason| reason.contains("route/scenario marker mismatch"))
        );
    }

    #[test]
    fn active_target_count_ratio_mismatch_is_not_comparable() {
        let baseline = segment(Some("combat-arena"));
        let mut candidate = segment(Some("combat-arena"));
        candidate.active_target_max = 8;

        let result =
            compare_context_segments(&baseline, &candidate, &ContextComparisonConfig::default());

        assert!(!result.is_comparable());
        assert!(
            result
                .reasons()
                .iter()
                .any(|reason| reason.contains("active target count differs beyond ratio threshold"))
        );
    }

    #[test]
    fn scored_sample_rate_mismatch_is_not_comparable() {
        let baseline = segment(Some("combat-arena"));
        let mut candidate = segment(Some("combat-arena"));
        candidate.scored_sample_rate = 50.0;

        let result =
            compare_context_segments(&baseline, &candidate, &ContextComparisonConfig::default());

        assert!(!result.is_comparable());
        assert!(
            result
                .reasons()
                .iter()
                .any(|reason| reason.contains("scored sample rate differs beyond ratio threshold"))
        );
    }

    #[test]
    fn frame_rate_zero_vs_nonzero_is_not_comparable() {
        let baseline = segment(Some("combat-arena"));
        let mut candidate = segment(Some("combat-arena"));
        candidate.frame_rate = 0.0;

        let result =
            compare_context_segments(&baseline, &candidate, &ContextComparisonConfig::default());

        assert!(!result.is_comparable());
        assert!(
            result
                .reasons()
                .iter()
                .any(|reason| reason.contains("frame rate is zero in one segment"))
        );
    }

    #[test]
    fn task_identity_overlap_below_threshold_is_not_comparable() {
        let baseline = segment(Some("combat-arena"));
        let mut candidate = segment(Some("combat-arena"));
        let mut replacement = BTreeMap::new();
        replacement.insert(
            TaskIdentity {
                class: TaskClass::BrowserForeground,
                process_comm: "firefox".to_owned(),
                comm: "Web Content".to_owned(),
                process_starttime_ticks: None,
                task_starttime_ticks: None,
                exe_dev: None,
                exe_ino: None,
            },
            4,
        );
        candidate.scored_identity_counts = scored_identity_map_to_counts(replacement);

        let result =
            compare_context_segments(&baseline, &candidate, &ContextComparisonConfig::default());

        assert!(!result.is_comparable());
        assert!(
            result
                .reasons()
                .iter()
                .any(|reason| reason.contains("task identity overlap below threshold"))
        );
    }

    #[test]
    fn gpu_busy_range_delta_mismatch_is_not_comparable() {
        let baseline = segment(Some("combat-arena"));
        let mut candidate = segment(Some("combat-arena"));
        candidate.gpu_busy_min_percent = Some(10.0);
        candidate.gpu_busy_max_percent = Some(20.0);

        let result =
            compare_context_segments(&baseline, &candidate, &ContextComparisonConfig::default());

        assert!(!result.is_comparable());
        assert!(
            result
                .reasons()
                .iter()
                .any(|reason| reason.contains("GPU busy range differs beyond delta threshold"))
        );
    }

    #[test]
    fn gpu_busy_present_only_on_one_side_is_not_comparable() {
        let baseline = segment(Some("combat-arena"));
        let mut candidate = segment(Some("combat-arena"));
        candidate.gpu_busy_min_percent = None;
        candidate.gpu_busy_max_percent = None;

        let result =
            compare_context_segments(&baseline, &candidate, &ContextComparisonConfig::default());

        assert!(!result.is_comparable());
        assert!(
            result
                .reasons()
                .iter()
                .any(|reason| reason.contains("GPU busy range is present in one segment"))
        );
    }

    #[test]
    fn cpu_psi_delta_mismatch_is_not_comparable() {
        let baseline = segment(Some("combat-arena"));
        let mut candidate = segment(Some("combat-arena"));
        candidate.cpu_psi_some_avg = Some(25.0);

        let result =
            compare_context_segments(&baseline, &candidate, &ContextComparisonConfig::default());

        assert!(!result.is_comparable());
        assert!(
            result.reasons().iter().any(
                |reason| reason.contains("CPU PSI some average differs beyond delta threshold")
            )
        );
    }

    #[test]
    fn route_marker_requirement_can_be_disabled() {
        let baseline = segment(Some("quiet-menu"));
        let candidate = segment(Some("combat-arena"));

        let result = compare_context_segments(
            &baseline,
            &candidate,
            &ContextComparisonConfig {
                require_matching_route_marker: false,
                ..ContextComparisonConfig::default()
            },
        );

        assert!(result.is_comparable());
    }

    #[test]
    fn invalid_config_is_not_comparable() {
        let baseline = segment(Some("combat-arena"));
        let candidate = segment(Some("combat-arena"));

        let result = compare_context_segments(
            &baseline,
            &candidate,
            &ContextComparisonConfig {
                max_active_target_ratio: 0.5,
                ..ContextComparisonConfig::default()
            },
        );

        assert!(!result.is_comparable());
        assert!(
            result
                .reasons()
                .iter()
                .any(|reason| reason.contains("max_active_target_ratio must be finite"))
        );
    }

    #[test]
    fn empty_identity_sets_have_full_overlap() {
        assert_eq!(task_identity_overlap_ratio(&[], &[]), 1.0);
    }
}
