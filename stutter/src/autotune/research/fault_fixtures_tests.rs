use super::*;
use crate::autotune::research::fault_matrix::{
    DESTRUCTIVE_CLASSES, FaultMatrixReceipt, REQUIRED_CLASSES, artifacts_verified, attested,
};

const NOW: u64 = 5_000_000;

fn study() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("fault-fixtures-{}", rand_suffix()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn rand_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    format!(
        "{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

fn pass(_: &str) -> Option<FixtureRun> {
    Some(FixtureRun {
        passed: true,
        output: "test ... ok".into(),
    })
}

/// A receipt around the produced fixture cases and a physical case for each benign class (their evidence written as an
/// operator's capture would be).
fn receipt_with(study: &std::path::Path, fixtures: Vec<FaultCase>) -> FaultMatrixReceipt {
    let me = "aa".repeat(32);
    let mut cases = fixtures;
    for class in REQUIRED_CLASSES
        .iter()
        .filter(|class| !DESTRUCTIVE_CLASSES.contains(class))
    {
        let record =
            json!({ "class": class, "mode": "physical", "outcome": "passed", "binary_sha256": me });
        let bytes = serde_json::to_vec(&record).unwrap();
        let digest = stutter_core::hash::sha256_hex(&bytes);
        std::fs::write(
            study.join(EVIDENCE_DIR).join(format!("{digest}.json")),
            bytes,
        )
        .unwrap();
        cases.push(FaultCase {
            class: (*class).to_owned(),
            mode: Mode::Physical,
            outcome: Outcome::Passed,
            evidence_sha256: digest,
            observed_unix: NOW,
        });
    }
    FaultMatrixReceipt {
        schema: 1,
        binary_sha256: me,
        captured_unix: NOW,
        cases,
    }
}

#[test]
fn passing_fixtures_make_cases_the_matrix_verifier_accepts() {
    let dir = study();
    let cases = produce(&dir, &"aa".repeat(32), NOW, pass).unwrap();
    assert_eq!(cases.len(), FIXTURE_TESTS.len());
    let receipt = receipt_with(&dir, cases);
    assert_eq!(attested(&receipt, &"aa".repeat(32), NOW), Ok(()));
    assert_eq!(artifacts_verified(&receipt, &dir), Ok(()));
    // Producing again is idempotent: identical evidence is not an error.
    assert!(produce(&dir, &"aa".repeat(32), NOW, pass).is_ok());
}

#[test]
fn a_failing_or_missing_fixture_is_a_failed_case_and_blocks_the_matrix() {
    for runner in [
        (|_: &str| {
            Some(FixtureRun {
                passed: false,
                output: "test ... FAILED".into(),
            })
        }) as fn(&str) -> Option<FixtureRun>,
        |_: &str| None,
    ] {
        let dir = study();
        let cases = produce(&dir, &"aa".repeat(32), NOW, runner).unwrap();
        assert!(cases.iter().all(|case| case.outcome == Outcome::Failed));
        let receipt = receipt_with(&dir, cases);
        assert_eq!(
            attested(&receipt, &"aa".repeat(32), NOW),
            Err("FaultCaseFailed")
        );
    }
}

#[test]
fn evidence_for_another_binary_is_refused_by_the_verifier() {
    let dir = study();
    let cases = produce(&dir, &"bb".repeat(32), NOW, pass).unwrap();
    let receipt = receipt_with(&dir, cases);
    assert_eq!(
        artifacts_verified(&receipt, &dir),
        Err("FaultEvidenceMisclassified")
    );
}

#[test]
fn a_test_log_gives_a_pass_only_for_a_fixture_whose_own_line_says_ok() {
    let log = "running 3 tests\n\
        test autotune::a::tests::a_worker_that_dies_after_the_prepare_is_an_error_with_the_token_already_journaled ... ok\n\
        test autotune::b::tests::a_hook_recovers_every_arm_that_a_crash_left_edited ... FAILED\n";
    let mut run = runner_from_log(log);
    assert!(run(FIXTURE_TESTS[0].1).unwrap().passed);
    assert!(!run(FIXTURE_TESTS[1].1).unwrap().passed);
    assert!(run(FIXTURE_TESTS[2].1).is_none());
}

/// The physical cases of `receipt_with`, split off from its fixture cases.
fn physical_of(receipt: &FaultMatrixReceipt) -> Vec<FaultCase> {
    receipt
        .cases
        .iter()
        .filter(|case| case.mode == Mode::Physical)
        .cloned()
        .collect()
}

#[test]
fn the_assembled_receipt_is_checked_then_written_and_gate_g6_reads_it_back() {
    let dir = study();
    let me = "aa".repeat(32);
    let fixtures = produce(&dir, &me, NOW, pass).unwrap();
    let physical = physical_of(&receipt_with(&dir, Vec::new()));
    let receipt = assemble(&dir, &me, NOW, fixtures, physical).unwrap();
    assert_eq!(receipt.cases.len(), REQUIRED_CLASSES.len());
    assert_eq!(
        crate::autotune::research::fault_matrix::gate_status(&dir, &me, NOW),
        Ok(())
    );
}

#[test]
fn a_matrix_that_would_not_attest_writes_nothing() {
    let dir = study();
    let me = "aa".repeat(32);
    // A failing fixture, or a missing physical case, never becomes a receipt.
    let failed = produce(&dir, &me, NOW, |_| None).unwrap();
    let physical = physical_of(&receipt_with(&dir, Vec::new()));
    assert!(assemble(&dir, &me, NOW, failed, physical.clone()).is_err());
    let fixtures = produce(&dir, &me, NOW, pass).unwrap();
    assert!(assemble(&dir, &me, NOW, fixtures, physical[1..].to_vec()).is_err());
    assert_eq!(
        crate::autotune::research::fault_matrix::gate_status(&dir, &me, NOW),
        Err("NoFaultMatrixReceipt")
    );
}
