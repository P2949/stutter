use serde_json::json;

use super::*;

const NAMES: [&str; 11] = [
    "campaign-summary.md",
    "report.json",
    "capability-matrix.json",
    "proton-comparison.json",
    "diagnosis-oracle.json",
    "action-transactions.json",
    "excluded-and-blocked.json",
    "source-integrity.json",
    "restore-proof.json",
    "verify-result.json",
    "manifest-and-hashes.txt",
];

fn response() -> Value {
    json!({
        "intent_sha256": "ab",
        "verdict": {
            "reconstruction_passed": true, "raw_files_verified": true,
            "loaded_runtime_qualified": false, "child_studies_qualified": false,
            "child_reports_verified": true, "restoration_verified": false,
            "final_gate_qualified": false, "disposition": "PartiallyQualified"
        },
        "raw_native_files": { "status": "replayed" },
        "child_studies": [
            { "study_sha256": "s1", "profile": "cpu", "stage": {"stage": "screen"}, "result": "qualified", "reason": "" },
            { "study_sha256": "s2", "profile": "long", "stage": {"stage": "action"}, "result": "blocked", "reason": "BlockedActionChild" },
            { "study_sha256": "s3", "profile": "gpu", "stage": {"stage": "screen"}, "result": null, "reason": "" }
        ]
    })
}

#[test]
fn every_named_artifact_is_produced_with_the_manifest_last_and_binding_the_rest() {
    let files = build(&json!({"attempts_completed": 4}), &response());
    let names: Vec<_> = files.iter().map(|(name, _)| *name).collect();
    assert_eq!(names.len(), NAMES.len());
    for name in NAMES {
        assert!(names.contains(&name), "{name}");
    }
    assert_eq!(names.last(), Some(&"manifest-and-hashes.txt"));
    let manifest = String::from_utf8(files.last().unwrap().1.clone()).unwrap();
    for (name, bytes) in &files[..files.len() - 1] {
        assert!(manifest.contains(&format!(
            "{}  {name}",
            stutter_core::hash::sha256_hex(bytes)
        )));
    }
}

#[test]
fn missing_evidence_is_stated_never_filled_in() {
    let files: std::collections::BTreeMap<_, _> =
        build(&json!({"attempts_completed": 4}), &response())
            .into_iter()
            .collect();
    let parse = |name: &str| serde_json::from_slice::<Value>(&files[name]).unwrap();
    assert_eq!(
        parse("restore-proof.json")["restoration_verified"],
        json!(false)
    );
    assert_eq!(parse("action-transactions.json")["status"], "NotQualified");
    let blocked = parse("excluded-and-blocked.json");
    let reasons: Vec<_> = blocked["studies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["result"].clone())
        .collect();
    assert_eq!(reasons, vec![json!("blocked"), json!("undecided")]);
    assert!(
        String::from_utf8(files["campaign-summary.md"].clone())
            .unwrap()
            .contains("Disposition: PartiallyQualified")
    );
}

#[test]
fn an_unplanned_action_study_is_not_a_result() {
    let mut empty = response();
    empty["child_studies"] = json!([]);
    let files: std::collections::BTreeMap<_, _> = build(&json!({}), &empty).into_iter().collect();
    let oracle: Value = serde_json::from_slice(&files["diagnosis-oracle.json"]).unwrap();
    assert_eq!(oracle["status"], "NotPlanned");
}

#[test]
fn an_identical_verification_replays_and_a_changed_one_is_a_new_bundle_beside_the_old() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let handle = File::open(dir.path()).unwrap();
    let first = write(&handle, &json!({}), &response()).unwrap();
    assert_eq!(write(&handle, &json!({}), &response()).unwrap(), first);
    let mut changed = response();
    changed["verdict"]["disposition"] = json!("Qualified");
    let second = write(&handle, &json!({}), &changed).unwrap();
    assert_ne!(first, second);
    let old = std::fs::read_to_string(dir.path().join(&first).join("verify-result.json")).unwrap();
    assert!(
        old.contains("PartiallyQualified"),
        "the old bundle is never overwritten"
    );
}

#[test]
fn a_tampered_file_in_an_existing_bundle_is_refused_on_replay() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let handle = File::open(dir.path()).unwrap();
    let name = write(&handle, &json!({}), &response()).unwrap();
    let file = dir.path().join(&name).join("verify-result.json");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&file, b"{}\n").unwrap();
    let error = write(&handle, &json!({}), &response())
        .unwrap_err()
        .to_string();
    assert!(error.contains("differs"), "{error}");
}

