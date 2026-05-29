#[test]
fn report_crate_docs_match_current_ownership_boundary() {
    let root = crate::architecture_tests::workspace_root();

    let report_lib = std::fs::read_to_string(root.join("stutter-report/src/lib.rs"))
        .expect("read stutter-report crate docs")
        .replace("\n//! ", " ");
    let report_model = std::fs::read_to_string(root.join("stutter-report/src/model/root.rs"))
        .expect("read stutter-report report model docs")
        .replace("\n/// ", " ");
    let migration_doc = std::fs::read_to_string(root.join("docs/REPORT_CRATE_MIGRATION.md"))
        .expect("read report crate migration docs")
        .replace('\n', " ");

    assert!(
        report_lib.contains("initial report-crate migration is complete"),
        "stutter-report crate docs should describe the initial migration as complete"
    );
    assert!(
        report_lib.contains("basic self-contained HTML rendering"),
        "stutter-report crate docs should acknowledge crate-local basic HTML rendering"
    );
    assert!(
        report_lib.contains("Rich CLI report assembly")
            && report_lib.contains("still live in the main `stutter` crate"),
        "stutter-report crate docs should identify remaining main-crate report ownership"
    );

    assert!(
        report_model.contains("stable report fields")
            && report_model.contains("Rich CLI HTML/report assembly still"),
        "ReportModel docs should describe the migrated model boundary precisely"
    );

    assert!(
        migration_doc.contains("The initial report-crate migration is complete"),
        "migration doc should distinguish initial migration completion from all report work"
    );
    assert!(
        migration_doc.contains("This means the initial migration checklist is closed")
            && migration_doc.contains("does **not** mean every report-related responsibility"),
        "migration doc should not read as if all report work is done"
    );
    assert!(
        migration_doc.contains("The main `stutter` crate still owns"),
        "migration doc should list remaining main-crate report responsibilities"
    );
    assert!(
        migration_doc.contains("Documentation rule"),
        "migration doc should preserve the wording rule for future updates"
    );
}

#[test]
fn report_crate_docs_do_not_use_stale_remaining_logic_wording() {
    let root = crate::architecture_tests::workspace_root();

    let report_lib = std::fs::read_to_string(root.join("stutter-report/src/lib.rs"))
        .expect("read stutter-report crate docs")
        .replace("\n//! ", " ");
    let migration_doc = std::fs::read_to_string(root.join("docs/REPORT_CRATE_MIGRATION.md"))
        .expect("read report crate migration docs")
        .replace('\n', " ");

    assert!(
        !report_lib.contains("The remaining main-crate report logic is tracked"),
        "stutter-report crate docs should not use stale migration-placeholder wording"
    );

    assert!(
        !migration_doc.contains("# Report Crate Migration Checklist")
            || migration_doc.contains("Remaining follow-up work"),
        "migration doc should not be only a fully checked checklist"
    );
}
