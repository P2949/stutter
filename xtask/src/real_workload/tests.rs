use super::{
    inventory::{collect, manifest_name, steam_titles},
    manifest::*,
    outcome::*,
};

fn metric(name: &str, direction: Direction) -> Metric {
    Metric {
        name: name.to_owned(),
        unit: "ms".to_owned(),
        direction,
    }
}

fn manifest() -> Manifest {
    Manifest {
        schema_version: 1,
        evidence_kind: EvidenceKind::ControlledReal,
        source_commit: "a".repeat(40),
        binary_sha256: "b".repeat(64),
        workload: "vkcube".into(),
        launch_mode: LaunchMode::Native,
        work_unit: "fixed 60 s scene".into(),
        command: vec!["vkcube".into()],
        time_boxed: true,
        instrumentation: vec!["mangohud".into()],
        context: Default::default(),
        warmup_seconds: 5,
        measure_seconds: 30,
        cooldown_seconds: 5,
        timeout_seconds: 120,
        max_pairs: 8,
        candidate: "cpu_affinity_profile".into(),
        primary: metric("runnable_p99_ms", Direction::LowerIsBetter),
        min_effect_percent: 10.0,
        equivalence_margin_percent: 3.0,
        guardrails: vec![Guardrail {
            metric: metric("frame_p99_ms", Direction::LowerIsBetter),
            max_regression_percent: 5.0,
        }],
        abort_rules: vec!["amdgpu reset or hang".into()],
    }
}

fn facts() -> RunFacts {
    RunFacts {
        engineering_correct: true,
        ..RunFacts::default()
    }
}

fn pair(base: f64, treat: f64, guard_base: f64, guard_treat: f64) -> PairResult {
    PairResult {
        baseline_primary: base,
        treatment_primary: treat,
        baseline_guardrails: vec![guard_base],
        treatment_guardrails: vec![guard_treat],
    }
}

fn pairs(count: usize, base: f64, treat: f64, guard_treat: f64) -> Vec<PairResult> {
    // A little deterministic spread so the interval is not degenerate.
    (0..count)
        .map(|index| {
            let jitter = 1.0 + (index as f64 % 3.0 - 1.0) * 0.01;
            pair(base * jitter, treat * jitter, 10.0, guard_treat)
        })
        .collect()
}

#[test]
fn the_manifest_is_validated_before_anything_runs() {
    assert!(manifest().validate().is_ok());
    let mut bad = manifest();
    bad.max_pairs = 100;
    assert!(bad.validate().is_err(), "trials are capped");
    let mut bad = manifest();
    bad.min_effect_percent = 2.0;
    assert!(
        bad.validate().is_err(),
        "effect must exceed the equivalence margin"
    );
    let mut bad = manifest();
    bad.guardrails.clear();
    assert!(bad.validate().is_err());
    let mut bad = manifest();
    bad.binary_sha256 = "zz".into();
    assert!(bad.validate().is_err());
    let mut bad = manifest();
    bad.timeout_seconds = 10;
    assert!(bad.validate().is_err());
    let mut changed = manifest();
    changed.min_effect_percent = 9.0;
    assert_ne!(
        manifest().frozen_digest().unwrap(),
        changed.frozen_digest().unwrap()
    );
}

#[test]
fn arms_are_counterbalanced() {
    let manifest = manifest();
    let order: Vec<Arm> = (0..8).map(|slot| manifest.arm_at(slot)).collect();
    use Arm::{Baseline as A, Treatment as B};
    assert_eq!(order, vec![A, B, B, A, A, B, B, A]);
}

#[test]
fn a_clear_gain_with_held_guardrails_is_a_performance_pass() {
    let verdict = classify(&manifest(), &facts(), &pairs(8, 10.0, 7.0, 10.1));
    assert_eq!(verdict.outcome, Outcome::PerformancePass, "{verdict:?}");
    assert!(verdict.engineering_pass);
    assert!(verdict.ci_low >= 10.0);
}

