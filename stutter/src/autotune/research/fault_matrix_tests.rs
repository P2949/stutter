use super::*;

const NOW: u64 = 5_000_000;
const ME: &str = "aa";

fn me() -> String {
    ME.repeat(32)
}

fn physical(class: &str) -> FaultCase {
    FaultCase {
        class: class.to_owned(),
        mode: Mode::Physical,
        outcome: Outcome::Passed,
        evidence_sha256: "ab".repeat(32),
        observed_unix: NOW - 200,
    }
}

fn complete() -> FaultMatrixReceipt {
    FaultMatrixReceipt {
        schema: 1,
        binary_sha256: me(),
        captured_unix: NOW - 100,
        cases: REQUIRED_CLASSES
            .iter()
            .map(|c| {
                let mut case = physical(c);
                if DESTRUCTIVE_CLASSES.contains(c) {
                    case.mode = Mode::Fixture;
                }
                case
            })
            .collect(),
    }
}

#[test]
fn a_complete_matrix_for_this_binary_attests() {
    assert_eq!(attested(&complete(), &me(), NOW), Ok(()));
}

#[test]
fn fixtures_never_count_and_every_other_gap_has_its_own_refusal() {
    let refused = |edit: &dyn Fn(&mut FaultMatrixReceipt), expected: &'static str| {
        let mut bad = complete();
        edit(&mut bad);
        assert_eq!(attested(&bad, &me(), NOW), Err(expected));
    };
    refused(&|r| r.schema = 2, "FaultMatrixSchemaUnsupported");
    refused(
        &|r| r.binary_sha256 = "bb".repeat(32),
        "FaultMatrixForAnotherBinary",
    );
    refused(
        &|r| r.captured_unix = NOW - MAX_AGE_SECONDS - 1,
        "FaultMatrixNotFresh",
    );
    refused(&|r| r.captured_unix = NOW + 3600, "FaultMatrixNotFresh");
    // A class proven only by a simulated fault is not a physical pass.
    refused(
        &|r| r.cases[0].mode = Mode::Fixture,
        "FaultClassOnlyFixtureTested",
    );
    // A missing class, and a class run twice (the second could be hiding the first).
    refused(&|r| drop(r.cases.pop()), "FaultClassMissingOrDuplicated");
    refused(
        &|r| {
            let again = r.cases[2].clone();
            r.cases.push(again);
        },
        "FaultClassMissingOrDuplicated",
    );
    refused(&|r| r.cases.clear(), "FaultClassMissingOrDuplicated");
    // A failed case blocks even when every required class has a passing one.
    refused(
        &|r| {
            let mut failed = physical("extra_probe");
            failed.outcome = Outcome::Failed;
            r.cases.push(failed);
        },
        "FaultCaseFailed",
    );
    refused(
        &|r| r.cases[3].evidence_sha256 = "not-a-digest".into(),
        "FaultCaseWithoutEvidence",
    );
    // A case that claims to have been observed after the receipt was taken.
    refused(&|r| r.cases[1].observed_unix = NOW, "FaultCaseAfterReceipt");
    // Another binary's matrix says nothing about the one verifying.
    assert_eq!(
        attested(&complete(), &"cc".repeat(32), NOW),
        Err("FaultMatrixForAnotherBinary")
    );
}

#[test]
fn destructive_faults_are_proven_by_fixtures_and_never_by_injecting_them_into_the_real_game() {
    // The complete matrix already holds fixtures for exactly the destructive classes.
    let matrix = complete();
    assert!(
        matrix
            .cases
            .iter()
            .filter(|case| DESTRUCTIVE_CLASSES.contains(&case.class.as_str()))
            .all(|case| case.mode == Mode::Fixture)
    );
    assert_eq!(attested(&matrix, &me(), NOW), Ok(()));
    // Injecting one for real is refused rather than counted, whatever it showed.
    let mut injected = complete();
    let index = injected
        .cases
        .iter()
        .position(|case| case.class == "gpu_health_halt_settles")
        .unwrap();
    injected.cases[index].mode = Mode::Physical;
    assert_eq!(
        attested(&injected, &me(), NOW),
        Err("DestructiveFaultInjectedPhysically")
    );
}

