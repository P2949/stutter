#[test]
fn kernel_compatibility_docs_and_tracepoint_fixtures_exist() {
    let root = crate::architecture_tests::workspace_root();

    let docs = std::fs::read_to_string(root.join("docs/KERNEL_COMPATIBILITY.md"))
        .expect("read kernel compatibility docs");
    assert!(
        docs.contains("Runtime preflight is the source of truth"),
        "kernel compatibility docs should explain runtime preflight"
    );
    assert!(
        docs.contains("stutter doctor tracepoints --dump --json"),
        "kernel compatibility docs should tell users how to collect diagnostics"
    );
    assert!(
        docs.contains("does not validate tracepoint layouts against the build host kernel"),
        "kernel compatibility docs should reject build-host compile-time validation"
    );

    let fixtures = root.join("stutter/tests/fixtures/tracepoints");
    assert!(
        fixtures.exists(),
        "tracepoint fixture directory should exist at {}",
        fixtures.display()
    );
}

#[test]
fn tracepoint_preflight_errors_include_bug_report_dump_hint() {
    let root = crate::architecture_tests::workspace_root();
    let preflight = std::fs::read_to_string(root.join("stutter/src/ebpf/preflight/tracepoints.rs"))
        .expect("read tracepoint preflight source");

    assert!(
        preflight.contains("doctor tracepoints --dump --json"),
        "tracepoint preflight errors should include diagnostic dump instructions"
    );
    assert!(
        preflight.contains("TRACEPOINT_COMPATIBILITY_BUG_REPORT_HINT"),
        "tracepoint preflight should centralize the bug-report hint"
    );
}

#[test]
fn doctor_tracepoint_dump_reports_validation_status() {
    let root = crate::architecture_tests::workspace_root();
    let source = std::fs::read_to_string(root.join("stutter/src/doctor/tracepoints.rs"))
        .expect("read doctor tracepoints source");

    assert!(
        source.contains("TracepointFormatValidationDump"),
        "doctor tracepoint dump should include validation status"
    );
    assert!(
        source.contains("validate_tracepoint_format_named"),
        "doctor tracepoint dump should validate dumped formats against expected offsets"
    );
    assert!(
        source.contains("kernel_osrelease") && source.contains("kernel_version"),
        "doctor tracepoint dump should include kernel metadata for bug reports"
    );
}
