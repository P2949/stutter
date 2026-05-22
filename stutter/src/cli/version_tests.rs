use super::*;

#[test]
fn clap_version_uses_build_version_metadata() {
    assert_eq!(
        Cli::command().get_version(),
        Some(crate::metadata::build_version())
    );
    assert_eq!(crate::metadata::build_git_rev(), env!("STUTTER_GIT_REV"));
}

#[test]
fn parse_version_features_request_bypasses_clap_version_exit() {
    let command = parse_app_command_from(["stutter", "--version", "--features"]).unwrap();

    let AppCommand::Version(input) = command else {
        panic!("expected version command");
    };

    assert!(input.features);
}

#[test]
fn nested_help_requests_have_display_help_kind() {
    let cases = [
        ["stutter", "daemon", "--help"].as_slice(),
        ["stutter", "daemon", "status", "--help"].as_slice(),
        ["stutter", "rules", "check", "--help"].as_slice(),
        ["stutter", "profile-template", "--help"].as_slice(),
    ];

    for argv in cases {
        let err = Cli::try_parse_from(argv)
            .expect_err("clap returns DisplayHelp for nested --help requests");

        assert_eq!(err.kind(), ErrorKind::DisplayHelp);
    }
}

#[test]
fn clap_top_level_command_tree_matches_snapshot() {
    let mut rendered = String::from("stutter\n");
    for subcommand in Cli::command().get_subcommands() {
        rendered.push_str(&format!("  {}\n", subcommand.get_name()));
    }

    assert_eq!(
        rendered,
        include_str!("../../tests/snapshots/clap_top_level_commands.txt")
    );
}
