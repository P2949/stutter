#[test]
fn autotune_complexity_has_targeted_invariant_and_mode_matrix_tests() {
    let root = crate::architecture_tests::workspace_root();

    let live_tests_mod =
        std::fs::read_to_string(root.join("stutter/src/autotune/live_experiment/tests/mod.rs"))
            .expect("read live experiment tests module");
    let live_invariants = std::fs::read_to_string(
        root.join("stutter/src/autotune/live_experiment/tests/invariants.rs"),
    )
    .expect("read live experiment invariant tests");

    assert!(
        live_tests_mod.contains("mod invariants;"),
        "live experiment tests should register invariant coverage"
    );
    assert!(
        live_invariants.contains("exposes_daemon_rollback_state_after_start"),
        "live experiment invariant tests should prove active apply experiments expose rollback state"
    );
    assert!(
        live_invariants.contains("decision_fingerprint")
            && live_invariants.contains("deterministic_for_identical_observation"),
        "live experiment invariant tests should prove deterministic decisions for identical observations"
    );

    let runtime_tests_mod =
        std::fs::read_to_string(root.join("stutter/src/autotune/runtime/tests/mod.rs"))
            .expect("read runtime tests module");
    let mode_matrix =
        std::fs::read_to_string(root.join("stutter/src/autotune/runtime/tests/mode_matrix.rs"))
            .expect("read runtime mode matrix tests");

    assert!(
        runtime_tests_mod.contains("mod mode_matrix;"),
        "runtime tests should register explicit mode-matrix coverage"
    );

    for required_mode in [
        "DaemonMode::Observe",
        "DaemonMode::Suggest",
        "DaemonMode::ApplyLowRisk",
        "DaemonMode::ApplyMediumRisk",
    ] {
        assert!(
            mode_matrix.contains(required_mode),
            "runtime mode matrix should cover {required_mode}"
        );
    }

    assert!(
        mode_matrix.contains("active_rollback")
            && mode_matrix.contains("requires explicit medium-risk unlock"),
        "runtime mode matrix should cover rollback exposure and medium-risk rejection"
    );
}
