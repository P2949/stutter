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
        cases: REQUIRED_CLASSES.iter().map(|c| physical(c)).collect(),
    }
}

#[test]
fn a_complete_physical_matrix_for_this_binary_attests() {
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
fn the_gate_reads_the_newest_receipt_and_without_one_says_so() {
    use std::os::unix::fs::PermissionsExt;
    let study = tempfile::tempdir().unwrap();
    assert_eq!(
        gate_status(study.path(), &me(), NOW),
        Err("NoFaultMatrixReceipt")
    );
    let write = |name: &str, receipt: &FaultMatrixReceipt| {
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
    // A malformed file is not a receipt.
    std::fs::write(study.path().join("fault-matrix-3.json"), b"{").unwrap();
    assert_eq!(
        gate_status(study.path(), &me(), NOW),
        Err("FaultClassOnlyFixtureTested")
    );
}
