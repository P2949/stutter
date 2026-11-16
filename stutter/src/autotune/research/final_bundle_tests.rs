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
fn a_bundle_is_written_once_and_never_overwritten() {
    let dir = std::env::temp_dir().join(format!("stutter-bundle-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let handle = File::open(&dir).unwrap();
    write(&handle, &json!({}), &response()).unwrap();
    let first = std::fs::read(dir.join("verify-result.json")).unwrap();
    let mut changed = response();
    changed["verdict"]["disposition"] = json!("Qualified");
    write(&handle, &json!({}), &changed).unwrap();
    assert_eq!(
        std::fs::read(dir.join("verify-result.json")).unwrap(),
        first
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
