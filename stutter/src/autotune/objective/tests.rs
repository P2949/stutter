use super::{decision::ObjectiveOutcome, *};
use crate::{
    autotune::{
        comparison::{ExperimentDataQuality, ExperimentResult},
        experiment::WindowScore,
    },
    scorer::StutterScore,
};

fn score(total: u64, frame_p99_ms: f64, over_5ms: u64) -> WindowScore {
    WindowScore {
        started_unix_nanos: 1,
        finished_unix_nanos: 2,
        interval_count: 5,
        scored_samples: 100,
        scored_task_count: 1,
        score: StutterScore {
            total,
            frame_p99_ms,
            over_5ms,
            ..StutterScore::default()
        },
    }
}

fn compare(
    objective: ObjectiveKind,
    baseline: &WindowScore,
    candidate: &WindowScore,
    baseline_signals: &ObjectiveSignals,
    candidate_signals: &ObjectiveSignals,
) -> ExperimentResult {
    compare_for_objective(ObjectiveComparisonInput {
        objective,
        baseline,
        candidate,
        baseline_signals,
        candidate_signals,
        data_quality: ExperimentDataQuality::High,
        target_disappeared: false,
    })
}

fn io_signals(score: &WindowScore, count: u64, worst_latency_ns: u64) -> ObjectiveSignals {
    ObjectiveSignals {
        block_io_overlap_count: Some(count),
        block_io_worst_latency_ns: Some(worst_latency_ns),
        ..ObjectiveSignals::from_window_score(score)
    }
}

fn irq_signals(score: &WindowScore, count: u64, worst_overlap_ns: u64) -> ObjectiveSignals {
    ObjectiveSignals {
        irq_overlap_count: Some(count),
        irq_worst_overlap_ns: Some(worst_overlap_ns),
        ..ObjectiveSignals::from_window_score(score)
    }
}

fn thermal_signals(score: &WindowScore, degraded: bool, throttle_count: u64) -> ObjectiveSignals {
    ObjectiveSignals {
        thermal_degraded: Some(degraded),
        thermal_throttle_count: Some(throttle_count),
        ..ObjectiveSignals::from_window_score(score)
    }
}

fn compile_signals(
    score: &WindowScore,
    progress_intervals: u64,
    progress_samples: u64,
) -> ObjectiveSignals {
    ObjectiveSignals {
        compile_progress_intervals: Some(progress_intervals),
        compile_progress_samples: Some(progress_samples),
        compile_progress_source: Some("test-progress".to_owned()),
        signal_quality: ObjectiveSignalQualitySnapshot {
            compile_throughput: ObjectiveSignalQuality::Direct,
            ..ObjectiveSignals::from_window_score(score).signal_quality
        },
        ..ObjectiveSignals::from_window_score(score)
    }
}

#[test]
fn game_objective_rejects_frame_p99_regression_even_when_total_improves() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(800, 14.0, 1);

    let baseline_signals = ObjectiveSignals::from_window_score(&baseline);
    let candidate_signals = ObjectiveSignals::from_window_score(&candidate);
    let result = compare(
        ObjectiveKind::GameFramePacing,
        &baseline,
        &candidate,
        &baseline_signals,
        &candidate_signals,
    );

    assert!(matches!(result, ExperimentResult::Regressed { .. }));
}

#[test]
fn desktop_objective_rejects_over_5ms_regression() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(800, 12.0, 2);

    let baseline_signals = ObjectiveSignals::from_window_score(&baseline);
    let candidate_signals = ObjectiveSignals::from_window_score(&candidate);
    let result = compare(
        ObjectiveKind::DesktopInteractivity,
        &baseline,
        &candidate,
        &baseline_signals,
        &candidate_signals,
    );

    assert!(matches!(result, ExperimentResult::Regressed { .. }));
}

#[test]
fn default_score_objective_keeps_existing_improvement_path() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(800, 12.0, 1);

    let baseline_signals = ObjectiveSignals::from_window_score(&baseline);
    let candidate_signals = ObjectiveSignals::from_window_score(&candidate);
    let result = compare(
        ObjectiveKind::StutterScore,
        &baseline,
        &candidate,
        &baseline_signals,
        &candidate_signals,
    );

    assert!(matches!(result, ExperimentResult::Improved { .. }));
}

#[test]
fn compile_objective_direct_signal_wins_over_proxy_regression() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(1_200, 12.0, 1);
    let baseline_signals = compile_signals(&baseline, 4, 400);
    let candidate_signals = compile_signals(&candidate, 8, 800);

    let result = compare(
        ObjectiveKind::CompileThroughputWithForegroundProtection,
        &baseline,
        &candidate,
        &baseline_signals,
        &candidate_signals,
    );

    assert!(matches!(result, ExperimentResult::Improved { .. }));
}

