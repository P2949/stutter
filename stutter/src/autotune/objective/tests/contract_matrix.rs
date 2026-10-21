//! Shared behavioral contracts. Fixtures vary only the objective's primary metric;
//! common safety cases retain a clearly improving primary metric.
use super::*;

const OBJECTIVES: [ObjectiveKind; 9] = [
    ObjectiveKind::StutterScore,
    ObjectiveKind::GameFramePacing,
    ObjectiveKind::GameRunnableLatency,
    ObjectiveKind::DesktopInteractivity,
    ObjectiveKind::BrowserInteractivity,
    ObjectiveKind::CompileThroughputWithForegroundProtection,
    ObjectiveKind::IoLatency,
    ObjectiveKind::IrqOverlapReduction,
    ObjectiveKind::ThermalRecovery,
];

#[derive(Clone, Copy, Debug)]
enum Change {
    Improve,
    Regress,
    Equal,
}

fn signals(
    kind: ObjectiveKind,
    score: &WindowScore,
    value: u64,
    candidate: bool,
) -> ObjectiveSignals {
    let mut signals = match kind {
        ObjectiveKind::StutterScore => ObjectiveSignals::from_window_score(score),
        ObjectiveKind::GameFramePacing => ObjectiveSignals {
            frame_p99_ms: Some(value as f64),
            ..ObjectiveSignals::from_window_score(score)
        },
        ObjectiveKind::GameRunnableLatency => runnable::runnable(score, value * 1_000_000),
        ObjectiveKind::DesktopInteractivity | ObjectiveKind::BrowserInteractivity => {
            ObjectiveSignals {
                foreground_over_5ms: Some(value),
                ..ObjectiveSignals::from_window_score(score)
            }
        }
        ObjectiveKind::CompileThroughputWithForegroundProtection => compile_fixed_work::fixed_work(
            score,
            value * 1_000_000_000,
            if candidate { "candidate" } else { "baseline" },
        ),
        ObjectiveKind::IoLatency => io_signals(score, value, 2_000_000),
        ObjectiveKind::IrqOverlapReduction => irq_signals(score, value, 2_000_000),
        ObjectiveKind::ThermalRecovery => thermal_signals(score, false, value),
    };
    signals.thermal_degraded = Some(false);
    signals.cpu_power_limited = Some(false);
    signals.gpu_power_limited = Some(false);
    signals
}

fn fixture(
    kind: ObjectiveKind,
    change: Change,
) -> (WindowScore, WindowScore, ObjectiveSignals, ObjectiveSignals) {
    let baseline = score(1_000, 12.0, 1);
    let mut candidate = baseline.clone();
    let value = match change {
        Change::Improve => 2,
        Change::Regress => 8,
        Change::Equal => 4,
    };
    if kind == ObjectiveKind::StutterScore {
        candidate.score.total = match change {
            Change::Improve => 700,
            Change::Regress => 1_300,
            Change::Equal => 1_000,
        };
    }
    let base_signals = signals(kind, &baseline, 4, false);
    let candidate_signals = signals(kind, &candidate, value, true);
    (baseline, candidate, base_signals, candidate_signals)
}

#[test]
fn contract_matrix_primary_improvement_regression_and_unchanged_for_every_objective() {
    for kind in OBJECTIVES {
        for change in [Change::Improve, Change::Regress, Change::Equal] {
            let (baseline, candidate, base_signals, candidate_signals) = fixture(kind, change);
            let result = compare(
                kind,
                &baseline,
                &candidate,
                &base_signals,
                &candidate_signals,
            );
            let correct = match change {
                Change::Improve => matches!(result, ExperimentResult::Improved { .. }),
                Change::Regress => matches!(result, ExperimentResult::Regressed { .. }),
                Change::Equal => matches!(result, ExperimentResult::Inconclusive { .. }),
            };
            assert!(correct, "{kind:?} {change:?}: {result:?}");
        }
    }
}

#[test]
fn contract_matrix_common_safety_rejects_improving_primary_for_every_objective() {
    for kind in OBJECTIVES {
        for failure in [
            "target_disappeared",
            "low_quality",
            "thermal",
            "throttle",
            "cpu_power",
            "gpu_power",
            "zero_samples",
        ] {
            let (baseline, mut candidate, base_signals, mut candidate_signals) =
                fixture(kind, Change::Improve);
            match failure {
                "thermal" => candidate_signals.thermal_degraded = Some(true),
                "throttle" => {
                    candidate_signals.thermal_throttle_count =
                        Some(base_signals.thermal_throttle_count.unwrap_or(0) + 1)
                }
                "cpu_power" => candidate_signals.cpu_power_limited = Some(true),
                "gpu_power" => candidate_signals.gpu_power_limited = Some(true),
                "zero_samples" => candidate.scored_samples = 0,
                _ => {}
            }
            // Set an observed zero baseline for the global throttle-count guard.
            let mut base_signals = base_signals;
            if failure == "throttle" && base_signals.thermal_throttle_count.is_none() {
                base_signals.thermal_throttle_count = Some(0);
            }
            let result = compare_for_objective(ObjectiveComparisonInput {
                objective: kind,
                baseline: &baseline,
                candidate: &candidate,
                baseline_signals: &base_signals,
                candidate_signals: &candidate_signals,
                data_quality: if failure == "low_quality" {
                    ExperimentDataQuality::Low
                } else {
                    ExperimentDataQuality::High
                },
                target_disappeared: failure == "target_disappeared",
            });
            if failure == "zero_samples" {
                assert!(
                    matches!(result, ExperimentResult::Invalid { .. }),
                    "{kind:?}: {result:?}"
                );
            } else {
                assert!(
                    matches!(result, ExperimentResult::Regressed { .. }),
                    "{kind:?} {failure}: {result:?}"
                );
            }
        }
    }
}

