use super::*;

fn runnable(score: &WindowScore, p99: u64) -> ObjectiveSignals {
    ObjectiveSignals {
        runnable_latency_p99_ns: Some(p99),
        runnable_latency_p95_ns: Some(p99),
        runnable_latency_max_ns: Some(p99),
        runnable_samples: Some(10_000),
        runnable_over_1ms: Some(0),
        runnable_over_2ms: Some(0),
        runnable_over_5ms: Some(0),
        runnable_latency_source: Some("target-game-scheduler-histogram-v1".to_owned()),
        runnable_target_root_pid: Some(42),
        runnable_target_starttime_ticks: Some(100),
        signal_quality: ObjectiveSignalQualitySnapshot {
            runnable_latency: ObjectiveSignalQuality::Direct,
            ..ObjectiveSignals::from_window_score(score).signal_quality
        },
        ..ObjectiveSignals::from_window_score(score)
    }
}

#[test]
fn runnable_regression_wins_over_generic_score_improvement() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(800, 12.0, 1);
    let result = compare(
        ObjectiveKind::GameRunnableLatency,
        &baseline,
        &candidate,
        &runnable(&baseline, 2_000_000),
        &runnable(&candidate, 4_000_000),
    );
    assert!(
        matches!(result, ExperimentResult::Regressed { .. }),
        "{result:?}"
    );
}

#[test]
fn runnable_improvement_detected_with_flat_generic_score() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = baseline.clone();
    let result = compare(
        ObjectiveKind::GameRunnableLatency,
        &baseline,
        &candidate,
        &runnable(&baseline, 4_000_000),
        &runnable(&candidate, 2_000_000),
    );
    assert!(
        matches!(result, ExperimentResult::Improved { .. }),
        "{result:?}"
    );
}

#[test]
fn runnable_primary_can_improve_despite_generic_score_regression() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(1_200, 12.0, 1);
    let result = compare(
        ObjectiveKind::GameRunnableLatency,
        &baseline,
        &candidate,
        &runnable(&baseline, 4_000_000),
        &runnable(&candidate, 2_000_000),
    );
    assert!(matches!(
        result,
        ExperimentResult::Improved {
            improvement_percent: 50.0
        }
    ));
}

#[test]
fn runnable_missing_approximate_insufficient_or_inconsistent_is_inconclusive() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(800, 12.0, 1);
    for case in 0..12 {
        let good = runnable(&baseline, 4_000_000);
        let mut bad = runnable(&candidate, 2_000_000);
        match case {
            0 => bad.runnable_latency_p99_ns = None,
            1 => bad.runnable_samples = Some(40),
            2 => bad.runnable_samples = None,
            3 => bad.signal_quality.runnable_latency = ObjectiveSignalQuality::Approximate,
            4 => bad.signal_quality.runnable_latency = ObjectiveSignalQuality::Missing,
            5 => bad.runnable_latency_source = None,
            6 => bad.runnable_over_5ms = Some(10_001),
            7 => bad.runnable_latency_max_ns = Some(1),
            8 => bad.runnable_target_root_pid = None,
            9 => bad.runnable_target_root_pid = Some(99),
            10 => bad.runnable_target_starttime_ticks = None,
            _ => bad.runnable_target_starttime_ticks = Some(101),
        }
        for (base_signals, candidate_signals) in [(&good, &bad), (&bad, &good)] {
            let decision = explain_for_objective(ObjectiveComparisonInput {
                objective: ObjectiveKind::GameRunnableLatency,
                baseline: &baseline,
                candidate: &candidate,
                baseline_signals: base_signals,
                candidate_signals,
                data_quality: ExperimentDataQuality::High,
                target_disappeared: false,
            });
            let result = decision.experiment_result();
            assert!(
                matches!(result, ExperimentResult::Inconclusive { .. }),
                "case {case}: {result:?}"
            );
            assert_eq!(
                decision.confidence,
                crate::diagnosis::Confidence::Low,
                "case {case}"
            );
        }
    }
}

#[test]
fn runnable_equal_or_small_primary_changes_are_inconclusive() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(800, 12.0, 1);
    for p99 in [3_999_999, 4_000_000, 4_000_001] {
        assert!(matches!(
            compare(
                ObjectiveKind::GameRunnableLatency,
                &baseline,
                &candidate,
                &runnable(&baseline, 4_000_000),
                &runnable(&candidate, p99)
            ),
            ExperimentResult::Inconclusive { .. }
        ));
    }
}

#[test]
fn runnable_tail_uses_population_rates_and_rejects_new_severe_events() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = baseline.clone();
    let mut base = runnable(&baseline, 4_000_000);
    let mut cand = runnable(&candidate, 2_000_000);
    base.runnable_samples = Some(1_000);
    base.runnable_over_5ms = Some(10);
    cand.runnable_samples = Some(100_000);
    cand.runnable_over_5ms = Some(11);
    assert!(matches!(
        compare(
            ObjectiveKind::GameRunnableLatency,
            &baseline,
            &candidate,
            &base,
            &cand
        ),
        ExperimentResult::Improved { .. }
    ));
    cand.runnable_over_5ms = Some(1_001);
    assert!(matches!(
        compare(
            ObjectiveKind::GameRunnableLatency,
            &baseline,
            &candidate,
            &base,
            &cand
        ),
        ExperimentResult::Regressed { .. }
    ));
    base.runnable_over_5ms = Some(0);
    cand.runnable_over_5ms = Some(1);
    let decision = explain_for_objective(ObjectiveComparisonInput {
        objective: ObjectiveKind::GameRunnableLatency,
        baseline: &baseline,
        candidate: &candidate,
        baseline_signals: &base,
        candidate_signals: &cand,
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
            .any(|guard| guard.guard == "runnable_severe_tail")
    );
}

