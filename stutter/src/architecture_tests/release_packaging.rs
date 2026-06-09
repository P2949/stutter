#[test]
fn release_readiness_tracks_distro_packaging_separately_from_service_units() {
    let root = crate::architecture_tests::workspace_root();
    let release = std::fs::read_to_string(root.join("stutter/src/release.rs"))
        .expect("read release readiness model");

    assert!(
        release.contains("production_distro_packaging"),
        "release readiness should track production distro packaging explicitly"
    );
    assert!(
        release.contains("reproducible_packaged_ebpf_object"),
        "release readiness should track packaged eBPF object readiness"
    );
    assert!(
        release.contains("packaging_install_tests"),
        "release readiness should track packaging install tests"
    );
    assert!(
        release.contains("packaging_service_smoke_tests"),
        "release readiness should track packaged service smoke tests"
    );
    assert!(
        release.contains("versioned_release_tarball"),
        "release readiness should track versioned release artifacts"
    );
    assert!(
        release.contains("\"production_distro_packaging\"")
            && release.contains("false")
            && release.contains("separate from source readiness"),
        "production distro packaging gate should be advisory and separate from source readiness"
    );
}

#[test]
fn packaging_docs_do_not_claim_distro_packages_are_production_ready() {
    let root = crate::architecture_tests::workspace_root();

    let install = std::fs::read_to_string(root.join("docs/INSTALL.md")).expect("read install docs");
    let packaging =
        std::fs::read_to_string(root.join("docs/PACKAGING.md")).expect("read packaging docs");
    let ebuild = std::fs::read_to_string(root.join("packaging/gentoo/stutter-9999.ebuild"))
        .expect("read Gentoo ebuild");

    assert!(
        install.contains("There is no production-ready distro package yet."),
        "install docs should keep distro packaging status honest"
    );
    assert!(
        packaging.contains("distro packaging is currently skeleton/experimental"),
        "packaging guide should state distro packaging is skeleton/experimental"
    );
    assert!(
        ebuild.contains("This ebuild is intentionally not production-ready yet."),
        "Gentoo ebuild should remain explicitly skeleton-only"
    );
}
