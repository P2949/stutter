use super::*;

fn run(scheduler: &str, block: u32, value: f64) -> SchedulerRun {
    SchedulerRun {
        scheduler: scheduler.into(),
        kernel: "6.18.57".into(),
        workload_id: "build".into(),
        workload_digest: "d1".into(),
        block_id: block,
        metric: "elapsed".into(),
        unit: "s".into(),
        lower_is_better: true,
        value,
    }
}

fn set(base: &[f64], cand: &[f64]) -> Vec<SchedulerRun> {
    base.iter()
        .enumerate()
        .map(|(i, v)| run("default", i as u32, *v))
        .chain(
            cand.iter()
                .enumerate()
                .map(|(i, v)| run("lavd", i as u32, *v)),
        )
        .collect()
}

#[test]
fn matched_blocks_give_a_signed_improvement_and_a_verdict() {
    let better = compare_schedulers(
        &set(&[10.0, 10.2, 9.9, 10.1], &[9.0, 9.1, 8.9, 9.0]),
        "default",
        "lavd",
        0.95,
    )
    .unwrap();
    assert_eq!(better.verdict, Verdict::CandidateBetter);
    assert!(better.interval.mean > 8.0 && better.interval.mean < 11.0);
    let worse = compare_schedulers(
        &set(&[10.0, 10.2, 9.9, 10.1], &[11.0, 11.2, 10.9, 11.1]),
        "default",
        "lavd",
        0.95,
    )
    .unwrap();
    assert_eq!(worse.verdict, Verdict::CandidateWorse);
    let noisy = compare_schedulers(
        &set(&[10.0, 10.0, 10.0, 10.0], &[9.0, 11.0, 8.5, 11.5]),
        "default",
        "lavd",
        0.95,
    )
    .unwrap();
    assert_eq!(noisy.verdict, Verdict::Inconclusive);
}

#[test]
fn higher_is_better_flips_the_sign() {
    let mut runs = set(&[100.0, 100.0, 101.0], &[120.0, 121.0, 119.0]);
    for run in &mut runs {
        run.lower_is_better = false;
    }
    let result = compare_schedulers(&runs, "default", "lavd", 0.95).unwrap();
    assert_eq!(result.verdict, Verdict::CandidateBetter);
}

#[test]
fn a_mismatched_unpaired_duplicate_or_thin_comparison_is_refused() {
    let base = set(&[10.0, 10.0, 10.0], &[9.0, 9.0, 9.0]);
    let mut other_kernel = base.clone();
    other_kernel[3].kernel = "7.2".into();
    assert_eq!(
        compare_schedulers(&other_kernel, "default", "lavd", 0.95),
        Err(CompareRefused::Mismatched("kernel"))
    );
    let mut other_workload = base.clone();
    other_workload[4].workload_digest = "d2".into();
    assert_eq!(
        compare_schedulers(&other_workload, "default", "lavd", 0.95),
        Err(CompareRefused::Mismatched("workload"))
    );
    let mut other_unit = base.clone();
    other_unit[5].unit = "ms".into();
    assert_eq!(
        compare_schedulers(&other_unit, "default", "lavd", 0.95),
        Err(CompareRefused::Mismatched("metric or unit"))
    );
    let mut unpaired = base.clone();
    unpaired[3].block_id = 9;
    assert!(matches!(
        compare_schedulers(&unpaired, "default", "lavd", 0.95),
        Err(CompareRefused::UnpairedBlock(_))
    ));
    let mut duplicate = base.clone();
    duplicate.push(run("lavd", 0, 9.0));
    assert_eq!(
        compare_schedulers(&duplicate, "default", "lavd", 0.95),
        Err(CompareRefused::DuplicateBlock(0))
    );
    assert_eq!(
        compare_schedulers(&set(&[10.0, 10.0], &[9.0, 9.0]), "default", "lavd", 0.95),
        Err(CompareRefused::TooFewBlocks(2))
    );
    assert_eq!(
        compare_schedulers(&base, "default", "rusty", 0.95),
        Err(CompareRefused::MissingScheduler("rusty".into()))
    );
    let mut bad = base;
    bad[0].value = f64::NAN;
    assert_eq!(
        compare_schedulers(&bad, "default", "lavd", 0.95),
        Err(CompareRefused::BadValue)
    );
}