#[test]
fn compile_objective_missing_direct_signal_falls_back_to_proxy() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(800, 12.0, 1);
    let baseline_signals = ObjectiveSignals::from_window_score(&baseline);
    let candidate_signals = ObjectiveSignals::from_window_score(&candidate);

    let result = compare(
        ObjectiveKind::CompileThroughputWithForegroundProtection,
        &baseline,
        &candidate,
        &baseline_signals,
        &candidate_signals,
    );

    assert!(matches!(result, ExperimentResult::Improved { .. }));
}

#[test]
fn compile_throughput_signal_missing_direct_is_degraded_fallback() {
    let baseline = score(1_000, 12.0, 1);
    let signals = ObjectiveSignals::from_window_score(&baseline);

    let signal = signals.compile_throughput_signal();

    assert_eq!(signal.quality, ObjectiveSignalQuality::Derived);
    assert_eq!(signal.direct_progress(), None);
    assert!(
        signal
            .degraded_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("direct compile progress intervals missing"))
    );
}

#[test]
fn objective_decision_explains_direct_compile_signal() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(1_200, 12.0, 1);
    let baseline_signals = compile_signals(&baseline, 4, 400);
    let candidate_signals = compile_signals(&candidate, 8, 800);

    let decision = explain_for_objective(ObjectiveComparisonInput {
        objective: ObjectiveKind::CompileThroughputWithForegroundProtection,
        baseline: &baseline,
        candidate: &candidate,
        baseline_signals: &baseline_signals,
        candidate_signals: &candidate_signals,
        data_quality: ExperimentDataQuality::High,
        target_disappeared: false,
    });

    assert!(matches!(
        decision.outcome,
        ObjectiveOutcome::Improved { .. }
    ));
    assert_eq!(decision.primary_signal.name, "compile_throughput");
    assert_eq!(
        decision.primary_signal.quality,
        ObjectiveSignalQuality::Direct
    );
    assert!(decision.guard_failures.is_empty());
    assert_eq!(decision.confidence, crate::diagnosis::Confidence::High);
}

#[test]
fn objective_decision_records_guard_failures() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(800, 15.0, 1);
    let baseline_signals = compile_signals(&baseline, 4, 400);
    let candidate_signals = compile_signals(&candidate, 8, 800);

    let decision = explain_for_objective(ObjectiveComparisonInput {
        objective: ObjectiveKind::CompileThroughputWithForegroundProtection,
        baseline: &baseline,
        candidate: &candidate,
        baseline_signals: &baseline_signals,
        candidate_signals: &candidate_signals,
        data_quality: ExperimentDataQuality::High,
        target_disappeared: false,
    });

    assert!(matches!(
        decision.outcome,
        ObjectiveOutcome::Regressed { .. }
    ));
    assert!(
        decision
            .guard_failures
            .iter()
            .any(|failure| failure.guard == "foreground_frame_pacing")
    );
    assert!(decision.summary().contains("guard_failures="));
}

#[test]
fn io_objective_is_inconclusive_when_io_signal_does_not_improve() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(800, 12.0, 1);
    let baseline_signals = io_signals(&baseline, 1, 2_000_000);
    let candidate_signals = io_signals(&candidate, 1, 2_000_000);

    let result = compare(
        ObjectiveKind::IoLatency,
        &baseline,
        &candidate,
        &baseline_signals,
        &candidate_signals,
    );

    assert!(matches!(result, ExperimentResult::Inconclusive { .. }));
}

#[test]
fn io_objective_improves_when_io_overlap_drops_even_if_generic_score_is_flat() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(1_000, 12.0, 1);
    let baseline_signals = io_signals(&baseline, 3, 2_000_000);
    let candidate_signals = io_signals(&candidate, 1, 2_000_000);

    let result = compare(
        ObjectiveKind::IoLatency,
        &baseline,
        &candidate,
        &baseline_signals,
        &candidate_signals,
    );

    assert!(matches!(result, ExperimentResult::Improved { .. }));
}

#[test]
fn io_objective_rejects_when_io_overlap_worsens_even_if_generic_score_improves() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(800, 12.0, 1);
    let baseline_signals = io_signals(&baseline, 1, 2_000_000);
    let candidate_signals = io_signals(&candidate, 2, 3_000_000);

    let result = compare(
        ObjectiveKind::IoLatency,
        &baseline,
        &candidate,
        &baseline_signals,
        &candidate_signals,
    );

    assert!(matches!(result, ExperimentResult::Regressed { .. }));
}

