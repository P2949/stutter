use std::fs;

use serde_json::json;

use super::{Scratch, fresh, manifest};
use crate::{
    evidence_bundle::tests::{COMMIT, Dir, raw},
    validation_campaign::{
        evidence::{
            PipelineFailure, publish_bundle, published_capture_ids, rebuild_combined_index,
            record_failure, reverify_task,
        },
        model::{AttemptOutcome, TaskAttempt},
        state::CampaignDir,
        verify::verify_state,
    },
};

const KINDS: &[&str] = &["controlled_real", "real_workload"];

fn raw_for(suite: &str) -> Dir {
    raw(suite, "ok")
}

fn publish(
    dir: &CampaignDir,
    scratch: &Scratch,
    task: &str,
    attempt: u32,
    raw_dir: &Dir,
    commit: &str,
    kinds: &[&str],
) -> Result<crate::validation_campaign::evidence::Published, PipelineFailure> {
    publish_bundle(
        dir,
        scratch.path(),
        task,
        attempt,
        &raw_dir.0,
        commit,
        kinds,
    )
}

#[test]
fn a_passing_suite_with_a_verifying_bundle_is_published_with_receipt_and_index() {
    let scratch = Scratch::new("publish");
    let (dir, _, _) = fresh(&scratch);
    let raw = raw_for("live_recovery");
    let published = publish(&dir, &scratch, "rc-recovery", 1, &raw, COMMIT, KINDS).unwrap();
    assert!(published.bundle.join("MANIFEST.sha256").is_file());
    assert!(published.receipt.is_file());
    assert_eq!(published.index_entry["source_commit"], COMMIT);
    assert!(
        published_capture_ids(&dir)
            .unwrap()
            .contains(&published.capture_id)
    );
    assert_eq!(reverify_task(&dir, "rc-recovery").unwrap().len(), 1);
    let combined = rebuild_combined_index(&dir, scratch.path()).unwrap();
    let combined: serde_json::Value = serde_json::from_slice(&fs::read(combined).unwrap()).unwrap();
    assert!(
        combined["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["capture_id"] == json!(published.capture_id))
    );
}

#[test]
fn a_suite_that_did_not_pass_never_publishes() {
    let scratch = Scratch::new("suite-fail");
    let (dir, _, _) = fresh(&scratch);
    let raw = raw_for("live_recovery");
    let path = raw.join("result.json");
    let mut result: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    result["status"] = json!("FAIL");
    fs::write(&path, serde_json::to_vec(&result).unwrap()).unwrap();
    let failure = publish(&dir, &scratch, "rc-recovery", 1, &raw, COMMIT, KINDS).unwrap_err();
    assert_eq!(failure.stage, "suite_result");
    assert!(published_capture_ids(&dir).unwrap().is_empty());
}

#[test]
fn the_wrong_source_commit_or_evidence_kind_is_rejected_and_nothing_is_published() {
    let scratch = Scratch::new("reject");
    let (dir, _, _) = fresh(&scratch);
    let raw = raw_for("live_recovery");
    let other = "fedcba9876543210fedcba9876543210fedcba98";
    assert_eq!(
        publish(&dir, &scratch, "rc-recovery", 1, &raw, other, KINDS)
            .unwrap_err()
            .stage,
        "source_commit"
    );
    let raw = raw_for("live_recovery");
    assert_eq!(
        publish(
            &dir,
            &scratch,
            "rc-recovery",
            2,
            &raw,
            COMMIT,
            &["real_workload"]
        )
        .unwrap_err()
        .stage,
        "evidence_kind"
    );
    assert!(
        published_capture_ids(&dir).unwrap().is_empty(),
        "a rejected bundle is not verified evidence"
    );
}

#[test]
fn a_duplicate_capture_is_rejected() {
    let scratch = Scratch::new("duplicate");
    let (dir, _, _) = fresh(&scratch);
    let first = raw_for("live_recovery");
    publish(&dir, &scratch, "rc-recovery", 1, &first, COMMIT, KINDS).unwrap();
    let again = raw_for("live_recovery");
    let failure = publish(&dir, &scratch, "rc-recovery", 2, &again, COMMIT, KINDS).unwrap_err();
    assert_eq!(failure.stage, "duplicate");
}

#[test]
fn tampering_with_a_published_bundle_fails_reverification() {
    let scratch = Scratch::new("tamper");
    let (dir, _, _) = fresh(&scratch);
    let raw = raw_for("live_recovery");
    let published = publish(&dir, &scratch, "rc-recovery", 1, &raw, COMMIT, KINDS).unwrap();
    fs::write(published.bundle.join("artifacts/one/before.json"), "[]").unwrap();
    assert!(reverify_task(&dir, "rc-recovery").is_err());
}

#[test]
fn a_partial_bundle_never_appears_verified() {
    let scratch = Scratch::new("partial");
    let (dir, _, _) = fresh(&scratch);
    let partial = dir.bundles().join("rc-recovery/.partial-0003");
    fs::create_dir_all(&partial).unwrap();
    fs::write(partial.join("half-written"), "x").unwrap();
    assert!(published_capture_ids(&dir).unwrap().is_empty());
    assert!(reverify_task(&dir, "rc-recovery").unwrap().is_empty());
}

#[test]
fn failures_are_retained_with_a_concise_machine_readable_record() {
    let scratch = Scratch::new("failure-record");
    let (dir, _, _) = fresh(&scratch);
    let failure = PipelineFailure {
        stage: "evidence_verify".into(),
        summary: "privacy_scan failed".into(),
    };
    let path = record_failure(&dir, "live-observation", 2, &failure).unwrap();
    let body: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(body["stage"], "evidence_verify");
    assert_eq!(body["attempt"], 2);
    assert!(
        body["artifacts"]
            .as_str()
            .unwrap()
            .contains("live-observation")
    );
}

#[test]
fn strict_verification_rebuilds_truth_from_retained_records() {
    let scratch = Scratch::new("strict");
    let (dir, mut state, graph) = fresh(&scratch);
    let manifest = manifest();
    // Snapshot the manifest the state recorded, as `init` does.
    crate::validation_campaign::state::write_atomic(
        &dir.manifest_snapshot(),
        super::MANIFEST_TEXT.as_bytes(),
    )
    .unwrap();
    state.manifest_sha256 =
        crate::evidence_bundle::sha256::sha256_hex(super::MANIFEST_TEXT.as_bytes());
    let spec = graph.get("offline-ready").unwrap();
    let attempt = TaskAttempt {
        number: 1,
        started_unix_ms: 1,
        finished_unix_ms: 2,
        outcome: AttemptOutcome::Passed,
        fingerprint: state.expected_fingerprint(spec),
        artifacts: None,
        receipt: None,
        limitations: vec![],
    };
    state
        .record_attempt(&dir, "offline-ready", attempt)
        .unwrap();
    assert_eq!(
        verify_state(&dir, &state, &graph, &manifest),
        Vec::<String>::new()
    );
    // A cached "Passed" with no immutable attempt record is exposed.
    fs::remove_file(dir.attempt_dir("offline-ready", 1).join("attempt.json")).unwrap();
    let problems = verify_state(&dir, &state, &graph, &manifest);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("no readable immutable record")),
        "{problems:?}"
    );
    // A bundle-backed task claiming Passed without any verified bundle is exposed.
    let mut forged = state.clone();
    let observation = graph.get("live-observation").unwrap();
    let forged_attempt = TaskAttempt {
        number: 1,
        started_unix_ms: 1,
        finished_unix_ms: 2,
        outcome: AttemptOutcome::Passed,
        fingerprint: forged.expected_fingerprint(observation),
        artifacts: None,
        receipt: None,
        limitations: vec![],
    };
    forged
        .record_attempt(&dir, "live-observation", forged_attempt)
        .unwrap();
    let problems = verify_state(&dir, &forged, &graph, &manifest);
    assert!(
        problems.iter().any(|p| p.contains("no verified bundle")),
        "{problems:?}"
    );
}
