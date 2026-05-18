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
    #[command(name = "schema-check", about = "Scaffold for schema contract checks")]
    SchemaCheck,
    #[command(
        name = "fixture-check",
        about = "Scaffold for fixture validation checks"
    )]
    FixtureCheck,
    #[command(name = "fixture-update", about = "Scaffold for fixture regeneration")]
    FixtureUpdate,
    #[command(
        name = "report-golden-update",
        about = "Scaffold for report golden output updates"
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
        XtaskCommand::SchemaCheck => {
            scaffold_only("schema-check");
            Ok(())
        }
        XtaskCommand::FixtureCheck => {
            scaffold_only("fixture-check");
            Ok(())
        }
        XtaskCommand::FixtureUpdate => {
            scaffold_only("fixture-update");
            Ok(())
        }
        XtaskCommand::ReportGoldenUpdate => {
            scaffold_only("report-golden-update");
            Ok(())
        }
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

    use super::{CI_COMMANDS, Cli, SMOKE_COMMANDS, format_command};

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
        let commands = CI_COMMANDS
            .iter()
            .map(|command| format_command(command.program, command.args))
            .collect::<Vec<_>>();

        assert_eq!(
            commands,
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
        let commands = SMOKE_COMMANDS
            .iter()
            .map(|command| format_command(command.program, command.args))
            .collect::<Vec<_>>();

        assert_eq!(
            commands,
            vec![
                "bash scripts/smoke/build.sh",
                "bash scripts/smoke/offline_recommendation.sh",
                "bash scripts/smoke/advisor_offline.sh",
            ]
        );
    }
}
