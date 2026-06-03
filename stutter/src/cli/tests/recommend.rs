use std::path::PathBuf;

use super::helpers::parse_report_command;
use crate::commands::AppCommand;

#[test]
fn parses_recommend_command_with_repeated_baselines_and_outputs() {
    let command = parse_report_command([
        "stutter",
        "recommend",
        "--baseline",
        "/tmp/base-1",
        "/tmp/base-2",
        "--tune",
        "/tmp/tune",
        "--fix-plan",
        "/tmp/fix-plan.json",
        "--markdown",
        "/tmp/recommendation.md",
        "--html",
        "/tmp/recommendation.html",
        "--allow-scenario-mismatch",
    ])
    .unwrap();

    let AppCommand::Recommend(input) = command else {
        panic!("expected recommend command");
    };

    assert_eq!(
        input.baseline,
        vec![PathBuf::from("/tmp/base-1"), PathBuf::from("/tmp/base-2")]
    );
    assert_eq!(input.tune, PathBuf::from("/tmp/tune"));
    assert_eq!(input.fix_plan, Some(PathBuf::from("/tmp/fix-plan.json")));
    assert!(input.allow_scenario_mismatch);
    assert_eq!(
        input.markdown,
        Some(PathBuf::from("/tmp/recommendation.md"))
    );
    assert_eq!(input.html, Some(PathBuf::from("/tmp/recommendation.html")));
}
