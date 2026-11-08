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