#[test]
fn runnable_improvement_is_rejected_by_each_safety_guardrail() {
    let baseline = score(1_000, 12.0, 1);
    for guard in ["frame", "foreground", "thermal", "throttle", "cpu", "gpu"] {
        let mut candidate = baseline.clone();
        let mut base = runnable(&baseline, 4_000_000);
        let mut cand = runnable(&candidate, 2_000_000);
        match guard {
            "frame" => {
                candidate.score.frame_p99_ms = 14.0;
                cand.frame_p99_ms = Some(14.0);
            }
            "foreground" => cand.foreground_over_5ms = Some(2),
            "thermal" => {
                base.thermal_degraded = Some(false);
                cand.thermal_degraded = Some(true);
            }
            "throttle" => {
                base.thermal_throttle_count = Some(0);
                cand.thermal_throttle_count = Some(1);
            }
            "cpu" => {
                base.cpu_power_limited = Some(false);
                cand.cpu_power_limited = Some(true);
            }
            "gpu" => {
                base.gpu_power_limited = Some(false);
                cand.gpu_power_limited = Some(true);
            }
            _ => unreachable!(),
        }
        assert!(
            matches!(
                compare(
                    ObjectiveKind::GameRunnableLatency,
                    &baseline,
                    &candidate,
                    &base,
                    &cand
                ),
                ExperimentResult::Regressed { .. }
            ),
            "{guard}"
        );
    }
}

#[test]
fn runnable_quality_and_target_loss_cannot_authorize_improvement() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = baseline.clone();
    let base = runnable(&baseline, 4_000_000);
    let cand = runnable(&candidate, 2_000_000);
    for (data_quality, target_disappeared) in [
        (ExperimentDataQuality::Low, false),
        (ExperimentDataQuality::High, true),
    ] {
        assert!(!matches!(
            compare_for_objective(ObjectiveComparisonInput {
                objective: ObjectiveKind::GameRunnableLatency,
                baseline: &baseline,
                candidate: &candidate,
                baseline_signals: &base,
                candidate_signals: &cand,
                data_quality,
                target_disappeared,
            }),
            ExperimentResult::Improved { .. }
        ));
    }
}

#[test]
fn old_objective_artifact_keeps_missing_runnable_evidence_distinct_from_zero() {
    let baseline = score(1_000, 12.0, 1);
    let mut value = serde_json::to_value(ObjectiveSignals::from_window_score(&baseline)).unwrap();
    value["signal_quality"]
        .as_object_mut()
        .unwrap()
        .remove("runnable_latency");
    let signals: ObjectiveSignals = serde_json::from_value(value).unwrap();
    assert_eq!(signals.runnable_latency_p99_ns, None);
    assert_eq!(
        signals.signal_quality.runnable_latency,
        ObjectiveSignalQuality::Missing
    );
    let decision = explain_for_objective(ObjectiveComparisonInput {
        objective: ObjectiveKind::GameRunnableLatency,
        baseline: &baseline,
        candidate: &baseline,
        baseline_signals: &signals,
        candidate_signals: &signals,
        data_quality: ExperimentDataQuality::High,
        target_disappeared: false,
    });
    assert_eq!(decision.primary_signal.baseline, None);
    assert_eq!(decision.primary_signal.candidate, None);
    assert!(decision.summary().contains("unavailable"));
    assert!(matches!(
        decision.outcome,
        ObjectiveOutcome::Inconclusive { .. }
    ));
}

#[test]
fn runnable_decision_reports_primary_values_samples_and_no_fallback() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = baseline.clone();
    let decision = explain_for_objective(ObjectiveComparisonInput {
        objective: ObjectiveKind::GameRunnableLatency,
        baseline: &baseline,
        candidate: &candidate,
        baseline_signals: &runnable(&baseline, 4_000_000),
        candidate_signals: &runnable(&candidate, 2_000_000),
        data_quality: ExperimentDataQuality::High,
        target_disappeared: false,
    });
    assert_eq!(decision.primary_signal.name, "runnable_p99_ns");
    assert_eq!(decision.primary_signal.baseline.as_deref(), Some("4000000"));
    assert_eq!(
        decision.primary_signal.candidate.as_deref(),
        Some("2000000")
    );
    assert_eq!(
        decision.primary_signal.quality,
        ObjectiveSignalQuality::Direct
    );
    assert!(
        decision
            .summary()
            .contains("samples baseline=10000 candidate=10000")
    );
    assert!(decision.summary().contains("no fallback"));
}
