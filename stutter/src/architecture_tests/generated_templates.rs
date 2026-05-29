#[test]
fn scenario_generator_does_not_emit_literal_todo_markers() {
    let root = crate::architecture_tests::workspace_root();
    let source = std::fs::read_to_string(root.join("stutter/src/scenario/create.rs"))
        .expect("read scenario generator");

    assert!(
        !source.contains("\"TODO:") && !source.contains("TODO: describe the route"),
        "scenario generator should use neutral guidance, not TODO markers"
    );
    assert!(
        source.contains(
            "Describe the route and edit watch_process/tree_pid/pid before running this scenario."
        ),
        "scenario generator should keep neutral user guidance"
    );
}