#[test]
fn a_guardrail_regression_blocks_a_gain() {
    let verdict = classify(&manifest(), &facts(), &pairs(8, 10.0, 7.0, 12.0));
    assert_eq!(verdict.outcome, Outcome::NoBenefit);
    assert!(verdict.reason.contains("frame_p99_ms"));
}

#[test]
fn no_change_is_equivalent_and_never_repeated_into_a_pass() {
    let verdict = classify(&manifest(), &facts(), &pairs(8, 10.0, 10.0, 10.0));
    assert_eq!(verdict.outcome, Outcome::NoBenefit);
    assert!(
        verdict.reason.starts_with("equivalent"),
        "{}",
        verdict.reason
    );
}

#[test]
fn a_noisy_small_effect_is_inconclusive_not_a_pass() {
    let noisy: Vec<PairResult> = [5.0, 30.0, -8.0, 22.0, 2.0, 25.0]
        .iter()
        .map(|gain| pair(100.0, 100.0 - gain, 10.0, 10.0))
        .collect();
    assert_eq!(
        classify(&manifest(), &facts(), &noisy).outcome,
        Outcome::Inconclusive
    );
}

#[test]
fn faults_blocks_and_thin_data_come_first() {
    let mut safety = facts();
    safety.safety_fault = Some("amdgpu reset".into());
    assert_eq!(
        classify(&manifest(), &safety, &pairs(8, 10.0, 5.0, 10.0)).outcome,
        Outcome::SafetyAbort
    );
    let mut blocked = facts();
    blocked.blocked = Some("no benchmark".into());
    assert_eq!(
        classify(&manifest(), &blocked, &[]).outcome,
        Outcome::BlockedResource
    );
    assert_eq!(
        classify(&manifest(), &facts(), &pairs(3, 10.0, 5.0, 10.0)).outcome,
        Outcome::Inconclusive
    );
    let damaged = RunFacts {
        engineering_correct: false,
        ..RunFacts::default()
    };
    let verdict = classify(&manifest(), &damaged, &pairs(8, 10.0, 5.0, 10.0));
    assert_eq!(verdict.outcome, Outcome::Inconclusive);
    assert!(!verdict.engineering_pass);
    let invalid = vec![pair(0.0, 1.0, 1.0, 1.0); 5];
    assert_eq!(
        classify(&manifest(), &facts(), &invalid).outcome,
        Outcome::Inconclusive
    );
}

#[test]
fn summaries_report_percentiles_and_refuse_bad_data() {
    let summary = summarize(&(1..=100).map(f64::from).collect::<Vec<_>>()).unwrap();
    assert_eq!(
        (summary.n, summary.p50, summary.p95, summary.p99),
        (100, 50.0, 95.0, 99.0)
    );
    assert!(summarize(&[]).is_none());
    assert!(summarize(&[1.0, f64::NAN]).is_none());
}

#[test]
fn an_installed_game_without_a_repeatable_benchmark_is_a_blocked_resource() {
    let dir = std::env::temp_dir().join(format!("stutter-inventory-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("appmanifest_1.acf"),
        "\"AppState\"\n{\n\t\"appid\"\t\"1\"\n\t\"name\"\t\t\"Some Game\"\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("appmanifest_2.acf"),
        "\"AppState\"\n{\n\t\"name\"\t\t\"Proton Experimental\"\n}\n",
    )
    .unwrap();
    assert_eq!(steam_titles(&dir), vec!["Some Game".to_owned()]);
    assert!(
        collect(&dir, &[])
            .real_game_disposition
            .starts_with("BlockedResource")
    );
    let allowed = collect(&dir, &["Some Game".to_owned()]);
    assert_eq!(allowed.qualifying_titles, vec!["Some Game".to_owned()]);
    assert!(allowed.real_game_disposition.starts_with("eligible"));
    assert_eq!(
        manifest_name("\t\"name\"\t\t\"X Y\""),
        Some("X Y".to_owned())
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
