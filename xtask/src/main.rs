use std::{env, path::PathBuf};
use anyhow::{Context, bail};
use clap::{Parser, Subcommand};

pub mod process;
pub mod workflow;
pub mod no_allow_attrs;
pub mod dependency_hygiene;
pub mod ebpf_smoke;
pub mod fixtures;
pub mod preflight;

use crate::workflow::{CommandSpec, run_workflow, run_command_specs};
use crate::process::run_cargo;
use crate::no_allow_attrs::run_no_allow_attrs;
use crate::dependency_hygiene::run_dependency_hygiene;
use crate::ebpf_smoke::{run_privileged_ebpf_smoke, EBPF_BUILD_COMMAND};
use crate::fixtures::{SCHEMA_CHECK_WORKFLOW, FIXTURE_CHECK_WORKFLOW, FIXTURE_UPDATE_WORKFLOW, REPORT_GOLDEN_UPDATE_WORKFLOW};
use crate::preflight::run_preflight;

#[derive(Debug, Parser)]
#[command(name = "xtask", about = "Stutter development workflow tasks")]
pub struct Cli {
    #[command(subcommand)]
    pub command: XtaskCommand,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Subcommand)]
pub enum XtaskCommand {
    #[command(about = "Run the non-root CI workflow used by local development")]
    Ci,
    #[command(about = "Check Rust formatting")]
    Fmt,
    #[command(about = "Run clippy with repository warning policy")]
    Clippy,
    #[command(about = "Run non-root smoke workflow scripts")]
    Smoke,
    #[command(
        name = "preflight",
        about = "Check local toolchain prerequisites before building or running stutter"
    )]
    Preflight,
    #[command(
        name = "ebpf-smoke",
        about = "Run gated eBPF load smoke tests that require Linux tracefs and eBPF privileges"
    )]
    EbpfSmoke,
    #[command(
        name = "privileged-ebpf-smoke",
        about = "Compatibility alias for ebpf-smoke"
    )]
    PrivilegedEbpfSmoke,
    #[command(
        name = "validate",
        about = "Run the complete non-root validation gate, including an explicit eBPF build"
    )]
    Validate,
    #[command(
        name = "dependency-hygiene",
        about = "Run dependency advisory, license, source, and duplicate dependency checks"
    )]
    DependencyHygiene,
    #[command(
        name = "schema-check",
        about = "Validate generated artifact schema/example contracts"
    )]
    SchemaCheck,
    #[command(
        name = "no-allow-attrs",
        about = "Reject Rust allow attributes in repository source"
    )]
    NoAllowAttrs,
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

pub const CI_COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        program: "cargo",
        args: &["fmt", "--check"],
    },
    CommandSpec {
        program: "cargo",
        args: &["run", "-p", "xtask", "--", "no-allow-attrs"],
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

pub const VALIDATION_COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        program: "cargo",
        args: &["fmt", "--check"],
    },
    CommandSpec {
        program: "cargo",
        args: &["run", "-p", "xtask", "--", "no-allow-attrs"],
    },
    CommandSpec {
        program: "cargo",
        args: &["build"],
    },
    EBPF_BUILD_COMMAND,
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

