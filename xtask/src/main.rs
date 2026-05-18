use std::{
    env,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
};

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};

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

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    run(cli.command)
}

fn run(command: XtaskCommand) -> anyhow::Result<()> {
    let root = repo_root()?;
    match command {
        XtaskCommand::Ci => run_ci(&root),
        XtaskCommand::Fmt => run_process(&root, "cargo", &["fmt", "--check"]),
        XtaskCommand::Clippy => run_process(
            &root,
            "cargo",
            &["clippy", "--all-targets", "--", "-D", "warnings"],
        ),
        XtaskCommand::Smoke => run_smoke(&root),
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

fn run_ci(root: &Path) -> anyhow::Result<()> {
    run_script(root, "scripts/smoke/build.sh")?;
    run_script(root, "scripts/smoke/offline_recommendation.sh")?;
    run_script(root, "scripts/smoke/advisor_offline.sh")
}

fn run_smoke(root: &Path) -> anyhow::Result<()> {
    run_script(root, "scripts/smoke/build.sh")?;
    run_script(root, "scripts/smoke/offline_recommendation.sh")?;
    run_script(root, "scripts/smoke/advisor_offline.sh")
}

fn scaffold_only(name: &str) {
    println!("xtask {name}: scaffold only; workflow not wired yet");
}

fn run_script(root: &Path, script: &str) -> anyhow::Result<()> {
    run_process(root, "bash", &[script])
}

fn run_process(root: &Path, program: &str, args: &[&str]) -> anyhow::Result<()> {
    let status = ProcessCommand::new(program)
        .args(args)
        .current_dir(root)
        .status()
        .with_context(|| format!("failed to start `{}`", format_command(program, args)))?;

    if !status.success() {
        bail!(
            "command `{}` failed with status {status}",
            format_command(program, args)
        );
    }

    Ok(())
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

    use super::Cli;

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
}
