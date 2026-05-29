#[test]
fn probe_activation_uses_activation_specs_not_raw_registry() {
    let root = crate::architecture_tests::workspace_root();
    let source = std::fs::read_to_string(root.join("stutter/src/probe_activation.rs"))
        .expect("read probe activation source");

    assert!(
        source.contains("activation_probe_specs()"),
        "probe activation should iterate activation_probe_specs()"
    );
    assert!(
        !source.contains("for spec in PROBE_REGISTRY"),
        "probe activation should not iterate raw PROBE_REGISTRY"
    );
    assert!(
        !source.contains("planned probe is not implemented"),
        "probe activation should not expose planned probes as disabled missing functionality"
    );
}

#[test]
fn probe_catalog_has_explicit_include_planned_path() {
    let root = crate::architecture_tests::workspace_root();

    let catalog = std::fs::read_to_string(root.join("stutter/src/probe_catalog.rs"))
        .expect("read probe catalog");
    assert!(
        catalog.contains("ProbeCatalogOptions")
            && catalog.contains("include_planned")
            && catalog.contains("visible_probe_specs(options.include_planned)"),
        "probe catalog should hide planned probes by default and expose include_planned option"
    );

    let cli = std::fs::read_to_string(root.join("stutter/src/cli/report.rs"))
        .expect("read CLI report args");
    assert!(
        cli.contains("include_planned") && cli.contains("long = \"include-planned\""),
        "probes CLI should expose explicit --include-planned flag"
    );
}
