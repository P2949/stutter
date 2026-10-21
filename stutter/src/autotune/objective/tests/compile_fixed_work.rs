use super::*;
use crate::workload_progress::tests::markers;

fn fixed_work(score: &WindowScore, duration_ns: u64, id: &str) -> ObjectiveSignals {
    let mut work = markers(duration_ns, id);
    if id != "baseline" { work.start.monotonic_ns += 100_000_000_000; work.complete.monotonic_ns += 100_000_000_000; }
    let mut signals = ObjectiveSignals { compile_fixed_work: Some(work), ..ObjectiveSignals::from_window_score(score) };
    signals.signal_quality.compile_throughput = ObjectiveSignalQuality::Direct;
    signals
}

#[test]
fn faster_identical_completed_compile_work_improves_despite_score_regression() {
    let baseline = score(1_000, 12.0, 1);
    let candidate = score(1_200, 12.0, 1);
    let result = compare(ObjectiveKind::CompileThroughputWithForegroundProtection, &baseline, &candidate,
        &fixed_work(&baseline, 31_400_000_000, "baseline"), &fixed_work(&candidate, 28_000_000_000, "candidate"));
    assert!(matches!(result, ExperimentResult::Improved { .. }), "{result:?}");
}
