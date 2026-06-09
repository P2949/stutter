const TASK_MARKER_PREFIX: &str = concat!("TO", "DO", ":");
const OLD_SCENARIO_NOTE_PREFIX: &str = concat!("TO", "DO", ": describe the route");
const GENERATED_SCENARIO_DEFAULT_NOTES: &str =
    "Describe the route and edit watch_process/tree_pid/pid before running this scenario.";

#[test]
fn scenario_generator_does_not_emit_literal_task_markers() {
    let root = crate::architecture_tests::workspace_root();
    let source = std::fs::read_to_string(root.join("stutter/src/scenario/create.rs"))
        .expect("read scenario generator");

    assert!(
        !source.contains(TASK_MARKER_PREFIX) && !source.contains(OLD_SCENARIO_NOTE_PREFIX),
        "scenario generator should use neutral guidance, not task markers"
    );
    assert!(
        source.contains(GENERATED_SCENARIO_DEFAULT_NOTES),
        "scenario generator should keep neutral user guidance"
    );
}
