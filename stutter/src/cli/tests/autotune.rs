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
    let err = validate_autotune_mode("apply-low-risk", false, true)
        .expect_err("dry-run-all-safe outside suggest mode should fail");

    assert!(err.to_string().contains("--dry-run-all-safe"));
}
