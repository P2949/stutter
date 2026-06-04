use clap::Parser;

use super::{super::*, helpers::parse_report_command};
use crate::cli::{Cli, Command};

#[test]
fn rules_check_requires_source_or_generated() {
    let result = Cli::try_parse_from(["stutter", "rules", "check"]);
    assert!(result.is_err());
}

#[test]
fn rules_import_requires_source() {
    let result = Cli::try_parse_from(["stutter", "rules", "import"]);
    assert!(result.is_err());
}

#[test]
fn rules_import_dry_run_does_not_write() {
    let cli = Cli::try_parse_from([
        "stutter",
        "rules",
        "import",
        "--source",
        "/tmp/ananicy-rules",
        "--dry-run",
    ])
    .unwrap();

    let command = cli.command.unwrap();
    match command {
        Command::Rules(args) => match args.command {
            RulesCommand::Import(import) => {
                assert!(import.dry_run);
                assert_eq!(import.out, None);
            }
            other => panic!("expected rules import command, got {other:?}"),
        },
        other => panic!("expected rules command, got {other:?}"),
    }
}

#[test]
fn rejects_zero_report_cluster_window() {
    let err =
        parse_report_command(["stutter", "report", "--cluster-ms", "0", "/tmp/run"]).unwrap_err();

    assert!(
        err.to_string()
            .contains("--cluster-ms must be greater than zero")
    );
}

#[test]
fn report_requires_path_unless_batch_is_set() {
    let err = parse_report_command(["stutter", "report"]).unwrap_err();

    assert!(
        err.to_string()
            .contains("report requires PATH unless --batch is set")
    );
}

#[test]
fn report_flag_conflicts() {
    assert!(
        parse_report_command(["stutter", "report", "--html", "r.html", "--batch", "dir"]).is_err()
    );
    assert!(
        parse_report_command(["stutter", "report", "--json", "--json-summary", "run"]).is_err()
    );
    assert!(
        parse_report_command(["stutter", "report", "--analysis-json", "--batch", "dir"]).is_err()
    );
    assert!(parse_report_command(["stutter", "report", "--batch", "dir", "run"]).is_err());
}

#[test]
fn rejects_zero_report_top() {
    let err = parse_report_command(["stutter", "report", "--top", "0", "/tmp/run"]).unwrap_err();

    assert!(err.to_string().contains("--top must be greater than zero"));
}

#[test]
fn rejects_summary_top_zero() {
    let err = parse_report_command(["stutter", "summary", "--top", "0", "/tmp/run"]).unwrap_err();

    assert!(err.to_string().contains("--top must be greater than zero"));
}

#[test]
fn apply_profile_rejects_zero_tree_pid() {
    let err = parse_report_command([
        "stutter",
        "apply-profile",
        "--tree-pid",
        "0",
        "--profile",
        "/tmp/profile.toml",
    ])
    .unwrap_err();

    assert!(
        err.to_string()
            .contains("--tree-pid must be greater than zero")
    );
}

#[test]
fn apply_profile_keep_applied_requires_watch() {
    let err = parse_report_command([
        "stutter",
        "apply-profile",
        "--tree-pid",
        "42",
        "--profile",
        "/tmp/profile.toml",
        "--keep-applied",
    ])
    .unwrap_err();

    assert!(err.to_string().contains("--keep-applied requires --watch"));
}

#[test]
fn apply_profile_rejects_zero_refresh_ms() {
    let err = parse_report_command([
        "stutter",
        "apply-profile",
        "--tree-pid",
        "42",
        "--profile",
        "/tmp/profile.toml",
        "--watch",
        "--refresh-ms",
        "0",
    ])
    .unwrap_err();

    assert!(
        err.to_string()
            .contains("--refresh-ms must be greater than zero")
    );
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

    assert!(
        err.to_string()
            .contains("--explain is only supported with --dry-run")
    );
}

#[test]
fn apply_profile_rejects_json_without_explain() {
    let err = parse_report_command([
        "stutter",
        "apply-profile",
        "--tree-pid",
        "42",
        "--profile",
        "/tmp/profile.toml",
        "--dry-run",
        "--json",
    ])
    .unwrap_err();

    assert!(err.to_string().contains("--json requires --explain"));
}

