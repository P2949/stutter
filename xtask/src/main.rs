use std::{
    env,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};

const DEFAULT_TOOLCHAIN: &str = "nightly";

#[derive(Debug, Parser)]
#[command(name = "xtask", about = "Stutter development workflow tasks")]
struct Cli {
    #[command(subcommand)]
    command: XtaskCommand,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Subcommand)]
enum XtaskCommand {
    #[command(about = "Run the non-root CI workflow used by local development")]
    Ci,
    #[command(about = "Check Rust formatting")]
    Fmt,
    #[command(about = "Run clippy with repository warning policy")]
    Clippy,
    #[command(about = "Run non-root smoke workflow scripts")]
    Smoke,
    #[command(
        name = "schema-check",
        about = "Validate generated artifact schema/example contracts"
    )]
    SchemaCheck,
    #[command(
        name = "fixture-check",
        about = "Validate committed validation corpus fixtures"
    )]
    FixtureCheck,
    #[command(
        name = "fixture-update",
        about = "Regenerate committed validation corpus and public example fixtures"
    )]
    FixtureUpdate,
    #[command(
        name = "report-golden-update",
        about = "Validate committed report text golden output fixture"
    )]
    ReportGoldenUpdate,
    #[command(name = "generate-man", about = "Scaffold for man page generation")]
    GenerateMan,
    #[command(
        name = "generate-completions",
        about = "Scaffold for shell completion generation"
    )]
    GenerateCompletions,
    #[command(about = "Scaffold for package build workflow")]
    Package,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CommandSpec {
    program: &'static str,
    args: &'static [&'static str],
}

#[derive(Clone, Copy, Debug)]
struct WorkflowSpec {
    name: &'static str,
    description: &'static str,
    affected_paths: &'static [&'static str],
    commands: &'static [CommandSpec],
}

const CI_COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        program: "cargo",
        args: &["fmt", "--check"],
    },
    CommandSpec {
        program: "cargo",
        args: &["build"],
    },
    CommandSpec {
        program: "cargo",
        args: &["clippy", "--all-targets", "--", "-D", "warnings"],
    },
    CommandSpec {
        program: "cargo",
        args: &["test"],
    },
    CommandSpec {
        program: "bash",
        args: &["scripts/smoke/offline_recommendation.sh"],
    },
    CommandSpec {
        program: "bash",
        args: &["scripts/smoke/advisor_offline.sh"],
    },
    CommandSpec {
        program: "cargo",
        args: &["test", "-p", "stutter", "architecture_tests"],
    },
];

const SMOKE_COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        program: "bash",
        args: &["scripts/smoke/build.sh"],
    },
    CommandSpec {
        program: "bash",
        args: &["scripts/smoke/offline_recommendation.sh"],
    },
    CommandSpec {
        program: "bash",
        args: &["scripts/smoke/advisor_offline.sh"],
    },
];

const SCHEMA_CHECK_COMMANDS: &[CommandSpec] = &[CommandSpec {
    program: "cargo",
    args: &["test", "-p", "stutter", "artifact_contract_tests"],
}];

const FIXTURE_CHECK_COMMANDS: &[CommandSpec] = &[CommandSpec {
    program: "cargo",
    args: &["test", "-p", "stutter", "validation_corpus"],
}];

const FIXTURE_UPDATE_COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        program: "cargo",
        args: &[
            "test",
            "-p",
            "stutter",
            "validation_corpus_tests::regenerate_validation_corpus",
            "--",
            "--ignored",
            "--exact",
        ],
    },
    CommandSpec {
        program: "cargo",
        args: &[
            "test",
            "-p",
            "stutter",
            "validation_corpus_tests::regenerate_public_examples_v21",
            "--",
            "--ignored",
            "--exact",
        ],
    },
];

const REPORT_GOLDEN_UPDATE_COMMANDS: &[CommandSpec] = &[CommandSpec {
    program: "cargo",
    args: &[
        "test",
        "-p",
        "stutter",
        "report::tests::report_text_rendering_matches_snapshot_fixture",
        "--",
        "--exact",
    ],
}];

const SCHEMA_CHECK_WORKFLOW: WorkflowSpec = WorkflowSpec {
    name: "schema-check",
    description: "validates artifact contract tests and public example artifact schema expectations",
    affected_paths: &[
        "stutter/src/artifact_contract_tests.rs",
        "docs/examples/artifacts/v21/**",
    ],
    commands: SCHEMA_CHECK_COMMANDS,
};

