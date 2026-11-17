use super::*;

#[test]
fn an_abort_whose_settlement_is_unproven_keeps_the_running_attempt_owned() {
    let mut host = Host::finishing();
    host.abort_fails = true;
    let (_root, mut runner, study, _) = attempts(host);
    assert_eq!(runner.poll(&study, 100).unwrap(), None);
    assert!(runner.has_running());
    assert_eq!(runner.abort(), Err("SettlementUnproven"));
    // The driver that owns the unsettled attempt is not dropped: a failed settlement stays visible.
    assert!(runner.has_running());
}

#[test]
fn a_proven_abort_releases_the_running_attempt() {
    let (_root, mut runner, study, _) = attempts(Host::finishing());
    assert_eq!(runner.poll(&study, 100).unwrap(), None);
    assert_eq!(runner.abort(), Ok(()));
    assert!(!runner.has_running());
}

#[test]
fn a_study_over_incomparable_prefixes_is_refused_before_anything_is_created() {
    let mut host = Host::finishing();
    host.prefix_refusal = Some("PrefixMixedColdAndWarm");
    let (root, mut runner, study, _) = attempts(host);
    let error = runner.poll(&study, 100).unwrap_err().to_string();
    assert!(error.contains("PrefixMixedColdAndWarm"), "{error}");
    assert!(!root.path().join("studies").exists(), "nothing was created");
    assert!(!runner.has_running());
}
