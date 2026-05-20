//! Tests for monitor auto-focus CLI defaults and parsing.
//!
//! Owns auto-focus argument regression tests. Does not own production monitor CLI defaults or
//! validation.

use super::*;

#[test]
fn monitor_args_default_contains_auto_focus_defaults() {
    let args = MonitorArgs::default();

    assert!(!args.auto_focus);
    assert_eq!(args.auto_focus_poll_ms, 1000);
    assert_eq!(args.auto_focus_min_confidence, 0.60);
    assert_eq!(args.auto_focus_switch_cooldown_ms, 5000);
    assert_eq!(args.auto_focus_switch_margin, 0.20);
    assert_eq!(args.auto_focus_required_polls, 2);
    assert_eq!(args.auto_focus_max_roots, 4);
}

#[test]
fn monitor_cli_parses_auto_focus_fields() {
    let cli = Cli::parse_from([
        "stutter",
        "monitor",
        "--auto-focus",
        "--auto-focus-poll-ms",
        "250",
        "--auto-focus-min-confidence",
        "0.75",
        "--auto-focus-switch-cooldown-ms",
        "7500",
        "--auto-focus-switch-margin",
        "0.30",
        "--auto-focus-required-polls",
        "3",
        "--auto-focus-max-roots",
        "2",
    ]);

    let Command::Monitor(args) = cli.command.unwrap() else {
        panic!("expected monitor command");
    };

    assert!(args.auto_focus);
    assert_eq!(args.auto_focus_poll_ms, 250);
    assert_eq!(args.auto_focus_min_confidence, 0.75);
    assert_eq!(args.auto_focus_switch_cooldown_ms, 7500);
    assert_eq!(args.auto_focus_switch_margin, 0.30);
    assert_eq!(args.auto_focus_required_polls, 3);
    assert_eq!(args.auto_focus_max_roots, 2);
}