const FIXTURE_CHECK_WORKFLOW: WorkflowSpec = WorkflowSpec {
    name: "fixture-check",
    description: "validates committed validation corpus fixtures and fixture metadata",
    affected_paths: &[
        "stutter/src/validation_corpus_tests.rs",
        "stutter/tests/fixtures/runs/**",
    ],
    commands: FIXTURE_CHECK_COMMANDS,
};

const FIXTURE_UPDATE_WORKFLOW: WorkflowSpec = WorkflowSpec {
    name: "fixture-update",
    description: "updates validation corpus fixtures and public v21 example artifact fixtures",
    affected_paths: &[
        "stutter/tests/fixtures/runs/**",
        "docs/examples/artifacts/v21/**",
    ],
    commands: FIXTURE_UPDATE_COMMANDS,
};

const REPORT_GOLDEN_UPDATE_WORKFLOW: WorkflowSpec = WorkflowSpec {
    name: "report-golden-update",
    description: "validates the committed report text golden output fixture",
    affected_paths: &[
        "stutter/src/report/snapshots/text_report_minimal.snap",
        "stutter/src/report/mod.rs",
    ],
    commands: REPORT_GOLDEN_UPDATE_COMMANDS,
};

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    run(cli.command)
}

fn run(command: XtaskCommand) -> anyhow::Result<()> {
    let root = repo_root()?;
    match command {
        XtaskCommand::Ci => run_command_specs(&root, CI_COMMANDS),
        XtaskCommand::Fmt => run_cargo(&root, &["fmt", "--check"]),
        XtaskCommand::Clippy => {
            run_cargo(&root, &["clippy", "--all-targets", "--", "-D", "warnings"])
        }
        XtaskCommand::Smoke => run_command_specs(&root, SMOKE_COMMANDS),
        XtaskCommand::SchemaCheck => run_workflow(&root, SCHEMA_CHECK_WORKFLOW),
        XtaskCommand::FixtureCheck => run_workflow(&root, FIXTURE_CHECK_WORKFLOW),
        XtaskCommand::FixtureUpdate => run_workflow(&root, FIXTURE_UPDATE_WORKFLOW),
        XtaskCommand::ReportGoldenUpdate => run_workflow(&root, REPORT_GOLDEN_UPDATE_WORKFLOW),
        XtaskCommand::GenerateMan => {
            scaffold_only("generate-man");
            Ok(())
        }
        XtaskCommand::GenerateCompletions => {
            scaffold_only("generate-completions");
            Ok(())
        }
        XtaskCommand::Package => {
            scaffold_only("package");
            Ok(())
        }
    }
}

fn run_workflow(root: &Path, workflow: WorkflowSpec) -> anyhow::Result<()> {
    println!("xtask {}: {}", workflow.name, workflow.description);
    println!("xtask {} affected paths:", workflow.name);
    for path in workflow.affected_paths {
        println!("  - {path}");
    }

    run_command_specs(root, workflow.commands).with_context(|| {
        format!(
            "xtask {} failed while processing affected paths: {}",
            workflow.name,
            workflow.affected_paths.join(", ")
        )
    })
}

fn run_command_specs(root: &Path, commands: &[CommandSpec]) -> anyhow::Result<()> {
    for command in commands {
        run_process(root, command.program, command.args)?;
    }
    Ok(())
}

fn scaffold_only(name: &str) {
    println!("xtask {name}: scaffold only; workflow not wired yet");
}

fn run_cargo(root: &Path, args: &[&str]) -> anyhow::Result<()> {
    run_process(root, "cargo", args)
}

fn run_process(root: &Path, program: &str, args: &[&str]) -> anyhow::Result<()> {
    let command_text = format_command(program, args);
    println!("--- STAGE: {command_text} ---");

    let mut command = ProcessCommand::new(program);
    command
        .args(args)
        .current_dir(root)
        .env("RUSTUP_TOOLCHAIN", rustup_toolchain());

    let status = command
        .status()
        .with_context(|| format!("failed to start `{command_text}`"))?;

    if !status.success() {
        bail!("command `{command_text}` failed with status {status}");
    }

    Ok(())
}

fn rustup_toolchain() -> String {
    env::var("RUSTUP_TOOLCHAIN").unwrap_or_else(|_| DEFAULT_TOOLCHAIN.to_owned())
}

fn format_command(program: &str, args: &[&str]) -> String {
    std::iter::once(program)
        .chain(args.iter().copied())
        .collect::<Vec<_>>()
        .join(" ")
}

