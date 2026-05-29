#[test]
fn live_autotune_cli_does_not_expose_apply_high_risk_as_mode_value() {
    let root = crate::architecture_tests::workspace_root();

    let cli_source = std::fs::read_to_string(root.join("stutter/src/cli/autotune.rs"))
        .expect("read cli autotune source");
    assert!(
        cli_source.contains("enum LiveAutotuneMode"),
        "live autotune CLI should use a CLI-specific mode enum"
    );
    assert!(
        !cli_source.contains("LiveAutotuneMode::ApplyHighRisk")
            && !cli_source.contains("ApplyHighRisk,"),
        "LiveAutotuneMode should not have an ApplyHighRisk variant"
    );
    assert!(
        cli_source.contains("High-risk apply is reserved internally"),
        "CLI help should explain high-risk apply as reserved"
    );

    let live_source = std::fs::read_to_string(root.join("stutter/src/autotune/commands/live.rs"))
        .expect("read live autotune command source");
    assert!(
        live_source.contains("pub mode: DaemonMode"),
        "live autotune command input should receive typed DaemonMode, not raw String"
    );
}
