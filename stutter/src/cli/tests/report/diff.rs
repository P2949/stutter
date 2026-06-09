use std::path::PathBuf;

use super::helpers::parse_report_command;
use crate::commands::input::AppCommand;

#[test]
fn compare_display_path_parses_expectation_and_strict() {
    let command = parse_report_command([
        "stutter",
        "compare",
        "display-path",
        "--baseline",
        "/tmp/direct-run",
        "--test",
        "/tmp/uhd630-run",
        "--expect",
        "direct-to-offload",
        "--strict",
        "--json",
    ])
    .unwrap();

    let AppCommand::DisplayPathCompare(input) = command else {
        panic!("expected display-path compare command");
    };

    assert_eq!(input.baseline, PathBuf::from("/tmp/direct-run"));
    assert_eq!(input.test, PathBuf::from("/tmp/uhd630-run"));
    assert_eq!(
        input.expect,
        Some(crate::display_path_compare::DisplayPathExpectation::DirectToOffload)
    );
    assert!(input.strict);
    assert!(input.json);
}