fn repo_root() -> anyhow::Result<PathBuf> {
    let mut dir = env::current_dir().context("failed to read current directory")?;
    loop {
        if dir.join("Cargo.toml").is_file()
            && dir.join("stutter/Cargo.toml").is_file()
            && dir.join("scripts/smoke/build.sh").is_file()
        {
            return Ok(dir);
        }

        if !dir.pop() {
            bail!("failed to locate stutter repository root from current directory");
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::{
        CI_COMMANDS, Cli, FIXTURE_CHECK_COMMANDS, FIXTURE_UPDATE_COMMANDS,
        REPORT_GOLDEN_UPDATE_COMMANDS, SCHEMA_CHECK_COMMANDS, SMOKE_COMMANDS,
        FIXTURE_CHECK_WORKFLOW, FIXTURE_UPDATE_WORKFLOW, REPORT_GOLDEN_UPDATE_WORKFLOW,
        SCHEMA_CHECK_WORKFLOW, format_command,
    };

    #[test]
    fn expected_subcommands_are_registered() {
        let mut names = Cli::command()
            .get_subcommands()
            .map(|command| command.get_name().to_owned())
            .collect::<Vec<_>>();
        names.sort();

        assert_eq!(
            names,
            vec![
                "ci",
                "clippy",
                "fixture-check",
                "fixture-update",
                "fmt",
                "generate-completions",
                "generate-man",
                "package",
                "report-golden-update",
                "schema-check",
                "smoke",
            ]
        );
    }

    #[test]
    fn ci_command_order_matches_local_validation_flow() {
        assert_eq!(
            command_texts(CI_COMMANDS),
            vec![
                "cargo fmt --check",
                "cargo build",
                "cargo clippy --all-targets -- -D warnings",
                "cargo test",
                "bash scripts/smoke/offline_recommendation.sh",
                "bash scripts/smoke/advisor_offline.sh",
                "cargo test -p stutter architecture_tests",
            ]
        );
    }

    #[test]
    fn smoke_command_keeps_existing_smoke_script_flow() {
        assert_eq!(
            command_texts(SMOKE_COMMANDS),
            vec![
                "bash scripts/smoke/build.sh",
                "bash scripts/smoke/offline_recommendation.sh",
                "bash scripts/smoke/advisor_offline.sh",
            ]
        );
    }

    #[test]
    fn schema_check_runs_artifact_contract_gate() {
        assert_eq!(
            command_texts(SCHEMA_CHECK_COMMANDS),
            vec!["cargo test -p stutter artifact_contract_tests"]
        );
        assert_eq!(
            SCHEMA_CHECK_WORKFLOW.affected_paths,
            &[
                "stutter/src/artifact_contract_tests.rs",
                "docs/examples/artifacts/v21/**",
            ]
        );
    }

    #[test]
    fn fixture_check_runs_validation_corpus_gate() {
        assert_eq!(
            command_texts(FIXTURE_CHECK_COMMANDS),
            vec!["cargo test -p stutter validation_corpus"]
        );
        assert_eq!(
            FIXTURE_CHECK_WORKFLOW.affected_paths,
            &[
                "stutter/src/validation_corpus_tests.rs",
                "stutter/tests/fixtures/runs/**",
            ]
        );
    }

    #[test]
    fn fixture_update_runs_existing_ignored_fixture_generators() {
        assert_eq!(
            command_texts(FIXTURE_UPDATE_COMMANDS),
            vec![
                "cargo test -p stutter validation_corpus_tests::regenerate_validation_corpus -- --ignored --exact",
                "cargo test -p stutter validation_corpus_tests::regenerate_public_examples_v21 -- --ignored --exact",
            ]
        );
        assert_eq!(
            FIXTURE_UPDATE_WORKFLOW.affected_paths,
            &[
                "stutter/tests/fixtures/runs/**",
                "docs/examples/artifacts/v21/**",
            ]
        );
    }

    #[test]
    fn report_golden_update_runs_report_text_snapshot_gate() {
        assert_eq!(
            command_texts(REPORT_GOLDEN_UPDATE_COMMANDS),
            vec![
                "cargo test -p stutter report::tests::report_text_rendering_matches_snapshot_fixture -- --exact",
            ]
        );
        assert_eq!(
            REPORT_GOLDEN_UPDATE_WORKFLOW.affected_paths,
            &[
                "stutter/src/report/snapshots/text_report_minimal.snap",
                "stutter/src/report/mod.rs",
            ]
        );
    }

    fn command_texts(commands: &[super::CommandSpec]) -> Vec<String> {
        commands
            .iter()
            .map(|command| format_command(command.program, command.args))
            .collect::<Vec<_>>()
    }
}