#[test]
fn apply_profile_rejects_output_without_explain() {
    let err = parse_report_command([
        "stutter",
        "apply-profile",
        "--tree-pid",
        "42",
        "--profile",
        "/tmp/profile.toml",
        "--dry-run",
        "--output",
        "/tmp/profile-plan.txt",
    ])
    .unwrap_err();

    assert!(err.to_string().contains("--output requires --explain"));
}

#[test]
fn apply_profile_rejects_explain_with_watch() {
    let err = parse_report_command([
        "stutter",
        "apply-profile",
        "--tree-pid",
        "42",
        "--profile",
        "/tmp/profile.toml",
        "--dry-run",
        "--explain",
        "--watch",
    ])
    .unwrap_err();

    assert!(
        err.to_string()
            .contains("--explain cannot be combined with --watch")
    );
}

#[test]
fn apply_profile_rejects_zero_top() {
    let err = parse_report_command([
        "stutter",
        "apply-profile",
        "--tree-pid",
        "42",
        "--profile",
        "/tmp/profile.toml",
        "--dry-run",
        "--explain",
        "--top",
        "0",
    ])
    .unwrap_err();

    assert!(err.to_string().contains("--top must be greater than zero"));
}

#[test]
fn profile_plan_rejects_zero_tree_pid() {
    let err = parse_report_command([
        "stutter",
        "profile-plan",
        "--tree-pid",
        "0",
        "--profile",
        "/tmp/profile.toml",
    ])
    .unwrap_err();

    assert!(
        err.to_string()
            .contains("--tree-pid must be greater than zero")
    );
}

#[test]
fn profile_plan_rejects_zero_top() {
    let err = parse_report_command([
        "stutter",
        "profile-plan",
        "--tree-pid",
        "42",
        "--profile",
        "/tmp/profile.toml",
        "--top",
        "0",
    ])
    .unwrap_err();

    assert!(err.to_string().contains("--top must be greater than zero"));
}

#[test]
fn tune_rejects_invalid_values() {
    for (args, expected) in [
        (
            vec![
                "stutter",
                "tune",
                "--tree-pid",
                "0",
                "--profiles",
                "/tmp/profiles.toml",
            ],
            "--tree-pid must be greater than zero",
        ),
        (
            vec![
                "stutter",
                "tune",
                "--tree-pid",
                "42",
                "--profiles",
                "/tmp/profiles.toml",
                "--epoch-seconds",
                "0",
            ],
            "--epoch-seconds must be greater than zero",
        ),
        (
            vec![
                "stutter",
                "tune",
                "--tree-pid",
                "42",
                "--profiles",
                "/tmp/profiles.toml",
                "--epoch-seconds",
                "10",
                "--warmup-seconds",
                "10",
            ],
            "--warmup-seconds must be less than --epoch-seconds",
        ),
        (
            vec![
                "stutter",
                "tune",
                "--tree-pid",
                "42",
                "--profiles",
                "/tmp/profiles.toml",
                "--runs",
                "0",
            ],
            "--runs must be greater than zero",
        ),
    ] {
        let err = crate::cli::parse_app_command_from(args).unwrap_err();

        assert!(
            err.to_string().contains(expected),
            "expected error containing {expected:?}, got {err:#}"
        );
    }
}

#[test]
fn scenario_create_rejects_zero_duration() {
    let err = parse_report_command(["stutter", "scenario", "create", "test", "--duration", "0"])
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("scenario duration must be greater than zero")
    );
}

#[test]
fn scenario_run_rejects_bad_role() {
    let err = parse_report_command(["stutter", "scenario", "run", "test", "--role", "other"])
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("--role must be baseline or current")
    );
}

#[test]
fn scenario_compare_rejects_top_zero() {
    let err =
        parse_report_command(["stutter", "scenario", "compare", "test", "--top", "0"]).unwrap_err();

    assert!(err.to_string().contains("--top must be greater than zero"));
}
