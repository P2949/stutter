use super::*;

fn arm(registry: usize, stamp: Option<&str>, pfx: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    if pfx {
        std::fs::create_dir(dir.path().join("pfx")).unwrap();
        for name in REGISTRY_FILES.iter().take(registry) {
            std::fs::write(dir.path().join("pfx").join(name), b"x").unwrap();
        }
    }
    if let Some(stamp) = stamp {
        std::fs::write(dir.path().join(VERSION_STAMP), stamp).unwrap();
    }
    dir
}

#[test]
fn a_prefix_is_cold_warm_or_indeterminate_by_what_a_run_left_behind() {
    assert_eq!(observe(arm(0, None, false).path()).class, PrefixClass::Cold);
    let warm = observe(arm(3, Some("11.0-100"), true).path());
    assert_eq!(warm.class, PrefixClass::Warm);
    assert_eq!(warm.stamped_by.as_deref(), Some("11.0-100"));
    // An empty or partly initialised prefix is not either.
    assert_eq!(
        observe(arm(0, None, true).path()).class,
        PrefixClass::Indeterminate
    );
    assert_eq!(
        observe(arm(2, None, true).path()).class,
        PrefixClass::Indeterminate
    );
    // Observing never creates anything.
    let absent = tempfile::tempdir().unwrap();
    observe(absent.path());
    assert!(!absent.path().join("pfx").exists());
    // A missing directory is cold, not an error.
    assert_eq!(
        observe(Path::new("/nonexistent/arm")).class,
        PrefixClass::Cold
    );
}

#[test]
fn only_arms_in_the_same_class_with_their_own_stamps_are_comparable() {
    let cold = |_: ()| (observe(arm(0, None, false).path()), "x");
    assert_eq!(comparable(&[cold(()), cold(())]), Ok(PrefixClass::Cold));
    let warm = |stamp: &str, own: &'static str| (observe(arm(3, Some(stamp), true).path()), own);
    assert_eq!(
        comparable(&[
            warm("11.0-100", "11.0-100"),
            warm("CachyOS-11.0-100", "CachyOS-11.0-100")
        ]),
        Ok(PrefixClass::Warm)
    );
    // Cold next to warm is a different experiment.
    assert_eq!(
        comparable(&[cold(()), warm("11.0-100", "11.0-100")]),
        Err(Incomparable::MixedColdAndWarm)
    );
    // A warm prefix last used by another Proton carries that build's state.
    assert_eq!(
        comparable(&[
            warm("11.0-100", "11.0-100"),
            warm("11.0-100", "CachyOS-11.0-100")
        ]),
        Err(Incomparable::ForeignStamp(1))
    );
    // An unstamped warm prefix says nothing about who used it.
    let unstamped = (observe(arm(3, None, true).path()), "11.0-100");
    assert_eq!(
        comparable(&[unstamped, warm("CachyOS-11.0-100", "CachyOS-11.0-100")]),
        Err(Incomparable::UnstampedWarm(0))
    );
    // Indeterminate poisons the comparison.
    let broken = (observe(arm(1, None, true).path()), "x");
    assert_eq!(
        comparable(&[cold(()), broken]),
        Err(Incomparable::Indeterminate(1))
    );
}

#[test]
fn a_stamp_that_merely_contains_the_expected_one_is_not_the_same_build() {
    // The real stamps: GE writes "11.0-100"; CachyOS writes "CachyOS-11.0-100", which contains it.
    let cachy_prefix = observe(arm(3, Some("CachyOS-11.0-100"), true).path());
    let ge_prefix = observe(arm(3, Some("11.0-100"), true).path());
    assert_eq!(
        comparable(&[
            (cachy_prefix.clone(), "11.0-100"),
            (ge_prefix.clone(), "11.0-100")
        ]),
        Err(Incomparable::ForeignStamp(0)),
        "a CachyOS prefix is not a GE prefix however the strings overlap"
    );
    assert_eq!(
        comparable(&[(ge_prefix, "11.0-100"), (cachy_prefix, "CachyOS-11.0-100")]),
        Ok(PrefixClass::Warm)
    );
    // Whitespace around the stamp is not part of the identity.
    let padded = observe(arm(3, Some("11.0-100\n"), true).path());
    assert_eq!(padded.stamped_by.as_deref(), Some("11.0-100"));
}
