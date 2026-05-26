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
        include_str!("../../../tests/snapshots/clap_top_level_commands.txt")
    );
}

#[test]
fn clap_help_output_matches_snapshots() {
    assert_help_snapshot(
        Cli::command(),
        include_str!("../../../tests/snapshots/clap_help_top_level.txt"),
    );
    assert_subcommand_help_snapshot(
        "monitor",
        include_str!("../../../tests/snapshots/clap_help_monitor.txt"),
    );
    assert_subcommand_help_snapshot(
        "daemon",
        include_str!("../../../tests/snapshots/clap_help_daemon.txt"),
    );
    assert_subcommand_help_snapshot(
        "autotune",
        include_str!("../../../tests/snapshots/clap_help_autotune.txt"),
    );
}

fn assert_subcommand_help_snapshot(name: &str, expected: &str) {
    let mut command = Cli::command();
    let subcommand = command
        .find_subcommand_mut(name)
        .unwrap_or_else(|| panic!("missing {name} subcommand"))
        .clone()
        .bin_name(format!("stutter {name}"));
    assert_help_snapshot(subcommand, expected);
}

fn assert_help_snapshot(command: clap::Command, expected: &str) {
    assert_eq!(render_help(command), expected);
}

fn render_help(mut command: clap::Command) -> String {
    let mut output = Vec::new();
    command
        .write_help(&mut output)
        .expect("clap can render help");
    String::from_utf8(output).expect("clap help is valid UTF-8")
}
