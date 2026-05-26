use std::path::PathBuf;

use crate::commands::input::AppCommand;

fn parse_validate_command<const N: usize>(args: [&str; N]) -> anyhow::Result<AppCommand> {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    crate::cli::parse_app_command_from(args)
}

#[test]
fn validate_requires_path() {
    let err = parse_validate_command(["stutter", "validate"]).unwrap_err();

    assert!(
        err.to_string().contains("required")
            || err.to_string().contains("PATH")
            || err.to_string().contains("path"),
        "expected missing path error, got {err:#}"
    );
}

#[test]
fn validate_accepts_path() {
    let command = parse_validate_command(["stutter", "validate", "/tmp/run"]).unwrap();

    let AppCommand::Validate(input) = command else {
        panic!("expected validate command");
    };

    assert_eq!(input.path, PathBuf::from("/tmp/run"));
    assert!(!input.json);
    assert!(!input.strict);
}

#[test]
fn validate_accepts_json_flag() {
    let command = parse_validate_command(["stutter", "validate", "--json", "/tmp/run"]).unwrap();

    let AppCommand::Validate(input) = command else {
        panic!("expected validate command");
    };

    assert_eq!(input.path, PathBuf::from("/tmp/run"));
    assert!(input.json);
    assert!(!input.strict);
}

#[test]
fn validate_accepts_strict_flag() {
    let command = parse_validate_command(["stutter", "validate", "--strict", "/tmp/run"]).unwrap();

    let AppCommand::Validate(input) = command else {
        panic!("expected validate command");
    };

    assert_eq!(input.path, PathBuf::from("/tmp/run"));
    assert!(!input.json);
    assert!(input.strict);
}

#[test]
fn validate_accepts_json_and_strict_flags() {
    let command =
        parse_validate_command(["stutter", "validate", "--json", "--strict", "/tmp/run"]).unwrap();

    let AppCommand::Validate(input) = command else {
        panic!("expected validate command");
    };

    assert_eq!(input.path, PathBuf::from("/tmp/run"));
    assert!(input.json);
    assert!(input.strict);
}