#[test]
fn the_gate_reads_the_newest_receipt_and_without_one_says_so() {
    use std::os::unix::fs::PermissionsExt;
    let study = tempfile::tempdir().unwrap();
    assert_eq!(
        gate_status(study.path(), &me(), NOW),
        Err("NoFaultMatrixReceipt")
    );
    let write = |name: &str, receipt: &FaultMatrixReceipt| {
        let mut receipt = receipt.clone();
        seal_evidence(study.path(), &mut receipt);
        let receipt = &receipt;
        let path = study.path().join(name);
        std::fs::write(&path, serde_json::to_vec(receipt).unwrap()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    };
    write("fault-matrix-1.json", &complete());
    assert_eq!(gate_status(study.path(), &me(), NOW), Ok(()));
    // A newer receipt withdraws it when it no longer attests.
    let mut newer = complete();
    newer.captured_unix = NOW - 10;
    newer.cases[0].mode = Mode::Fixture;
    write("fault-matrix-2.json", &newer);
    assert_eq!(
        gate_status(study.path(), &me(), NOW),
        Err("FaultClassOnlyFixtureTested")
    );
    // A present file that is not a receipt is corrupt evidence: it is named, never skipped for an older pass.
    std::fs::write(study.path().join("fault-matrix-3.json"), b"{").unwrap();
    assert_eq!(
        gate_status(study.path(), &me(), NOW),
        Err("FaultMatrixInvalid")
    );
}

/// Write each case's raw record beside the receipt and point the case at it by its real digest.
fn seal_evidence(study: &Path, receipt: &mut FaultMatrixReceipt) {
    let dir = study.join(EVIDENCE_DIR);
    std::fs::create_dir_all(&dir).unwrap();
    for case in &mut receipt.cases {
        let record = serde_json::json!({
            "class": case.class,
            "mode": if case.mode == Mode::Fixture { "fixture" } else { "physical" },
            "outcome": if case.outcome == Outcome::Passed { "passed" } else { "failed" },
            "binary_sha256": receipt.binary_sha256,
        });
        let bytes = serde_json::to_vec(&record).unwrap();
        case.evidence_sha256 = stutter_core::hash::sha256_hex(&bytes);
        std::fs::write(dir.join(format!("{}.json", case.evidence_sha256)), bytes).unwrap();
    }
}

#[test]
fn every_case_must_name_a_real_unmodified_correctly_classified_unshared_record() {
    let study = tempfile::tempdir().unwrap();
    let mut receipt = complete();
    seal_evidence(study.path(), &mut receipt);
    assert_eq!(artifacts_verified(&receipt, study.path()), Ok(()));
    let file = |case: &FaultCase| {
        study
            .path()
            .join(EVIDENCE_DIR)
            .join(format!("{}.json", case.evidence_sha256))
    };

    // Modified after the digest was recorded.
    std::fs::write(file(&receipt.cases[0]), b"{}").unwrap();
    assert_eq!(
        artifacts_verified(&receipt, study.path()),
        Err("FaultEvidenceModified")
    );
    // Missing.
    std::fs::remove_file(file(&receipt.cases[0])).unwrap();
    assert_eq!(
        artifacts_verified(&receipt, study.path()),
        Err("FaultEvidenceMissing")
    );
    // A record that describes another class, however well it hashes.
    let mut wrong = complete();
    seal_evidence(study.path(), &mut wrong);
    let (first, second) = (wrong.cases[0].clone(), wrong.cases[1].clone());
    wrong.cases[0].evidence_sha256 = second.evidence_sha256.clone();
    wrong.cases[1].evidence_sha256 = first.evidence_sha256.clone();
    assert_eq!(
        artifacts_verified(&wrong, study.path()),
        Err("FaultEvidenceMisclassified")
    );
    // One record standing for two cases.
    let mut shared = complete();
    seal_evidence(study.path(), &mut shared);
    shared.cases[1].evidence_sha256 = shared.cases[0].evidence_sha256.clone();
    assert_eq!(
        artifacts_verified(&shared, study.path()),
        Err("FaultEvidenceShared")
    );
    // A link in place of the record.
    let mut linked = complete();
    seal_evidence(study.path(), &mut linked);
    let target = file(&linked.cases[2]);
    let moved = study.path().join("moved");
    std::fs::rename(&target, &moved).unwrap();
    std::os::unix::fs::symlink(&moved, &target).unwrap();
    assert_eq!(
        artifacts_verified(&linked, study.path()),
        Err("FaultEvidenceUnusable")
    );
}

/// Every destructive class is proven by a named fixture, and that fixture exists: a mapping to a renamed or removed test
/// would leave a class with no proof while the receipt still named one.
#[test]
fn every_destructive_class_names_a_fixture_test_that_exists() {
    let classes: Vec<&str> = FIXTURE_TESTS.iter().map(|(class, _)| *class).collect();
    let mut expected: Vec<&str> = DESTRUCTIVE_CLASSES.to_vec();
    expected.sort_unstable();
    let mut named = classes.clone();
    named.sort_unstable();
    assert_eq!(named, expected, "exactly the destructive classes");
    // Every named test is defined, once, in the crate's own sources.
    fn sources(dir: &std::path::Path, out: &mut String) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push_str(&std::fs::read_to_string(&path).unwrap());
            }
        }
    }
    let mut all = String::new();
    sources(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut all,
    );
    for (class, test) in FIXTURE_TESTS {
        let defined = all.matches(&format!("fn {test}(")).count();
        assert_eq!(defined, 1, "{class}: `{test}` must be defined exactly once");
    }
}
