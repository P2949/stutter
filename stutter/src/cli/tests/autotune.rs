use super::*;

#[test]
fn autotune_cli_parses_washout_flags() {
    let cli = Cli::try_parse_from([
        "stutter",
        "autotune",
        "--washout-seconds",
        "30",
        "--washout-verify-interval-ms",
        "2000",
    ])
    .unwrap();

    let Some(Command::Autotune(args)) = cli.command else {
        panic!("expected autotune command");
    };

    assert_eq!(args.washout_seconds, 30);
    assert_eq!(args.washout_verify_interval_ms, 2_000);
    assert_eq!(
        args.min_focus_confidence,
        crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE
    );
}

#[test]
fn autotune_cli_parses_min_focus_confidence() {
    let cli =
        Cli::try_parse_from(["stutter", "autotune", "--min-focus-confidence", "0.42"]).unwrap();

    let Some(Command::Autotune(args)) = cli.command else {
        panic!("expected autotune command");
    };

    assert_eq!(args.min_focus_confidence, 0.42);
}

#[test]
fn autotune_cli_parses_high_risk_dry_run_flag() {
    let cli = Cli::try_parse_from(["stutter", "autotune", "--high-risk-dry-run"]).unwrap();

    let Some(Command::Autotune(args)) = cli.command else {
        panic!("expected autotune command");
    };

    assert!(args.high_risk_dry_run);
}

#[test]
fn autotune_cli_parses_dry_run_all_safe_flag() {
    let cli = Cli::try_parse_from([
        "stutter",
        "autotune",
        "--mode",
        "suggest",
        "--dry-run-all-safe",
    ])
    .unwrap();

    let Some(Command::Autotune(args)) = cli.command else {
        panic!("expected autotune command");
    };

    assert!(args.dry_run_all_safe);
}

#[test]
fn dry_run_all_safe_requires_suggest_mode() {
    let err = validate_autotune_mode(LiveAutotuneMode::ApplyLowRisk, false, true)
        .expect_err("dry-run-all-safe outside suggest mode should fail");

    assert!(err.to_string().contains("--dry-run-all-safe"));
}

#[test]
fn autotune_help_does_not_list_apply_high_risk_as_live_mode() {
    use clap::CommandFactory;

    let mut command = Cli::command();
    let autotune = command
        .find_subcommand_mut("autotune")
        .expect("autotune subcommand should exist");

    let mut output = Vec::new();
    autotune
        .write_help(&mut output)
        .expect("clap can render autotune help");

    let help = String::from_utf8(output).expect("help should be utf8");

    assert!(help.contains("apply-low-risk"));
    assert!(help.contains("apply-medium-risk"));
    assert!(
        !help.contains("or apply-high-risk"),
        "autotune help should not list apply-high-risk as a supported live mode:\n{help}"
    );
    assert!(
        help.contains("High-risk apply is reserved internally"),
        "autotune help should explain high-risk apply as reserved, not supported:\n{help}"
    );
}

#[test]
fn autotune_cli_rejects_apply_high_risk_at_parse_time() {
    let err = Cli::try_parse_from(["stutter", "autotune", "--mode", "apply-high-risk"])
        .expect_err("live autotune should not accept apply-high-risk mode");

    let message = err.to_string();

    assert!(
        message.contains("invalid value") || message.contains("possible values"),
        "unexpected clap error: {message}"
    );
    assert!(
        message.contains("apply-medium-risk"),
        "error should list supported live modes: {message}"
    );
}

#[test]
fn autotune_cli_still_parses_medium_risk_mode() {
    let cli = Cli::try_parse_from([
        "stutter",
        "autotune",
        "--mode",
        "apply-medium-risk",
        "--allow-medium-risk",
    ])
    .unwrap();

    let Some(Command::Autotune(args)) = cli.command else {
        panic!("expected autotune command");
    };

    assert_eq!(args.mode, LiveAutotuneMode::ApplyMediumRisk);
    assert!(args.allow_medium_risk);
}
