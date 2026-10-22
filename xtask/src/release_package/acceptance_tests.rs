use std::{fs, path::PathBuf};

use serde_json::{Value, json};
use stutter::api::capabilities::{Capability, CapabilityClass, CapabilityTier};

use super::acceptance::check_acceptance;
use crate::evidence_bundle::tests::{COMMIT, Dir, export, raw};

const OTHER_COMMIT: &str = "fedcba9876543210fedcba9876543210fedcba98";

fn supported(required_evidence: &'static [&'static str]) -> Capability {
    Capability {
        id: "test_capability",
        title: "Test capability",
        class: CapabilityClass::NonMutating,
        tier: CapabilityTier::Supported,
        target_tier: CapabilityTier::Supported,
        commands: &[],
        providers: &[],
        required_evidence,
        limitations: "test",
    }
}

/// An exported, verified bundle for `suite` (commit `COMMIT`, capture `capture`).
fn bundle(suite: &str, capture: &str, status: &str) -> (Dir, Dir, PathBuf) {
    let (raw_dir, out) = (raw(suite, "ok"), Dir::new("out"));
    for (file, extra) in [("environment.json", false), ("result.json", true)] {
        let path = raw_dir.join(file);
        let mut value: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value["capture_id"] = json!(capture);
        if extra {
            value["status"] = json!(status);
            let case = if status == "PASS" { "PASS" } else { "FAIL" };
            value["cases"] = json!([{"checkpoint":"one","status":case}]);
            if status != "PASS" {
                value["error"] = json!("injected");
            }
        }
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
    let path = export(&raw_dir, &out).unwrap();
    (raw_dir, out, path)
}

fn two_suites() -> (Vec<Capability>, Vec<(Dir, Dir, PathBuf)>) {
    (
        vec![supported(&["live_recovery", "live_affinity"])],
        vec![
            bundle("live_recovery", "recovery-a", "PASS"),
            bundle("live_affinity", "affinity-a", "PASS"),
        ],
    )
}

fn paths(bundles: &[(Dir, Dir, PathBuf)]) -> Vec<PathBuf> {
    bundles.iter().map(|(_, _, path)| path.clone()).collect()
}

#[test]
fn valid_exact_commit_bundles_satisfy_every_required_suite() {
    let (capabilities, bundles) = two_suites();
    let accepted = check_acceptance(&capabilities, COMMIT, &paths(&bundles)).unwrap();
    assert_eq!(
        accepted.into_iter().collect::<Vec<_>>(),
        ["live_affinity", "live_recovery"]
    );
}

#[test]
fn a_bundle_for_another_commit_fails() {
    let (capabilities, bundles) = two_suites();
    let error = check_acceptance(&capabilities, OTHER_COMMIT, &paths(&bundles))
        .unwrap_err()
        .to_string();
    assert!(error.contains("not release commit"), "{error}");
}

#[test]
fn a_tampered_bundle_fails_even_when_its_receipt_says_pass() {
    let (capabilities, bundles) = two_suites();
    fs::write(bundles[0].2.join("artifacts/one/before.json"), "[]").unwrap();
    let error = format!(
        "{:#}",
        check_acceptance(&capabilities, COMMIT, &paths(&bundles)).unwrap_err()
    );
    assert!(error.contains("does not verify"), "{error}");
}

#[test]
fn a_missing_required_suite_fails() {
    let (capabilities, bundles) = two_suites();
    let only_recovery = &paths(&bundles)[..1];
    let error = check_acceptance(&capabilities, COMMIT, only_recovery)
        .unwrap_err()
        .to_string();
    assert!(error.contains("live_affinity"), "{error}");
}

#[test]
fn the_committed_historical_index_alone_is_insufficient() {
    let (capabilities, _) = two_suites();
    let error = check_acceptance(&capabilities, COMMIT, &[])
        .unwrap_err()
        .to_string();
    assert!(error.contains("historical"), "{error}");
    // And nothing is releasable as Supported without a Supported capability.
    let none = [Capability {
        tier: CapabilityTier::Experimental,
        ..supported(&[])
    }];
    assert!(check_acceptance(&none, COMMIT, &[]).is_err());
}

#[test]
fn a_non_pass_bundle_fails_in_either_order() {
    let capabilities = vec![supported(&["live_recovery"])];
    let good = bundle("live_recovery", "recovery-good", "PASS");
    let bad = bundle("live_recovery", "recovery-bad", "FAIL");
    for order in [[&good.2, &bad.2], [&bad.2, &good.2]] {
        let error = check_acceptance(
            &capabilities,
            COMMIT,
            &order.iter().map(|p| (*p).clone()).collect::<Vec<_>>(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("not PASS"), "{error}");
    }
}

#[test]
fn duplicate_suites_are_deterministic() {
    let capabilities = vec![supported(&["live_recovery"])];
    let first = bundle("live_recovery", "recovery-one", "PASS");
    let second = bundle("live_recovery", "recovery-two", "PASS");
    // Two distinct captures of one suite satisfy it, whichever order they are given in.
    let forward = check_acceptance(&capabilities, COMMIT, &[first.2.clone(), second.2.clone()]);
    let reverse = check_acceptance(&capabilities, COMMIT, &[second.2.clone(), first.2.clone()]);
    assert_eq!(forward.unwrap(), reverse.unwrap());
    // The same capture supplied twice is rejected rather than counted as two.
    let error = check_acceptance(&capabilities, COMMIT, &[first.2.clone(), first.2.clone()])
        .unwrap_err()
        .to_string();
    assert!(error.contains("supplied twice"), "{error}");
}
