use super::*;

const NOW: u64 = 9_000_000;

fn good() -> PressureControlReceipt {
    PressureControlReceipt {
        schema: 1,
        binary_sha256: "aa".repeat(32),
        captured_unix: NOW - 10,
        load_owned_by_campaign: true,
        scoped_events: 5_000,
        baseline_runnable_p95_ms: 2.0,
        loaded_runnable_p95_ms: 9.0,
        action_applied: true,
        restored_independently: true,
    }
}

#[test]
fn only_a_fresh_owned_observed_applied_and_restored_control_for_this_binary_attests() {
    let me = "aa".repeat(32);
    assert_eq!(attested(&good(), &me, NOW), Ok(()));
    let refused = |edit: &dyn Fn(&mut PressureControlReceipt), why: &'static str| {
        let mut bad = good();
        edit(&mut bad);
        assert_eq!(attested(&bad, &me, NOW), Err(why));
    };
    refused(&|r| r.schema = 2, "PressureControlSchemaUnsupported");
    refused(
        &|r| r.binary_sha256 = "bb".repeat(32),
        "PressureControlForAnotherBinary",
    );
    refused(&|r| r.captured_unix = 1, "PressureControlNotFresh");
    refused(
        &|r| r.load_owned_by_campaign = false,
        "PressureNotOwnedByCampaign",
    );
    refused(&|r| r.scoped_events = 999, "PressureControlTooFewEvents");
    refused(&|r| r.loaded_runnable_p95_ms = 2.0, "PressureNotObserved");
    refused(
        &|r| r.baseline_runnable_p95_ms = f64::NAN,
        "PressureNotObserved",
    );
    refused(
        &|r| r.action_applied = false,
        "PressureControlActionNotApplied",
    );
    refused(
        &|r| r.restored_independently = false,
        "PressureControlNotRestored",
    );
}

#[test]
fn a_directory_with_no_raw_record_has_no_pressure_control() {
    let me = "aa".repeat(32);
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(status(dir.path(), &me, NOW), Err("NoPressureControl"));
}
