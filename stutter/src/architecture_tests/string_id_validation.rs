#[test]
fn stutter_core_string_ids_have_fallible_non_empty_constructors() {
    let root = crate::architecture_tests::workspace_root();
    let source = std::fs::read_to_string(root.join("stutter-core/src/ids.rs"))
        .expect("read stutter-core ids.rs");

    assert!(
        source.contains("pub struct EmptyStringIdError"),
        "stutter-core string IDs should expose EmptyStringIdError"
    );
    assert!(
        source.contains(
            "pub fn try_new(value: impl Into<String>) -> Result<Self, EmptyStringIdError>"
        ),
        "string_id! macro should provide try_new()"
    );
    assert!(
        source.contains("pub fn validate_non_empty(&self) -> Result<(), EmptyStringIdError>"),
        "string_id! macro should provide validate_non_empty() for deserialized values"
    );
}

#[test]
fn stutter_report_load_validates_deserialized_run_id() {
    let root = crate::architecture_tests::workspace_root();
    let load_source = std::fs::read_to_string(root.join("stutter-report/src/load.rs"))
        .expect("read stutter-report load.rs");
    let model_source = std::fs::read_to_string(root.join("stutter-report/src/model/root.rs"))
        .expect("read stutter-report model root");

    assert!(
        model_source.contains("validate_identity_strings"),
        "ReportModel should expose identity validation for serde-loaded IDs"
    );
    assert!(
        load_source.contains("validate_identity_strings()"),
        "stutter-report loader should validate deserialized string IDs"
    );
}

#[test]
fn autotune_experiment_id_uses_shared_core_id_type() {
    let root = crate::architecture_tests::workspace_root();
    let source = std::fs::read_to_string(root.join("stutter/src/autotune/experiment.rs"))
        .expect("read autotune experiment.rs");

    assert!(
        source.contains("pub use stutter_core::ids::ExperimentId;"),
        "autotune should use stutter_core::ids::ExperimentId instead of a duplicate local string ID"
    );
    assert!(
        !source.contains("pub struct ExperimentId(pub String)"),
        "autotune must not reintroduce a duplicate local ExperimentId"
    );
}
