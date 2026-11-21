use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::autotune::research::{ProtonSource, report::ResearchReportV1};

#[test]
fn a_completed_receipt_requires_its_prepared_decision_report_and_bundle() {
    let root = tempfile::tempdir().unwrap();
    std::fs::set_permissions(root.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    let dir = File::open(root.path()).unwrap();
    let uid = crate::syscall::geteuid();
    let report = ResearchReportV1 {
        schema_version: 1,
        intent_sha256: "a".repeat(64),
        attempts_scheduled: 0,
        attempts_sealed: 0,
        attempts_completed: 0,
        control: ProtonSource::ValveStable,
        arms: Vec::new(),
        primary_metric: None,
        treatment: None,
    };
    let report_bytes = serde_json::to_vec(&report).unwrap();
    write_identical(&dir, uid, "report.json", &report_bytes).unwrap();
    let gates: Vec<_> = (0..10)
        .map(|n| serde_json::json!({"gate": format!("G{n}"), "accepted": true, "blockers": []}))
        .collect();
    let verdict = serde_json::json!({
        "reconstruction_passed": true, "raw_files_verified": true,
        "loaded_runtime_qualified": true, "child_studies_qualified": true,
        "child_reports_verified": true, "restoration_verified": true,
        "final_gate_qualified": true, "disposition": "Qualified", "gates": gates,
    });
    let prepared = serde_json::to_vec(&serde_json::json!({
        "schema": 1, "intent_sha256": report.intent_sha256,
        "report_sha256": stutter_core::hash::sha256_hex(&report_bytes),
        "target_phase": "completed", "verdict": verdict,
    }))
    .unwrap();
    write_identical(&dir, uid, "finalization-prepared.json", &prepared).unwrap();
    let response = serde_json::json!({"intent_sha256": report.intent_sha256, "verdict": verdict});
    let bundle = crate::autotune::research::final_bundle::write(
        &dir,
        &serde_json::to_value(&report).unwrap(),
        &response,
    )
    .unwrap();
    let receipt = serde_json::to_vec(&serde_json::json!({
        "schema": 1, "intent_sha256": report.intent_sha256,
        "prepared_sha256": stutter_core::hash::sha256_hex(&prepared),
        "terminal_phase": "completed", "bundle": bundle, "verdict": verdict,
    }))
    .unwrap();
    write_identical(&dir, uid, "finalization-verified.json", &receipt).unwrap();
    verify_completed_receipt(&dir, uid, &report).unwrap();
    std::fs::write(
        root.path().join(&bundle).join("verify-result.json"),
        b"{}\n",
    )
    .unwrap();
    assert!(verify_completed_receipt(&dir, uid, &report).is_err());
}
