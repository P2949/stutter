use super::*;
use crate::workload_progress::tests::markers;

pub(super) fn fixed_work(score: &WindowScore, duration_ns: u64, id: &str) -> ObjectiveSignals {
    let mut work = markers(duration_ns, id);
    if id != "baseline" {
        work.start.monotonic_ns += 100_000_000_000;
        work.complete.monotonic_ns += 100_000_000_000;
    }
    let mut signals = ObjectiveSignals {
        compile_fixed_work: Some(work),
        ..ObjectiveSignals::from_window_score(score)
    };
    signals.signal_quality.compile_throughput = ObjectiveSignalQuality::Direct;
    signals
}

#[test]
fn faster_identical_completed_compile_work_improves_despite_score_regression() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(1_200, 12.0, 1);
    let result = compare(
        ObjectiveKind::CompileThroughputWithForegroundProtection,
        &baseline,
        &candidate,
        &fixed_work(&baseline, 31_400_000_000, "baseline"),
        &fixed_work(&candidate, 28_000_000_000, "candidate"),
    );
    assert!(
        matches!(result, ExperimentResult::Improved { .. }),
        "{result:?}"
    );
}

#[test]
fn fixed_work_compares_completion_time_instead_of_activity_or_score() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(800, 12.0, 1);
    for (duration, expected) in [
        (40_000_000_000, "regressed"),
        (30_000_000_000, "inconclusive"),
        (29_999_000_000, "inconclusive"),
        (20_000_000_000, "improved"),
    ] {
        let decision = explain_for_objective(ObjectiveComparisonInput {
            objective: ObjectiveKind::CompileThroughputWithForegroundProtection,
            baseline: &baseline,
            candidate: &candidate,
            baseline_signals: &fixed_work(&baseline, 30_000_000_000, "baseline"),
            candidate_signals: &fixed_work(&candidate, duration, "candidate"),
            data_quality: ExperimentDataQuality::High,
            target_disappeared: false,
        });
        let actual = match decision.experiment_result() {
            ExperimentResult::Improved { .. } => "improved",
            ExperimentResult::Regressed { .. } => "regressed",
            ExperimentResult::Inconclusive { .. } => "inconclusive",
            ExperimentResult::Invalid { .. } => "invalid",
        };
        assert_eq!(actual, expected);
        assert_eq!(
            decision.primary_signal.name,
            "compile_fixed_work_elapsed_ns"
        );
        assert_eq!(
            decision.primary_signal.baseline.as_deref(),
            Some("30000000000")
        );
        assert!(decision.summary().contains("no score fallback"));
    }
}

#[test]
fn mismatched_or_incomplete_compile_work_is_inconclusive() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(800, 12.0, 1);
    for case in 0..19 {
        let base = fixed_work(&baseline, 30_000_000_000, "baseline");
        let mut cand = fixed_work(&candidate, 20_000_000_000, "candidate");
        if case <= 12 {
            let work = cand.compile_fixed_work.as_mut().unwrap();
            match case {
                0 => {
                    work.start.identity.target = "different".to_owned();
                    work.complete.identity = work.start.identity.clone();
                }
                1 => {
                    work.start.identity.source_tree = format!("sha256:{}", "b".repeat(64));
                    work.complete.identity = work.start.identity.clone();
                }
                2 => {
                    work.start.identity.job_count = 8;
                    work.complete.identity = work.start.identity.clone();
                }
                3 => {
                    work.start.identity.build_state = "incremental".to_owned();
                    work.complete.identity = work.start.identity.clone();
                }
                4 => {
                    work.start.identity.cache_policy = "warm".to_owned();
                    work.complete.identity = work.start.identity.clone();
                }
                5 => {
                    work.start.identity.command.push("other-target".to_owned());
                    work.complete.identity = work.start.identity.clone();
                }
                6 => {
                    work.start.identity.toolchain = "different".to_owned();
                    work.complete.identity = work.start.identity.clone();
                }
                7 => {
                    work.start.identity.environment = "different".to_owned();
                    work.complete.identity = work.start.identity.clone();
                }
                8 => {
                    work.start.execution_id = "baseline".to_owned();
                    work.complete.execution_id = "baseline".to_owned();
                }
                9 => {
                    work.start.monotonic_ns = 1_000;
                    work.complete.monotonic_ns = 20_000_001_000;
                }
                10 => work.complete.phase = crate::workload_progress::WorkloadPhase::Failed,
                11 => work.complete.completed_units = 0,
                _ => {
                    work.start.identity.kind = crate::workload_progress::WorkloadKind::Custom;
                    work.complete.identity = work.start.identity.clone();
                }
            }
        } else {
            match case {
                13 => cand.signal_quality.compile_throughput = ObjectiveSignalQuality::Approximate,
                14 => cand.signal_quality.foreground_latency = ObjectiveSignalQuality::Missing,
                15 => cand.compile_fixed_work = None,
                16 => cand.memory_pressure_some_avg10_percent = Some(f32::NAN),
                17 => {
                    let work = cand.compile_fixed_work.as_mut().unwrap();
                    work.start.target_root_pid += 1;
                    work.complete.target_root_pid = work.start.target_root_pid;
                }
                _ => {
                    let work = cand.compile_fixed_work.as_mut().unwrap();
                    work.start.target_starttime_ticks += 1;
                    work.complete.target_starttime_ticks = work.start.target_starttime_ticks;
                }
            }
        }
        let result = compare(
            ObjectiveKind::CompileThroughputWithForegroundProtection,
            &baseline,
            &candidate,
            &base,
            &cand,
        );
        assert!(
            matches!(result, ExperimentResult::Inconclusive { .. }),
            "case {case}: {result:?}"
        );
    }
}

#[test]
fn faster_compile_rejects_foreground_power_thermal_and_memory_regressions() {
    let baseline = score(1_000, 12.0, 1);
    for guard in ["foreground", "frame", "thermal", "cpu", "gpu", "memory"] {
        let mut candidate = score(800, 12.0, 1);
        let mut base = fixed_work(&baseline, 30_000_000_000, "baseline");
        let mut cand = fixed_work(&candidate, 20_000_000_000, "candidate");
        match guard {
            "foreground" => cand.foreground_over_5ms = Some(2),
            "frame" => candidate.score.frame_p99_ms = 15.0,
            "thermal" => {
                base.thermal_degraded = Some(false);
                cand.thermal_degraded = Some(true);
            }
            "cpu" => {
                base.cpu_power_limited = Some(false);
                cand.cpu_power_limited = Some(true);
            }
            "gpu" => {
                base.gpu_power_limited = Some(false);
                cand.gpu_power_limited = Some(true);
            }
            _ => {
                base.memory_pressure_some_avg10_percent = Some(0.0);
                cand.memory_pressure_some_avg10_percent = Some(1.0);
            }
        }
        let result = compare(
            ObjectiveKind::CompileThroughputWithForegroundProtection,
            &baseline,
            &candidate,
            &base,
            &cand,
        );
        assert!(
            matches!(result, ExperimentResult::Regressed { .. }),
            "{guard}: {result:?}"
        );
    }
}