pub const SMOKE_COMMANDS: &[CommandSpec] = &[
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
        XtaskCommand::Preflight => run_preflight(),
        XtaskCommand::EbpfSmoke => run_privileged_ebpf_smoke(&root),
        XtaskCommand::PrivilegedEbpfSmoke => run_privileged_ebpf_smoke(&root),
        XtaskCommand::Validate => {
            run_preflight()?;
            run_command_specs(&root, VALIDATION_COMMANDS)
        }
        XtaskCommand::DependencyHygiene => run_dependency_hygiene(&root),
        XtaskCommand::SchemaCheck => run_workflow(&root, SCHEMA_CHECK_WORKFLOW),
        XtaskCommand::NoAllowAttrs => run_no_allow_attrs(&root),
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

fn scaffold_only(name: &str) {
    println!("xtask {name}: scaffold only; workflow not wired yet");
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

    use crate::dependency_hygiene::{APPROVED_DUPLICATE_PACKAGES, DEPENDENCY_HYGIENE_COMMANDS, DEPENDENCY_HYGIENE_WORKFLOW, DUPLICATE_DEPENDENCY_COMMAND, duplicate_package_names};
    use crate::ebpf_smoke::PRIVILEGED_EBPF_SMOKE_COMMANDS;
    use crate::fixtures::{SCHEMA_CHECK_COMMANDS, SCHEMA_CHECK_WORKFLOW, FIXTURE_CHECK_COMMANDS, FIXTURE_CHECK_WORKFLOW, FIXTURE_UPDATE_COMMANDS, FIXTURE_UPDATE_WORKFLOW, REPORT_GOLDEN_UPDATE_COMMANDS, REPORT_GOLDEN_UPDATE_WORKFLOW};
    use crate::no_allow_attrs::scan_allow_attribute_file;
    use crate::process::format_command;

    use super::{
        CI_COMMANDS, Cli, SMOKE_COMMANDS, VALIDATION_COMMANDS
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
                "dependency-hygiene",
                "ebpf-smoke",
                "fixture-check",
                "fixture-update",
                "fmt",
                "generate-completions",
                "generate-man",
                "no-allow-attrs",
                "package",
                "preflight",
                "privileged-ebpf-smoke",
                "report-golden-update",
                "schema-check",
                "smoke",
                "validate",
            ]
        );
    }

    #[test]
    fn ci_command_order_matches_local_validation_flow() {
        assert_eq!(
            command_texts(CI_COMMANDS),
            vec![
                "cargo fmt --check",
                "cargo run -p xtask -- no-allow-attrs",
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
    fn validate_command_runs_explicit_ebpf_build_before_lints_and_tests() {
        assert_eq!(
            command_texts(VALIDATION_COMMANDS),
            vec![
                "cargo fmt --check",
                "cargo run -p xtask -- no-allow-attrs",
                "cargo build",
                "cargo build -p stutter",
                "cargo clippy --all-targets -- -D warnings",
                "cargo test",
                "bash scripts/smoke/offline_recommendation.sh",
                "bash scripts/smoke/advisor_offline.sh",
                "cargo test -p stutter architecture_tests",
            ]
        );
    }

    #[test]
    fn privileged_ebpf_smoke_command_targets_gated_loader_suite() {
        assert_eq!(
            command_texts(PRIVILEGED_EBPF_SMOKE_COMMANDS),
            vec![
                "cargo build -p stutter",
                "cargo test -p stutter privileged_ -- --nocapture",
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
    fn dependency_hygiene_runs_deny_and_duplicate_dependency_checks() {
        assert_eq!(
            command_texts(DEPENDENCY_HYGIENE_COMMANDS),
            vec!["cargo deny check"]
        );
        assert_eq!(
            format_command(
                DUPLICATE_DEPENDENCY_COMMAND.program,
                DUPLICATE_DEPENDENCY_COMMAND.args
            ),
            "cargo tree -d"
        );
        assert_eq!(
            DEPENDENCY_HYGIENE_WORKFLOW.affected_paths,
            &[
                "Cargo.toml",
                "Cargo.lock",
                "deny.toml",
                "*/Cargo.toml",
                "xtask/src/main.rs",
            ]
        );
    }

    #[test]
    fn duplicate_dependency_parser_reads_top_level_cargo_tree_entries() {
        let output = "\
bitflags v1.3.2
└── example v0.1.0

bitflags v2.11.1
└── other v0.1.0

syn v1.0.109
└── proc-macro-helper v0.1.0

syn v2.0.117
└── proc-macro-helper v0.2.0
";

        assert_eq!(
            duplicate_package_names(output),
            vec!["bitflags".to_owned(), "syn".to_owned()]
        );
    }

    #[test]
    fn approved_duplicate_package_list_records_current_baseline() {
        assert!(APPROVED_DUPLICATE_PACKAGES.contains(&"bitflags"));
        assert!(APPROVED_DUPLICATE_PACKAGES.contains(&"getrandom"));
        assert!(APPROVED_DUPLICATE_PACKAGES.contains(&"syn"));
        assert!(APPROVED_DUPLICATE_PACKAGES.contains(&"windows-sys"));
    }

    #[test]
    fn allow_attribute_scanner_rejects_direct_and_cfg_attr_suppressions() {
        let root = std::env::temp_dir().join(format!("stutter-allow-scan-{}", std::process::id()));
        let nested = root.join("src");
        std::fs::create_dir_all(&nested).expect("create temp scanner fixture directory");
        let source = nested.join("lib.rs");
        let source_content = concat!(
            "#![deny(warnings)]\n",
            "#![",
            "allow(dead_code)]\n",
            "#[",
            "allow(unused_imports)]\n",
            "#[cfg_attr(test, ",
            "allow",
            "(dead_code))]\n",
            "pub fn live() {}\n",
        );
        std::fs::write(&source, source_content).expect("write temp scanner fixture");

        let mut matches = Vec::new();
        scan_allow_attribute_file(&root, &source, &mut matches).expect("scan temp scanner fixture");

        assert_eq!(matches.len(), 3);
        assert_eq!(
            matches
                .iter()
                .map(|allow_match| allow_match.line)
                .collect::<Vec<_>>(),
            vec![2, 3, 4]
        );

        std::fs::remove_dir_all(root).expect("remove temp scanner fixture");
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
                "docs/examples/artifacts/v22/**",
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
                "stutter/src/validation_corpus_tests/",
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
                "cargo test -p stutter validation_corpus_tests::regenerate_public_examples_v22 -- --ignored --exact",
            ]
        );
        assert_eq!(
            FIXTURE_UPDATE_WORKFLOW.affected_paths,
            &[
                "stutter/tests/fixtures/runs/**",
                "docs/examples/artifacts/v22/**",
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

    fn command_texts(commands: &[crate::workflow::CommandSpec]) -> Vec<String> {
        commands
            .iter()
            .map(|command| format_command(command.program, command.args))
            .collect::<Vec<_>>()
    }
}
