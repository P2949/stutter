use std::fs;

use serde_json::json;

use super::{Scratch, fresh, manifest};
use crate::{
    evidence_bundle::tests::{COMMIT, Dir, raw},
    validation_campaign::{
        evidence::{
            Accept, PipelineFailure, PublishRequest, publish_bundle, published_capture_ids,
            rebuild_combined_index, record_failure, reverify_task,
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
    publish_bundle(&PublishRequest {
        dir,
        repository: scratch.path(),
        task,
        attempt,
        raw: &raw_dir.0,
        expected_commit: commit,
        allowed_kinds: kinds,
        accept: Accept::PassOnly,
    })
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

/// A raw `live_kernel_compat` directory for `kernel` with its own capture id.
fn kernel_raw(kernel: &str, capture: &str, status: &str, compatibility: &str) -> Dir {
    let raw_dir = raw("live_kernel_compat", "ok");
    for (file, is_result) in [("environment.json", false), ("result.json", true)] {
        let path = raw_dir.join(file);
        let mut value: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        value["capture_id"] = json!(capture);
        if is_result {
            value["status"] = json!(status);
            value["compatibility"] = json!(compatibility);
            let case = if status == "PASS" { "PASS" } else { "FAIL" };
            value["cases"] = json!([{"checkpoint":"one","status":case}]);
            if status != "PASS" {
                value["error"] = json!("observed incompatible");
            }
        } else {
            value["kernel_release"] = json!(kernel);
        }
        fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
    raw_dir
}

fn publish_with(
    dir: &CampaignDir,
    scratch: &Scratch,
    raw_dir: &Dir,
    attempt: u32,
    accept: Accept,
) -> Result<crate::validation_campaign::evidence::Published, PipelineFailure> {
    publish_bundle(&PublishRequest {
        dir,
        repository: scratch.path(),
        task: "kernel-matrix",
        attempt,
        raw: &raw_dir.0,
        expected_commit: COMMIT,
        allowed_kinds: KINDS,
        accept,
    })
}

#[test]
fn an_observed_incompatible_kernel_is_a_valid_observation_only_under_the_kernel_policy() {
    let scratch = Scratch::new("kernel-incompatible");
    let (dir, _, _) = fresh(&scratch);
    let incompatible = kernel_raw("6.1.0", "cap-bad", "FAIL", "ObservedIncompatible");
    let rejected = publish_with(&dir, &scratch, &incompatible, 1, Accept::PassOnly).unwrap_err();
    assert_eq!(
        rejected.stage, "suite_result",
        "an ordinary suite FAIL is never evidence"
    );
    let published = publish_with(
        &dir,
        &scratch,
        &incompatible,
        2,
        Accept::PassOrObservedIncompatible,
    )
    .unwrap();
    assert_eq!(published.index_entry["status"], "FAIL");
    assert_eq!(
        published.index_entry["compatibility"],
        "ObservedIncompatible"
    );
    assert_eq!(published.index_entry["kernel_release"], "6.1.0");
    // A FAIL that is not an incompatibility observation is rejected even under that policy.
    let other = kernel_raw("6.2.0", "cap-other", "FAIL", "ObservedCompatible");
    assert!(
        publish_with(
            &dir,
            &scratch,
            &other,
            3,
            Accept::PassOrObservedIncompatible
        )
        .is_err()
    );
}

#[test]
fn strict_verification_requires_the_distinct_kernel_count_for_the_kernel_stage() {
    use crate::validation_campaign::model::{AttemptOutcome, TaskAttempt};
    let scratch = Scratch::new("kernel-strict");
    let (dir, mut state, graph) = fresh(&scratch);
    let manifest = manifest();
    crate::validation_campaign::state::write_atomic(
        &dir.manifest_snapshot(),
        super::MANIFEST_TEXT.as_bytes(),
    )
    .unwrap();
    state.manifest_sha256 =
        crate::evidence_bundle::sha256::sha256_hex(super::MANIFEST_TEXT.as_bytes());
    state.evidence_base = COMMIT.to_owned();
    let spec = graph.get("kernel-matrix").unwrap();
    let pass = TaskAttempt {
        number: 1,
        started_unix_ms: 1,
        finished_unix_ms: 2,
        outcome: AttemptOutcome::Passed,
        fingerprint: state.expected_fingerprint(spec),
        artifacts: None,
        receipt: None,
        limitations: vec![],
    };
    state.record_attempt(&dir, "kernel-matrix", pass).unwrap();
    for (n, (kernel, status, compat)) in [
        ("6.18.55", "PASS", "ObservedCompatible"),
        ("6.12.4", "PASS", "ObservedDegraded"),
    ]
    .into_iter()
    .enumerate()
    {
        let raw_dir = kernel_raw(kernel, &format!("cap-{n}"), status, compat);
        publish_with(
            &dir,
            &scratch,
            &raw_dir,
            n as u32 + 1,
            Accept::PassOrObservedIncompatible,
        )
        .unwrap();
    }
    let problems = verify_state(&dir, &state, &graph, &manifest);
    assert!(
        problems
            .iter()
            .any(|p| p.contains("2 distinct kernels; 3 are required")),
        "{problems:?}"
    );
    // The third kernel is observed as incompatible: still a distinct, kept observation.
    let third = kernel_raw("6.1.0", "cap-2", "FAIL", "ObservedIncompatible");
    publish_with(
        &dir,
        &scratch,
        &third,
        3,
        Accept::PassOrObservedIncompatible,
    )
    .unwrap();
    let problems = verify_state(&dir, &state, &graph, &manifest);
    assert!(
        !problems.iter().any(|p| p.contains("distinct kernels")),
        "{problems:?}"
    );
    assert!(
        !problems.iter().any(|p| p.contains("does not certify")),
        "{problems:?}"
    );
}
