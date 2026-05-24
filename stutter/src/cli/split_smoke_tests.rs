//! Tests extracted from the parent module to keep production files below the architecture size gate.

use super::*;

#[test]
fn cli_split_preserves_monitor_parse_path() {
    let command = parse_app_command_from(["stutter", "monitor", "--pid", "1234"]).unwrap();
    assert!(matches!(command, AppCommand::Monitor(_)));
}

#[test]
fn cli_split_preserves_autotune_parse_path() {
    let command = parse_app_command_from([
        "stutter",
        "autotune",
        "--tree-pid",
        "1234",
        "--mode",
        "observe",
    ])
    .unwrap();
    assert!(matches!(command, AppCommand::Autotune(_)));
}

#[test]
fn cli_split_preserves_daemon_status_parse_path() {
    let command = parse_app_command_from(["stutter", "daemon", "status", "--json"]).unwrap();
    assert!(matches!(command, AppCommand::DaemonStatus(_)));
}

#[test]
fn cli_split_preserves_service_parse_path() {
    let command =
        parse_app_command_from(["stutter", "service", "doctor", "--mode", "user-observe"]).unwrap();
    assert!(matches!(command, AppCommand::Service(_)));
}

#[test]
fn cli_split_preserves_rules_parse_path() {
    let command = parse_app_command_from(["stutter", "rules", "list"]).unwrap();
    assert!(matches!(command, AppCommand::Rules(_)));
}

#[test]
fn cli_split_review_guard_covers_all_split_cli_modules() {
    struct CliSplitCase {
        argv: &'static [&'static str],
        matches_command: fn(AppCommand) -> bool,
    }

    let cases: &[CliSplitCase] = &[
        CliSplitCase {
            argv: &["stutter", "monitor", "--pid", "1234"],
            matches_command: |command| matches!(command, AppCommand::Monitor(_)),
        },
        CliSplitCase {
            argv: &[
                "stutter",
                "autotune",
                "--tree-pid",
                "1234",
                "--mode",
                "observe",
            ],
            matches_command: |command| matches!(command, AppCommand::Autotune(_)),
        },
        CliSplitCase {
            argv: &["stutter", "daemon", "status", "--json"],
            matches_command: |command| matches!(command, AppCommand::DaemonStatus(_)),
        },
        CliSplitCase {
            argv: &["stutter", "agent"],
            matches_command: |command| matches!(command, AppCommand::Agent(_)),
        },
        CliSplitCase {
            argv: &["stutter", "report", "/tmp/run"],
            matches_command: |command| matches!(command, AppCommand::Report(_)),
        },
        CliSplitCase {
            argv: &["stutter", "config", "check"],
            matches_command: |command| matches!(command, AppCommand::ConfigCheck(_)),
        },
        CliSplitCase {
            argv: &["stutter", "service", "doctor", "--mode", "user-observe"],
            matches_command: |command| matches!(command, AppCommand::Service(_)),
        },
        CliSplitCase {
            argv: &["stutter", "validate", "/tmp/run"],
            matches_command: |command| matches!(command, AppCommand::Validate(_)),
        },
    ];

    for case in cases {
        Cli::try_parse_from(case.argv)
            .unwrap_or_else(|err| panic!("Cli::try_parse_from failed for {:?}: {err}", case.argv));

        let command = parse_app_command_from(case.argv).unwrap_or_else(|err| {
            panic!("parse_app_command_from failed for {:?}: {err}", case.argv)
        });

        assert!(
            (case.matches_command)(command),
            "parsed command did not match expected AppCommand variant for {:?}",
            case.argv
        );
    }
}
