#[test]
fn rollback_production_defaults_use_typed_errors_not_string_bails() {
    let root = crate::architecture_tests::workspace_root();
    let path = root.join("stutter/src/actions/rollback/mod.rs");
    let source = std::fs::read_to_string(&path).expect("read rollback mod.rs");

    let production_source = source
        .split("#[cfg(test)]")
        .next()
        .expect("rollback.rs should have a production section");

    assert!(
        !production_source.contains("anyhow::bail!"),
        "{} production code should use typed errors instead of string-coded anyhow::bail!",
        path.display()
    );

    assert!(
        production_source.contains("RollbackRegistryError"),
        "{} should expose typed rollback registry errors",
        path.display()
    );
}
