use super::*;

fn entry(status: &str, dispositions: &[&str], evidence: &[&str], unblock: Option<&str>) -> Entry {
    Entry {
        id: "x".into(),
        tools: vec!["tool".into()],
        packages: vec!["W1".into()],
        status: status.into(),
        dispositions: dispositions.iter().map(|d| (*d).to_owned()).collect(),
        evidence: evidence.iter().map(|e| (*e).to_owned()).collect(),
        unblock: unblock.map(str::to_owned),
        g0: String::new(),
        g1: String::new(),
        not_wired: "nothing | wired".into(),
        missing_required: Vec::new(),
    }
}

fn ledger(entries: Vec<Entry>) -> Ledger {
    Ledger {
        source_sha: "abc".into(),
        entries,
    }
}

#[test]
fn an_honest_ledger_validates_and_renders_counts_and_unblock_conditions() {
    let ledger = ledger(vec![
        entry(
            "first_slice",
            &["FixtureVerified", "HardwareUnavailable", "EvidenceDeferred"],
            &[],
            Some("Needs perf"),
        ),
        entry(
            "first_slice",
            &["FixtureVerified", "RealObserved"],
            &[],
            None,
        ),
    ]);
    validate(&ledger).unwrap();
    let report = render(&ledger, "6.18");
    assert!(report.contains("| FixtureVerified | 2 |"));
    assert!(report.contains("| CodeComplete | 0 |"));
    assert!(report.contains("Needs perf"));
    assert!(
        report.contains("nothing / wired"),
        "table pipes are escaped"
    );
}

#[test]
fn the_report_cannot_overstate_the_ledger() {
    let cases: Vec<(Entry, &str)> = vec![
        (entry("first_slice", &[], &[], None), "no disposition"),
        (
            entry("first_slice", &["Production"], &[], None),
            "not a disposition",
        ),
        (
            entry("first_slice", &["CodeComplete"], &[], None),
            "CodeComplete but the status",
        ),
        (
            entry("first_slice", &["BenefitValidated"], &[], None),
            "without an evidence path",
        ),
        (
            entry("first_slice", &["AutonomySupported"], &[], None),
            "without an evidence path",
        ),
        (
            entry("code_complete", &["CodeComplete"], &[], None),
            "needs verified fixtures",
        ),
        (
            entry("first_slice", &["HardwareUnavailable"], &[], None),
            "without an unblock",
        ),
    ];
    for (entry, expected) in cases {
        let error = validate(&ledger_of(entry)).unwrap_err().to_string();
        assert!(error.contains(expected), "{error}");
    }
    // With evidence the same claim is allowed.
    let proven = entry(
        "first_slice",
        &["MechanismValidated"],
        &["docs/evidence/x"],
        None,
    );
    assert!(validate(&ledger_of(proven)).is_ok());
}

fn ledger_of(entry: Entry) -> Ledger {
    ledger(vec![entry])
}

#[test]
fn the_real_ledger_in_the_repository_validates() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let text = fs::read(root.join("docs/integration-coverage.json")).unwrap();
    let ledger: Ledger = serde_json::from_slice(&text).unwrap();
    validate(&ledger).unwrap();
    assert_eq!(ledger.entries.len(), 34);
}

#[test]
fn code_complete_with_a_missing_required_subcapability_is_refused() {
    let mut incomplete = entry("code_complete", &["FixtureVerified"], &[], None);
    incomplete.missing_required = vec!["resctrl/mba-allocation".into()];
    let error = validate(&ledger(vec![incomplete])).unwrap_err().to_string();
    assert!(error.contains("resctrl/mba-allocation"), "{error}");
    let mut partial = entry("first_slice", &["FixtureVerified"], &[], None);
    partial.missing_required = vec!["resctrl/mba-allocation".into()];
    validate(&ledger(vec![partial])).unwrap();
}

fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

#[test]
fn the_repository_subcapability_manifest_verifies_against_its_anchors_and_tests() {
    let root = workspace_root();
    let ledger: Ledger =
        serde_json::from_slice(&fs::read(root.join("docs/integration-coverage.json")).unwrap())
            .unwrap();
    let manifest = subcaps::load(&root).unwrap();
    subcaps::validate(&manifest, &ledger, &root).unwrap();
}

#[test]
fn a_manifest_that_drops_a_baseline_item_or_names_a_missing_anchor_or_test_fails() {
    let root = workspace_root();
    let ledger: Ledger =
        serde_json::from_slice(&fs::read(root.join("docs/integration-coverage.json")).unwrap())
            .unwrap();
    let load = || subcaps::load(&root).unwrap();
    let mut dropped = load();
    dropped
        .items
        .retain(|item| item.id != "resctrl/mba-allocation");
    let error = subcaps::validate(&dropped, &ledger, &root)
        .unwrap_err()
        .to_string();
    assert!(error.contains("resctrl/mba-allocation"), "{error}");

    let mut anchor = load();
    anchor.items[0].anchor = "stutter/src/no_such_file.rs".into();
    assert!(subcaps::validate(&anchor, &ledger, &root).is_err());

    let mut symbol = load();
    symbol.items[0].symbol = "definitely_not_in_that_file_xyzzy".into();
    assert!(subcaps::validate(&symbol, &ledger, &root).is_err());

    let mut test = load();
    test.items[0].negative_test = "a_test_that_does_not_exist".into();
    let error = subcaps::validate(&test, &ledger, &root)
        .unwrap_err()
        .to_string();
    assert!(error.contains("a_test_that_does_not_exist"), "{error}");

    let mut kind = load();
    kind.items[0].kind = "actuator".into();
    assert!(subcaps::validate(&kind, &ledger, &root).is_err());
}

#[test]
fn only_documentation_changes_are_allowed_between_the_ledger_source_and_head() {
    let changed = [
        "docs/validation/x.json",
        "README.md",
        "stutter/src/actions/damos_scheme.rs",
        "xtask/Cargo.toml",
    ];
    assert_eq!(
        subcaps::code_changes(&changed),
        ["stutter/src/actions/damos_scheme.rs", "xtask/Cargo.toml"]
    );
    assert!(subcaps::code_changes(&["docs/a.md", "docs/b/c.json"]).is_empty());
}