#[test]
fn io_objective_rejects_when_required_signals_missing() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(800, 12.0, 1);
    let baseline_signals = ObjectiveSignals::from_window_score(&baseline);
    let candidate_signals = ObjectiveSignals::from_window_score(&candidate);

    let result = compare(
        ObjectiveKind::IoLatency,
        &baseline,
        &candidate,
        &baseline_signals,
        &candidate_signals,
    );

    assert!(matches!(result, ExperimentResult::Inconclusive { .. }));
}

#[test]
fn irq_objective_improves_when_overlap_drops_even_if_generic_score_is_flat() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(1_000, 12.0, 1);
    let baseline_signals = irq_signals(&baseline, 3, 5_000_000);
    let candidate_signals = irq_signals(&candidate, 1, 5_000_000);

    let result = compare(
        ObjectiveKind::IrqOverlapReduction,
        &baseline,
        &candidate,
        &baseline_signals,
        &candidate_signals,
    );

    assert!(matches!(result, ExperimentResult::Improved { .. }));
}

#[test]
fn irq_objective_rejects_when_overlap_worsens_even_if_generic_score_improves() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(800, 12.0, 1);
    let baseline_signals = irq_signals(&baseline, 1, 5_000_000);
    let candidate_signals = irq_signals(&candidate, 2, 6_000_000);

    let result = compare(
        ObjectiveKind::IrqOverlapReduction,
        &baseline,
        &candidate,
        &baseline_signals,
        &candidate_signals,
    );

    assert!(matches!(result, ExperimentResult::Regressed { .. }));
}

#[test]
fn irq_objective_rejects_when_required_signals_missing() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(800, 12.0, 1);
    let baseline_signals = ObjectiveSignals::from_window_score(&baseline);
    let candidate_signals = ObjectiveSignals::from_window_score(&candidate);

    let result = compare(
        ObjectiveKind::IrqOverlapReduction,
        &baseline,
        &candidate,
        &baseline_signals,
        &candidate_signals,
    );

    assert!(matches!(result, ExperimentResult::Inconclusive { .. }));
}

#[test]
fn thermal_objective_improves_when_candidate_clears_degraded_state() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(1_000, 12.0, 1);
    let baseline_signals = thermal_signals(&baseline, true, 1);
    let candidate_signals = thermal_signals(&candidate, false, 1);

    let result = compare(
        ObjectiveKind::ThermalRecovery,
        &baseline,
        &candidate,
        &baseline_signals,
        &candidate_signals,
    );

    assert!(matches!(result, ExperimentResult::Improved { .. }));
}

#[test]
fn thermal_objective_improves_when_throttle_count_drops() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(1_000, 12.0, 1);
    let baseline_signals = thermal_signals(&baseline, false, 3);
    let candidate_signals = thermal_signals(&candidate, false, 1);

    let result = compare(
        ObjectiveKind::ThermalRecovery,
        &baseline,
        &candidate,
        &baseline_signals,
        &candidate_signals,
    );

    assert!(matches!(result, ExperimentResult::Improved { .. }));
}

#[test]
fn thermal_objective_rejects_new_thermal_degradation() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(800, 12.0, 1);
    let baseline_signals = thermal_signals(&baseline, false, 0);
    let candidate_signals = thermal_signals(&candidate, true, 1);

    let result = compare(
        ObjectiveKind::ThermalRecovery,
        &baseline,
        &candidate,
        &baseline_signals,
        &candidate_signals,
    );

    assert!(matches!(result, ExperimentResult::Regressed { .. }));
}

#[test]
fn thermal_objective_rejects_new_power_limit() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(800, 12.0, 1);
    let baseline_signals = ObjectiveSignals {
        cpu_power_limited: Some(false),
        ..thermal_signals(&baseline, false, 0)
    };
    let candidate_signals = ObjectiveSignals {
        cpu_power_limited: Some(true),
        ..thermal_signals(&candidate, false, 0)
    };

    let result = compare(
        ObjectiveKind::ThermalRecovery,
        &baseline,
        &candidate,
        &baseline_signals,
        &candidate_signals,
    );

    assert!(matches!(result, ExperimentResult::Regressed { .. }));
}

#[test]
fn power_or_thermal_regression_rejects_frame_pacing_improvement() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(800, 11.0, 1);
    let baseline_signals = ObjectiveSignals {
        gpu_power_limited: Some(false),
        ..thermal_signals(&baseline, false, 0)
    };
    let candidate_signals = ObjectiveSignals {
        gpu_power_limited: Some(true),
        ..thermal_signals(&candidate, true, 1)
    };

    let result = compare(
        ObjectiveKind::GameFramePacing,
        &baseline,
        &candidate,
        &baseline_signals,
        &candidate_signals,
    );

    assert!(matches!(result, ExperimentResult::Regressed { .. }));
}
