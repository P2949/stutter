use serde_json::json;

use super::*;

const NAMES: [&str; 12] = [
    "campaign-summary.md",
    "report.json",
    "crosswalk-34.json",
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
fn summary_counts_reconstructed_children_and_labels_missing_evidence() {
    let mut value = response();
    value["child_reports"] = json!({ "children": [
        { "reconstruction": true, "report": { "attempts_completed": 6 } },
        { "reconstruction": true, "report": { "attempts_completed": 3 } }
    ], "missing": [] });
    let report = json!({ "attempts_completed": 0 });
    assert!(summary(&report, &value).contains("Attempts completed: 9 (verified child reports)."));
    value["child_reports"]["children"][1]["reconstruction"] = json!(false);
    value["child_reports"]["missing"] = json!(["missing-study"]);
    assert!(summary(&report, &value).contains("Attempts completed: at least 6 (verified child reports; some child evidence is unavailable)."));
    value["child_reports"] = json!({ "children": [], "missing": [] });
    assert!(summary(&json!({"attempts_completed": 4}), &value).contains("Attempts completed: 4."));
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
    let uid = crate::syscall::geteuid();
    verify_written(&handle, uid, &json!({}), &response(), &name).unwrap();
    verify_manifest(&handle, uid, &name, &json!({}), &response()["verdict"]).unwrap();
    let file = dir.path().join(&name).join("verify-result.json");
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    std::fs::write(&file, b"{}\n").unwrap();
    assert!(verify_written(&handle, uid, &json!({}), &response(), &name).is_err());
    assert!(verify_manifest(&handle, uid, &name, &json!({}), &response()["verdict"]).is_err());
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
        child_telemetry: None,
        child_replication: None,
        fault_matrix: Err("NoFaultMatrixReceipt"),
        pressure_control: Err("NoPressureControl"),
        child_resident: Ok(()),
        sealed_protected_state: Ok(()),
        protected_state: Ok(()),
        hardware: Ok(()),
        control_socket: None,
        deployment_attested: false,
        lease: None,
        host: None,
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

#[test]
fn the_summary_lists_every_gate_with_its_blockers() {
    let mut with_gates = response();
    with_gates["verdict"]["gates"] = json!([
        { "gate": "G2", "accepted": true, "blockers": [] },
        { "gate": "G5", "accepted": false, "blockers": ["ActionNotQualified", "RestorationUnverified"] }
    ]);
    let files: std::collections::BTreeMap<_, _> =
        build(&json!({}), &with_gates).into_iter().collect();
    let text = String::from_utf8(files["campaign-summary.md"].clone()).unwrap();
    assert!(text.contains("- G2: accepted\n"), "{text}");
    assert!(
        text.contains("- G5: not accepted (ActionNotQualified, RestorationUnverified)\n"),
        "{text}"
    );
}

#[test]
fn a_restored_action_is_not_a_demonstrated_benefit() {
    let mut full = response();
    full["child_studies"] = json!([
        { "study_sha256": "ab".repeat(32), "profile": "long", "stage": {"stage": "action"},
          "result": "qualified", "reason": "" }
    ]);
    let receipt = json!([{ "action": "CpuAffinityProfile", "pid": 4242, "transaction": "exp-1" }]);
    let action = |verdict: Value, pairs: u32| {
        let mut with = full.clone();
        with["child_reports"] = json!({
            "children": [{
                "study": "abababababababab", "reconstruction": true, "raw_files_verified": true,
                "action_receipts": receipt,
                "report": { "arms": [{ "status": "native_context_consistent_unqualified",
                                       "pairs": pairs, "numeric_verdict": verdict }] }
            }],
            "missing": []
        });
        let files: std::collections::BTreeMap<_, _> =
            build(&json!({}), &with).into_iter().collect();
        serde_json::from_slice::<Value>(&files["action-transactions.json"]).unwrap()
    };
    // Safely applied and restored, but the off/on comparison found nothing: no benefit claimed.
    let neutral = action(json!("no_useful_effect"), 4);
    assert_eq!(neutral["status"], "Qualified");
    assert_eq!(neutral["benefit_demonstrated"], false);
    assert_eq!(neutral["outcome_comparison"][0]["pairs"], 4);
    // An improvement is reported separately from the safety claim.
    assert_eq!(action(json!("improved"), 4)["benefit_demonstrated"], true);
    // A refused comparison (too few pairs) has no verdict and demonstrates nothing.
    assert_eq!(action(Value::Null, 1)["benefit_demonstrated"], false);
}

#[test]
fn the_capability_matrix_distinguishes_the_feature_dispositions_and_proves_none() {
    let files: std::collections::BTreeMap<_, _> =
        build(&json!({}), &response()).into_iter().collect();
    let matrix: Value = serde_json::from_slice(&files["capability-matrix.json"]).unwrap();
    let rows = matrix["feature_dispositions"].as_array().unwrap();
    assert_eq!(rows.len(), 60);
    let kinds: std::collections::BTreeSet<_> = rows
        .iter()
        .filter_map(|row| row["disposition"].as_str())
        .collect();
    assert!(kinds.contains("NotApplicable") && kinds.contains("Unavailable"));
    assert!(kinds.contains("ControlAvailableNotFunctionallyProven"));
    assert!(!kinds.contains("FunctionallyProven"));
}

#[test]
fn the_child_rows_carry_the_telemetry_and_the_process_identities_the_join_used() {
    let mut full = response();
    full["child_reports"] = json!({
        "children": [{
            "study": "s1", "reconstruction": true, "raw_files_verified": true, "problem": null,
            "telemetry_receipts": [{ "attempt": 0, "pid": 7, "frame_verdict": "Eligible" }],
            "completed_attempts": [0],
            "completed_processes": [{ "slot": 0, "pid": 7, "start_ticks": 9, "uid": 1000 }]
        }],
        "missing": []
    });
    let files: std::collections::BTreeMap<_, _> = build(&json!({}), &full).into_iter().collect();
    let integrity: Value = serde_json::from_slice(&files["source-integrity.json"]).unwrap();
    let row = &integrity["child_reports"][0];
    assert_eq!(row["telemetry_receipts"][0]["frame_verdict"], "Eligible");
    assert_eq!(row["completed_attempts"], json!([0]));
    assert_eq!(row["completed_processes"][0]["start_ticks"], 9);
    let restore: Value = serde_json::from_slice(&files["restore-proof.json"]).unwrap();
    let basis = restore["basis"].as_str().unwrap();
    assert!(
        basis.contains("treated On") && !basis.contains("no physical restoration verifier"),
        "{basis}"
    );
}

#[test]
fn a_bundle_that_cannot_be_written_is_reported_not_just_logged() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let dir = File::open(root.path()).unwrap();
    // A written bundle names itself.
    let name = write_reporting(&dir, &json!({}), &response()).unwrap();
    assert!(name.starts_with("bundle-"), "{name}");
    // A directory that does not hold a private bundle location cannot take one: the error comes back.
    let file = root.path().join("not-a-dir");
    std::fs::write(&file, b"x").unwrap();
    let handle = File::open(&file).unwrap();
    assert!(write_reporting(&handle, &json!({}), &response()).is_err());
}

#[test]
fn the_diagnosis_oracle_file_carries_the_scored_cases_or_the_reason_there_are_none() {
    let mut scored = response();
    scored["diagnosis_oracle"] = json!({
        "status": "scored",
        "evaluation": { "positives": 3, "included": 3, "gate_passed": true }
    });
    let files: std::collections::BTreeMap<_, _> = build(&json!({}), &scored).into_iter().collect();
    let oracle: Value = serde_json::from_slice(&files["diagnosis-oracle.json"]).unwrap();
    assert_eq!(oracle["scored"]["status"], "scored");
    assert_eq!(oracle["scored"]["evaluation"]["gate_passed"], true);
    // With no case file the bundle says why rather than restating a study status.
    let mut none = response();
    none["diagnosis_oracle"] = json!({ "status": "not_scored", "reason": "NoOracleCases" });
    let files: std::collections::BTreeMap<_, _> = build(&json!({}), &none).into_iter().collect();
    let oracle: Value = serde_json::from_slice(&files["diagnosis-oracle.json"]).unwrap();
    assert_eq!(oracle["scored"]["reason"], "NoOracleCases");
}

#[test]
fn the_bundle_carries_the_complete_thirty_four_row_crosswalk_with_no_validated_benefit() {
    let files = build(&json!({}), &json!({}));
    let (_, bytes) = files
        .iter()
        .find(|(name, _)| *name == "crosswalk-34.json")
        .expect("crosswalk file");
    let matrix: Value = serde_json::from_slice(bytes).unwrap();
    let crosswalk = &matrix["crosswalk_34"];
    assert_eq!(crosswalk["count"], 34, "{crosswalk}");
    assert_eq!(crosswalk["rows"].as_array().unwrap().len(), 34);
    assert!(
        !crosswalk.to_string().contains("GameBenefitValidated\"")
            || crosswalk["note"]
                .as_str()
                .unwrap()
                .contains("not GameBenefitValidated")
    );
}

#[test]
fn host_dependent_gates_are_reported_apart_from_the_historical_ones() {
    let verdict = json!({ "gates": [
        { "gate": "G0", "accepted": false }, { "gate": "G2", "accepted": true },
        { "gate": "G8", "accepted": false }, { "gate": "G9", "accepted": true },
    ]});
    let scope = gate_scope(&verdict);
    let names = |key: &str| -> Vec<String> {
        scope[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|g| g["gate"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(names("historical_sealed_evidence"), ["G2", "G9"]);
    assert_eq!(names("current_host_at_verification"), ["G0", "G8"]);
}

#[test]
fn a_numeric_improvement_of_an_unqualified_action_study_is_not_a_demonstrated_benefit() {
    let response = |result: &str, problem: Value| {
        json!({
            "verdict": { "gates": [] },
            "child_studies": [{
                "study_sha256": "a".repeat(64), "profile": "long", "stage": "action", "result": result,
            }],
            "children": { "children": [{
                "study": "a".repeat(16), "reconstruction": true, "problem": problem,
                "report": { "arms": [{ "status": "x", "pairs": 6, "numeric_verdict": "improved" }] },
            }]},
        })
    };
    let benefit = |response: &Value| -> Value {
        let files = build(&json!({}), response);
        let (_, bytes) = files
            .iter()
            .find(|(n, _)| *n == "action-transactions.json")
            .unwrap();
        serde_json::from_slice::<Value>(bytes).unwrap()["benefit_demonstrated"].clone()
    };
    // Whatever the shape of the fixture, a study that is not qualified can never demonstrate a benefit.
    assert_eq!(benefit(&response("unqualified", Value::Null)), json!(false));
    assert_eq!(
        benefit(&response("qualified", json!("a problem"))),
        json!(false)
    );
}

#[test]
fn every_bundle_file_fits_the_private_store_record_limit_with_the_real_ledger() {
    // The store refuses a record over 64 KiB, and a refused bundle ends a campaign as unverifiable.
    for (name, bytes) in build(&json!({}), &json!({})) {
        assert!(bytes.len() < 60 * 1024, "{name} is {} bytes", bytes.len());
    }
}

#[test]
fn only_the_dossiers_the_campaigns_own_record_names_with_a_digest_read_as_observed() {
    let table = |response: Value| -> Value {
        let files = build(&json!({}), &response);
        let (_, bytes) = files
            .iter()
            .find(|(n, _)| *n == "crosswalk-34.json")
            .unwrap();
        serde_json::from_slice::<Value>(bytes).unwrap()["crosswalk_34"].clone()
    };
    let observed = |response: Value| table(response)["observed_in_campaign"].clone();
    assert_eq!(observed(json!({})), 0);
    let digest = "ab".repeat(32);
    let good = json!({ "crosswalk_observations": { "rows": [{ "id": "5.1", "evidence_sha256": digest }] } });
    assert_eq!(observed(good), 1);
    // An unknown id, a malformed digest, a repeated id or a record the report marked invalid makes the whole record
    // unusable, and is reported as an integrity failure rather than as a campaign that observed nothing.
    for bad in [
        json!({ "rows": [{ "id": "9.9", "evidence_sha256": digest }] }),
        json!({ "rows": [{ "id": "5.1", "evidence_sha256": "xyz" }] }),
        json!({ "rows": [{ "id": "5.1", "evidence_sha256": digest }, { "id": "5.1", "evidence_sha256": digest }] }),
    ] {
        assert_eq!(
            table(json!({ "crosswalk_observations": bad }))["status"],
            "CrosswalkObservationsInvalid"
        );
    }
    assert_eq!(
        table(json!({ "crosswalk_observations": { "error": "CrosswalkObservationsInvalid" } }))["status"],
        "CrosswalkObservationsInvalid"
    );
}

/// A CP-LONG action study has no native FPS, so its report arm is `ActionMetricUnavailable`. The measured effect the
/// bundle reports is the runnable-latency result reconstructed from the sealed paired record: a benefit only on Keep,
/// and a regression or no effect is reported as such, never read from the placeholder.
#[test]
fn a_long_action_studys_measured_runnable_latency_result_reaches_the_bundle() {
    let mut full = response();
    full["child_studies"] = json!([
        { "study_sha256": "ab".repeat(32), "profile": "long", "stage": {"stage": "action"},
          "result": "qualified", "reason": "" }
    ]);
    let receipt = json!([{ "action": "CpuAffinityProfile", "pid": 4242, "transaction": "exp-1" }]);
    let action = |latency: Value| {
        let mut with = full.clone();
        with["child_reports"] = json!({
            "children": [{
                "study": "abababababababab", "reconstruction": true, "raw_files_verified": true,
                "action_receipts": receipt, "action_latency_result": latency,
                "report": { "arms": [{ "status": "ActionMetricUnavailable", "pairs": 0 }] }
            }],
            "missing": []
        });
        let files: std::collections::BTreeMap<_, _> =
            build(&json!({}), &with).into_iter().collect();
        serde_json::from_slice::<Value>(&files["action-transactions.json"]).unwrap()
    };
    let result = |decision: &str, improved: u32, regressed: u32| {
        json!({ "objective": "game_runnable_latency", "pairs": 4, "improved": improved,
                "regressed": regressed, "decision": decision })
    };
    let kept = action(result("keep", 4, 0));
    assert_eq!(kept["benefit_demonstrated"], true);
    assert_eq!(
        kept["outcome_comparison"][0]["status"],
        "MeasuredRunnableLatency"
    );
    assert_eq!(kept["outcome_comparison"][0]["numeric_verdict"], "improved");
    assert_eq!(kept["outcome_comparison"][0]["pairs"], 4);
    assert_eq!(
        kept["outcome_comparison"][0]["runnable_latency"]["objective"],
        "game_runnable_latency"
    );
    let reverted = action(result("revert", 0, 4));
    assert_eq!(reverted["benefit_demonstrated"], false);
    assert_eq!(
        reverted["outcome_comparison"][0]["numeric_verdict"],
        "regressed"
    );
    let null = action(result("null", 0, 0));
    assert_eq!(null["benefit_demonstrated"], false);
    assert_eq!(
        null["outcome_comparison"][0]["numeric_verdict"],
        "no_effect"
    );
    // Without a reconstructed result the placeholder is still never a measurement.
    let missing = action(Value::Null);
    assert_eq!(missing["benefit_demonstrated"], false);
    assert_eq!(
        missing["outcome_comparison"][0]["status"],
        "ActionMetricUnavailable"
    );
}
