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
    let (root, mut runner, study, _) = attempts(Host::finishing());
    assert_eq!(runner.poll(&study, 100).unwrap(), None);
    assert_eq!(runner.abort(), Ok(()));
    assert!(!runner.has_running());
    let seal = root
        .path()
        .join("studies")
        .join(&study.study_sha256[..16])
        .join("journal/0000.json");
    assert!(
        seal.is_file(),
        "an active attempt is durably sealed before release"
    );
}

#[test]
fn an_ambiguous_sent_launch_cannot_be_released_by_a_successful_noop_host_abort() {
    let mut host = Host::finishing();
    host.launch_unresolved = true;
    let (root, mut runner, study, _) = attempts(host);
    let error = runner.poll(&study, 100).unwrap_err();
    assert!(
        error
            .downcast_ref::<crate::autotune::research::driver::LaunchUnresolved>()
            .is_some()
    );
    assert_eq!(runner.abort(), Err("SettlementUnproven"));
    assert!(runner.has_running());
    assert!(
        !root
            .path()
            .join("studies")
            .join(&study.study_sha256[..16])
            .join("journal/0000.json")
            .exists()
    );
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