#[test]
fn contract_matrix_missing_primary_distinguishes_explicit_fallback_from_no_fallback() {
    for kind in OBJECTIVES {
        let baseline = score(1_000, 12.0, 1);
        let candidate = score(700, 12.0, 1);
        let missing = ObjectiveSignals::default();
        let result = compare(kind, &baseline, &candidate, &missing, &missing);
        let fallback = match kind {
            ObjectiveKind::StutterScore
            | ObjectiveKind::GameFramePacing
            | ObjectiveKind::DesktopInteractivity
            | ObjectiveKind::BrowserInteractivity => true,
            ObjectiveKind::GameRunnableLatency
            | ObjectiveKind::CompileThroughputWithForegroundProtection
            | ObjectiveKind::IoLatency
            | ObjectiveKind::IrqOverlapReduction
            | ObjectiveKind::ThermalRecovery => false,
        };
        if fallback {
            assert!(
                matches!(result, ExperimentResult::Improved { .. }),
                "{kind:?}: {result:?}"
            );
        } else {
            assert!(
                matches!(result, ExperimentResult::Inconclusive { .. }),
                "{kind:?}: {result:?}"
            );
        }
    }
}

#[test]
fn contract_matrix_generic_score_conflict_preserves_primary_and_declared_score_guards() {
    for kind in OBJECTIVES {
        let (baseline, mut candidate, base_signals, candidate_signals) =
            fixture(kind, Change::Improve);
        candidate.score.total = 1_300;
        let result = compare(
            kind,
            &baseline,
            &candidate,
            &base_signals,
            &candidate_signals,
        );
        match kind {
            ObjectiveKind::GameRunnableLatency
            | ObjectiveKind::CompileThroughputWithForegroundProtection => {
                assert!(
                    matches!(result, ExperimentResult::Improved { .. }),
                    "{kind:?}: {result:?}"
                );
            }
            _ => assert!(
                matches!(result, ExperimentResult::Regressed { .. }),
                "{kind:?}: {result:?}"
            ),
        }
    }
}

#[test]
fn contract_matrix_score_normalization_survives_large_sample_imbalance() {
    let baseline = score(1_000, 12.0, 0);
    let mut candidate = score(10_000, 12.0, 0);
    candidate.scored_samples = 1_000;
    let result = compare(
        ObjectiveKind::StutterScore,
        &baseline,
        &candidate,
        &ObjectiveSignals::default(),
        &ObjectiveSignals::default(),
    );
    assert!(
        matches!(result, ExperimentResult::Inconclusive { .. }),
        "{result:?}"
    );
    candidate.score.total = 7_000;
    let result = compare(
        ObjectiveKind::StutterScore,
        &baseline,
        &candidate,
        &ObjectiveSignals::default(),
        &ObjectiveSignals::default(),
    );
    assert!(
        matches!(result, ExperimentResult::Improved { .. }),
        "{result:?}"
    );
}

#[test]
fn contract_matrix_io_and_irq_worst_latency_is_independent_of_overlap_count() {
    for kind in [ObjectiveKind::IoLatency, ObjectiveKind::IrqOverlapReduction] {
        let baseline = score(1_000, 12.0, 1);
        let candidate = baseline.clone();
        for (worst_ns, improved) in [(1_000_000, true), (3_000_000, false)] {
            let (base_signals, candidate_signals) = match kind {
                ObjectiveKind::IoLatency => (
                    io_signals(&baseline, 4, 2_000_000),
                    io_signals(&candidate, 4, worst_ns),
                ),
                ObjectiveKind::IrqOverlapReduction => (
                    irq_signals(&baseline, 4, 2_000_000),
                    irq_signals(&candidate, 4, worst_ns),
                ),
                _ => unreachable!("test cases contain only overlap objectives"),
            };
            let result = compare(
                kind,
                &baseline,
                &candidate,
                &base_signals,
                &candidate_signals,
            );
            assert!(
                if improved {
                    matches!(result, ExperimentResult::Improved { .. })
                } else {
                    matches!(result, ExperimentResult::Regressed { .. })
                },
                "{kind:?} {worst_ns}: {result:?}"
            );
        }
    }
}
