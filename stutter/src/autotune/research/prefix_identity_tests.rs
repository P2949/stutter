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

mod enforcement {
    use super::*;

    fn tool(declared: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("proton"),
            format!("#!/usr/bin/env python3\nimport os\nCURRENT_PREFIX_VERSION=\"{declared}\"\n"),
        )
        .unwrap();
        dir
    }

    #[test]
    fn a_builds_own_stamp_is_what_its_proton_script_declares() {
        assert_eq!(
            own_stamp(tool("11.0-100").path()).as_deref(),
            Some("11.0-100")
        );
        assert_eq!(
            own_stamp(tool("CachyOS-11.0-100").path()).as_deref(),
            Some("CachyOS-11.0-100")
        );
        let single = tempfile::tempdir().unwrap();
        std::fs::write(
            single.path().join("proton"),
            "CURRENT_PREFIX_VERSION = 'EM-11.0-101'\n",
        )
        .unwrap();
        assert_eq!(own_stamp(single.path()).as_deref(), Some("EM-11.0-101"));
        let none = tempfile::tempdir().unwrap();
        std::fs::write(none.path().join("proton"), "print('hi')\n").unwrap();
        assert_eq!(own_stamp(none.path()), None);
        assert_eq!(own_stamp(Path::new("/nonexistent")), None);
    }

    #[test]
    fn the_study_path_refuses_prefixes_that_are_not_comparable() {
        let (ge, cachy) = (tool("11.0-100"), tool("CachyOS-11.0-100"));
        let pair = |a: &tempfile::TempDir, b: &tempfile::TempDir| {
            vec![
                (a.path().to_path_buf(), ge.path().to_path_buf()),
                (b.path().to_path_buf(), cachy.path().to_path_buf()),
            ]
        };
        // Both cold: comparable (the first run creates each).
        let (cold_a, cold_b) = (arm(0, None, false), arm(0, None, false));
        assert_eq!(check(&pair(&cold_a, &cold_b)), Ok(PrefixClass::Cold));
        // Both warm and stamped by their own build: comparable.
        let (warm_a, warm_b) = (
            arm(3, Some("11.0-100"), true),
            arm(3, Some("CachyOS-11.0-100"), true),
        );
        assert_eq!(check(&pair(&warm_a, &warm_b)), Ok(PrefixClass::Warm));
        // One cold beside one warm is a different experiment.
        assert_eq!(
            check(&pair(&cold_a, &warm_b)),
            Err("PrefixMixedColdAndWarm")
        );
        // A warm prefix last used by a different build, or never stamped, is not this arm's.
        let foreign = arm(3, Some("CachyOS-11.0-100"), true);
        assert_eq!(check(&pair(&foreign, &warm_b)), Err("PrefixForeignStamp"));
        let unstamped = arm(3, None, true);
        assert_eq!(check(&pair(&unstamped, &warm_b)), Err("PrefixUnstamped"));
        // A half-initialised prefix is neither.
        let partial = arm(1, Some("11.0-100"), true);
        assert_eq!(check(&pair(&partial, &warm_b)), Err("PrefixIndeterminate"));
        // A build whose own stamp cannot be read cannot own a prefix.
        let no_script = vec![(warm_a.path().to_path_buf(), PathBuf::from("/nonexistent"))];
        assert_eq!(check(&no_script), Err("PrefixOwnStampUnknown"));
    }
}
