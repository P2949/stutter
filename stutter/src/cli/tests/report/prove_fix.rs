use std::path::PathBuf;

use super::helpers::parse_report_command;
use crate::commands::AppCommand;

#[test]
fn parses_prove_fix_command_with_identity_and_counts() {
    let command = parse_report_command([
        "stutter",
        "prove-fix",
        "--plan",
        "/tmp/plan.json",
        "--profiles",
        "/tmp/profiles.toml",
        "--tree-pid",
        "123",
        "--scenario",
        " city-route ",
        "--workload-label",
        " Game.exe ",
        "--route-label",
        " city-loop ",
        "--duration",
        "180",
        "--baseline-runs",
        "5",
        "--test-runs",
        "4",
        "--baseline-profile",
        "baseline-online",
        "--html",
        "/tmp/fix-validation.html",
    ])
    .unwrap();

    let AppCommand::ProveFix(input) = command else {
        panic!("expected prove-fix command");
    };

    assert_eq!(input.plan, PathBuf::from("/tmp/plan.json"));
    assert_eq!(input.profiles, PathBuf::from("/tmp/profiles.toml"));
    assert_eq!(input.tree_pid, 123);
    assert_eq!(input.scenario_name.as_deref(), Some("city-route"));
    assert_eq!(input.workload_label.as_deref(), Some("Game.exe"));
    assert_eq!(input.route_label.as_deref(), Some("city-loop"));
    assert_eq!(input.duration_seconds, 180);
    assert_eq!(input.baseline_runs, Some(5));
    assert_eq!(input.test_runs, Some(4));
    assert_eq!(input.baseline_profile, "baseline-online");
    assert_eq!(input.html, PathBuf::from("/tmp/fix-validation.html"));
}

#[test]
fn prove_fix_rejects_zero_counts() {
    let err = parse_report_command([
        "stutter",
        "prove-fix",
        "--plan",
        "/tmp/plan.json",
        "--profiles",
        "/tmp/profiles.toml",
        "--tree-pid",
        "123",
        "--baseline-runs",
        "0",
    ])
    .unwrap_err();

    assert!(
        err.to_string()
            .contains("--baseline-runs must be greater than zero")
    );
}