/// Read-only on the campaign: derive the bundle of a real preserved campaign into a fresh directory.
/// `STUTTER_REAL_CAMPAIGN=<dir> STUTTER_REAL_RESULTS=<benchmarkResults> STUTTER_BUNDLE_OUT=<empty dir>
/// cargo test real_campaign_bundle -- --ignored`.
#[test]
#[ignore = "needs a preserved real campaign directory"]
fn real_campaign_bundle() {
    use std::os::fd::AsFd;

    use crate::autotune::research::{
        ResearchIntentV1, child_ledger::read_dispositions, driver::build_schedule,
        journal::ResearchJournal, raw_verify::verify_raw_evidence, report_store, verdict,
    };
    let var = |name: &str| std::env::var_os(name).unwrap_or_else(|| panic!("set {name}"));
    let uid = crate::syscall::geteuid();
    let dir = File::from(
        crate::syscall::open_dir_no_symlinks(std::path::Path::new(&var("STUTTER_REAL_CAMPAIGN")))
            .unwrap(),
    );
    let report = report_store::load_and_verify(&dir, uid).unwrap();
    let intent: ResearchIntentV1 =
        serde_json::from_slice(&report_store::read_private(&dir, "intent.json", uid).unwrap())
            .unwrap();
    let records = ResearchJournal::open(
        crate::syscall::openat_dir_nofollow(dir.as_fd(), "journal").unwrap(),
        uid,
        build_schedule(&intent).unwrap(),
    )
    .unwrap()
    .verify()
    .unwrap();
    let evidence_dir =
        File::from(crate::syscall::openat_dir_nofollow(dir.as_fd(), "evidence").unwrap());
    let results = std::path::PathBuf::from(var("STUTTER_REAL_RESULTS"));
    let raw = verify_raw_evidence(&evidence_dir, uid, &records, &|_| results.clone()).unwrap();
    let evidence: Vec<_> = report_store::read_evidence(&evidence_dir, uid)
        .unwrap()
        .into_values()
        .collect();
    let studies = read_dispositions(&dir, uid, &intent).unwrap();
    let verdict = verdict::judge(&verdict::VerdictInputs {
        reconstruction_passed: true,
        raw: Some(&raw),
        evidence: &evidence,
        child_studies: &studies,
        child_reports_verified: false,
        funnel: false,
        child_runtime_qualified: false,
        restoration_verified: false,
        health_blocked: false,
    });
    let response = json!({
        "verdict": verdict,
        "raw_native_files": raw,
        "child_studies": studies,
        "intent_sha256": report.intent_sha256,
    });
    let out = File::open(var("STUTTER_BUNDLE_OUT")).unwrap();
    write(&out, &serde_json::to_value(&report).unwrap(), &response).unwrap();
}

#[test]
fn the_capability_matrix_lists_every_recorded_integration_with_its_own_dispositions() {
    let files: std::collections::BTreeMap<_, _> =
        build(&json!({}), &response()).into_iter().collect();
    let matrix: Value = serde_json::from_slice(&files["capability-matrix.json"]).unwrap();
    let coverage = &matrix["integration_coverage"];
    assert_eq!(coverage["count"], 34);
    let rows = coverage["integrations"].as_array().unwrap();
    assert!(
        rows.iter()
            .all(|row| row["dispositions"].is_array() && row["id"].is_string())
    );
    assert_eq!(matrix["child_studies"].as_array().unwrap().len(), 3);
}

#[test]
fn the_comparison_carries_the_verified_child_reports_and_flags_unverified_children() {
    let mut full = response();
    full["child_reports"] = json!({
        "children": [
            { "study": "s1", "reconstruction": true, "raw_files_verified": true, "problem": null,
              "report": { "control": "ValveStable", "attempts_completed": 6 } },
            { "study": "s2", "reconstruction": false, "raw_files_verified": false,
              "problem": "tampered" }
        ],
        "missing": ["s3"]
    });
    let files: std::collections::BTreeMap<_, _> = build(&json!({"attempts_completed": 0}), &full)
        .into_iter()
        .collect();
    let comparison: Value = serde_json::from_slice(&files["proton-comparison.json"]).unwrap();
    let rows = comparison["child_reports"].as_array().unwrap();
    assert_eq!(rows[0]["profile"], "cpu");
    assert_eq!(rows[0]["report"]["attempts_completed"], 6);
    assert_eq!(rows[1]["report"], Value::Null);
    assert_eq!(rows[1]["problem"], "tampered");
    assert_eq!(comparison["flat_campaign_report"]["attempts_completed"], 0);
    let integrity: Value = serde_json::from_slice(&files["source-integrity.json"]).unwrap();
    assert_eq!(integrity["children_missing"], json!(["s3"]));
    assert!(integrity["child_reports"][0].get("report").is_none());
}

#[test]
fn a_qualified_action_study_needs_its_receipts_to_be_a_qualified_action() {
    let mut full = response();
    full["child_studies"] = json!([
        { "study_sha256": "ab".repeat(32), "profile": "long", "stage": {"stage": "action"},
          "result": "qualified", "reason": "" }
    ]);
    let action = |children: Value| {
        let mut with = full.clone();
        with["child_reports"] = json!({ "children": children, "missing": [] });
        let files: std::collections::BTreeMap<_, _> =
            build(&json!({}), &with).into_iter().collect();
        serde_json::from_slice::<Value>(&files["action-transactions.json"]).unwrap()
    };
    let without = action(json!([
        { "study": "abababababababab", "reconstruction": true, "raw_files_verified": true }
    ]));
    assert_eq!(without["status"], "ReceiptsMissing");
    let receipt = json!([{ "action": "CpuAffinityProfile", "pid": 4242,
        "original": {"cpus_allowed": "0-15", "cgroup": "0::/a"},
        "restored": {"cpus_allowed": "0-15", "cgroup": "0::/a"} }]);
    let with = action(json!([
        { "study": "abababababababab", "reconstruction": true, "raw_files_verified": true,
          "action_receipts": receipt }
    ]));
    assert_eq!(with["status"], "Qualified");
    assert_eq!(with["receipts"][0][0]["pid"], 4242);
}
