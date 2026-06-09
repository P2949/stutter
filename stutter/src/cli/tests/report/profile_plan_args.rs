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
    assert_eq!(input.profile_name, None);
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
    assert_eq!(input.profile_name, None);
    assert_eq!(input.output, Some(PathBuf::from("/tmp/profile-plan.json")));
}

#[test]
fn apply_profile_accepts_profile_name() {
    let command = parse_report_command([
        "stutter",
        "apply-profile",
        "--tree-pid",
        "123",
        "--profile",
        "profiles.toml",
        "--profile-name",
        "tuned",
        "--dry-run",
        "--explain",
    ])
    .unwrap();

    let AppCommand::ApplyProfile(input) = command else {
        panic!("expected apply profile command");
    };

    assert_eq!(input.profile_name, Some("tuned".to_owned()));
    assert!(input.dry_run);
    assert!(input.explain);
}

#[test]
fn apply_profile_rejects_explain_without_dry_run() {
    let err = parse_report_command([
        "stutter",
        "apply-profile",
        "--tree-pid",
        "42",
        "--profile",
        "/tmp/profile.toml",
        "--explain",
    ])
    .unwrap_err();

    assert!(err.to_string().contains("--explain requires --dry-run"));
}

#[test]
fn parses_profile_plan_command_with_defaults() {
    let command = parse_report_command([
        "stutter",
        "profile-plan",
        "--tree-pid",
        "123",
        "--profile",
        "profiles.toml",
    ])
    .unwrap();

    let AppCommand::ProfilePlan(input) = command else {
        panic!("expected profile-plan command");
    };

    assert_eq!(input.tree_pid, 123);
    assert_eq!(input.profile, PathBuf::from("profiles.toml"));
    assert_eq!(input.profile_name, None);
    assert!(!input.json);
    assert_eq!(input.output, None);
    assert_eq!(input.top, 10);
    assert!(input.highlight_comm.is_empty());
}

#[test]
fn parses_profile_plan_command_with_json_output_and_highlight() {
    let command = parse_report_command([
        "stutter",
        "profile-plan",
        "--tree-pid",
        "123",
        "--profile",
        "profiles.toml",
        "--profile-name",
        "tuned",
        "--json",
        "--output",
        "/tmp/profile-plan.json",
        "--top",
        "20",
        "--highlight-comm",
        "RenderThread",
    ])
    .unwrap();

    let AppCommand::ProfilePlan(input) = command else {
        panic!("expected profile-plan command");
    };

    assert_eq!(input.tree_pid, 123);
    assert_eq!(input.profile, PathBuf::from("profiles.toml"));
    assert_eq!(input.profile_name, Some("tuned".to_owned()));
    assert!(input.json);
    assert_eq!(input.output, Some(PathBuf::from("/tmp/profile-plan.json")));
    assert_eq!(input.top, 20);
    assert_eq!(input.highlight_comm, vec!["RenderThread"]);
}

#[test]
fn profile_plan_accepts_profile_name() {
    let command = parse_report_command([
        "stutter",
        "profile-plan",
        "--tree-pid",
        "123",
        "--profile",
        "profiles.toml",
        "--profile-name",
        "tuned",
    ])
    .unwrap();

    let AppCommand::ProfilePlan(input) = command else {
        panic!("expected profile-plan command");
    };

    assert_eq!(input.profile_name, Some("tuned".to_owned()));
}
