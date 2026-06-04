use std::path::PathBuf;

use super::helpers::parse_report_command;
use crate::commands::input::AppCommand;

#[test]
fn apply_profile_accepts_dry_run_explain() {
    let command = parse_report_command([
        "stutter",
        "apply-profile",
        "--tree-pid",
        "42",
        "--profile",
        "/tmp/profile.toml",
        "--dry-run",
        "--explain",
        "--top",
        "5",
        "--highlight-comm",
        "RenderThread",
    ])
    .unwrap();

    let AppCommand::ApplyProfile(input) = command else {
        panic!("expected apply profile command");
    };

    assert!(input.dry_run);
    assert!(input.explain);
    assert!(!input.json);
    assert_eq!(input.top, 5);
    assert_eq!(input.highlight_comm, vec!["RenderThread"]);
}

#[test]
fn apply_profile_accepts_dry_run_explain_json() {
    let command = parse_report_command([
        "stutter",
        "apply-profile",
        "--tree-pid",
        "42",
        "--profile",
        "/tmp/profile.toml",
        "--dry-run",
        "--explain",
        "--json",
        "--output",
        "/tmp/profile-plan.json",
    ])
    .unwrap();

    let AppCommand::ApplyProfile(input) = command else {
        panic!("expected apply profile command");
    };

    assert!(input.explain);
    assert!(input.json);
    assert_eq!(input.output, Some(PathBuf::from("/tmp/profile-plan.json")));
}

#[test]
fn parses_profile_plan_command() {
    let command = parse_report_command([
        "stutter",
        "profile-plan",
        "--tree-pid",
        "42",
        "--profile",
        "/tmp/profile.toml",
        "--json",
        "--output",
        "/tmp/profile-plan.json",
        "--top",
        "4",
        "--highlight-comm",
        "dxvk",
    ])
    .unwrap();

    let AppCommand::ProfilePlan(input) = command else {
        panic!("expected profile-plan command");
    };

    assert_eq!(input.tree_pid, 42);
    assert_eq!(input.profile, PathBuf::from("/tmp/profile.toml"));
    assert!(input.json);
    assert_eq!(input.output, Some(PathBuf::from("/tmp/profile-plan.json")));
    assert_eq!(input.top, 4);
    assert_eq!(input.highlight_comm, vec!["dxvk"]);
}
