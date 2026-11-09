//! The integration coverage ledger must stay truthful and complete.
use std::{collections::BTreeSet, fs};

use super::workspace_root;

fn ledger() -> serde_json::Value {
    let path = workspace_root().join("docs/integration-coverage.json");
    let text =
        fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[test]
fn every_dossier_and_extra_is_accounted_for_exactly_once() {
    let ledger = ledger();
    let ids: Vec<&str> = ledger["entries"]
        .as_array()
        .expect("entries array")
        .iter()
        .map(|entry| entry["id"].as_str().expect("entry id"))
        .collect();
    let unique: BTreeSet<&str> = ids.iter().copied().collect();
    assert_eq!(unique.len(), ids.len(), "duplicate ledger ids");
    for number in 1..=29 {
        let id = format!("5.{number}");
        assert!(
            unique.contains(id.as_str()),
            "dossier {id} is missing from the ledger"
        );
    }
    for extra in [
        "extra.mpi",
        "extra.diag",
        "extra.kernels",
        "extra.boot",
        "extra.workloads",
    ] {
        assert!(unique.contains(extra), "{extra} is missing from the ledger");
    }
}

#[test]
fn entries_are_truthful_about_status_and_anchors() {
    let ledger = ledger();
    let allowed: Vec<&str> = ledger["statuses"]
        .as_array()
        .expect("statuses")
        .iter()
        .filter_map(|status| status.as_str())
        .collect();
    let root = workspace_root();
    let mut problems = Vec::new();
    for entry in ledger["entries"].as_array().expect("entries") {
        let id = entry["id"].as_str().unwrap_or("?");
        let status = entry["status"].as_str().unwrap_or("");
        if !allowed.contains(&status) {
            problems.push(format!("{id}: unknown status {status:?}"));
        }
        let anchors = entry["anchors"].as_array().map_or(0, Vec::len);
        // A claim of any implementation needs a real file behind it, and none needs none.
        if status == "not_started" && anchors != 0 {
            problems.push(format!("{id}: not_started but lists code anchors"));
        }
        if status != "not_started" && anchors == 0 {
            problems.push(format!("{id}: {status} without a code anchor"));
        }
        for anchor in entry["anchors"].as_array().into_iter().flatten() {
            let anchor = anchor.as_str().unwrap_or("");
            if !root.join(anchor).exists() {
                problems.push(format!("{id}: anchor {anchor} does not exist"));
            }
        }
        for field in ["tools", "packages"] {
            if entry[field].as_array().is_none_or(Vec::is_empty) {
                problems.push(format!("{id}: empty {field}"));
            }
        }
        for field in ["g0", "g1", "runtime_availability", "authority", "not_wired"] {
            if entry[field].as_str().is_none_or(str::is_empty) {
                problems.push(format!("{id}: missing {field}"));
            }
        }
        let missing = entry["missing_required"].as_array().map_or(0, Vec::len);
        if status == "code_complete" && missing != 0 {
            problems.push(format!(
                "{id}: code_complete but {missing} required subcapabilities are missing"
            ));
        }
        if status == "code_complete"
            && entry["g1"].as_str() == Some("none")
            && entry["not_wired"].as_str() != Some("nothing")
        {
            problems.push(format!(
                "{id}: code_complete but something is still unwired"
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "integration ledger problems:\n{}",
        problems.join("\n")
    );
}
