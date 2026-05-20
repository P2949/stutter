use std::{
    collections::BTreeSet,
    env, fs,
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

const DEPENDENCY_HYGIENE_COMMANDS: &[CommandSpec] = &[CommandSpec {
    program: "cargo",
    args: &["deny", "check"],
}];

const DUPLICATE_DEPENDENCY_COMMAND: CommandSpec = CommandSpec {
    program: "cargo",
    args: &["tree", "-d"],
};

const APPROVED_DUPLICATE_PACKAGES: &[&str] = &[
    "bitflags",
    "either",
    "foldhash",
    "getrandom",
    "hashbrown",
    "indexmap",
    "linux-raw-sys",
    "memchr",
    "r-efi",
    "rand",
    "rand_chacha",
    "rand_core",
    "rustix",
    "serde",
    "serde_core",
    "serde_json",
    "socket2",
    "stutter-common",
    "syn",
    "thiserror",
    "thiserror-impl",
    "tower",
    "which",
    "windows-sys",
    "wit-bindgen",
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
            "validation_corpus_tests::regenerate_public_examples_v22",
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

const DEPENDENCY_HYGIENE_WORKFLOW: WorkflowSpec = WorkflowSpec {
    name: "dependency-hygiene",
    description: "validates cargo-deny policy and rejects newly introduced duplicate dependency families",
    affected_paths: &[
        "Cargo.toml",
        "Cargo.lock",
        "deny.toml",
        "*/Cargo.toml",
        "xtask/src/main.rs",
    ],
    commands: DEPENDENCY_HYGIENE_COMMANDS,
};

const SCHEMA_CHECK_WORKFLOW: WorkflowSpec = WorkflowSpec {
    name: "schema-check",
    description: "validates artifact contract tests and public example artifact schema expectations",
    affected_paths: &[
        "stutter/src/artifact_contract_tests.rs",
        "docs/examples/artifacts/v22/**",
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
    description: "updates validation corpus fixtures and public v22 example artifact fixtures",
    affected_paths: &[
        "stutter/tests/fixtures/runs/**",
        "docs/examples/artifacts/v22/**",
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

fn run_no_allow_attrs(root: &Path) -> anyhow::Result<()> {
    let matches = find_allow_attributes(root)?;
    if matches.is_empty() {
        return Ok(());
    }

    println!("Rust allow attributes are forbidden:");
    for allow_match in matches {
        println!(
            "{}:{}: {}",
            allow_match.path.display(),
            allow_match.line,
            allow_match.text.trim()
        );
    }

    bail!("remove Rust allow attributes and fix the lint directly")
}

#[derive(Debug, Eq, PartialEq)]
struct AllowAttributeMatch {
    path: PathBuf,
    line: usize,
    text: String,
}

fn find_allow_attributes(root: &Path) -> anyhow::Result<Vec<AllowAttributeMatch>> {
    let mut matches = Vec::new();
    for source_dir in rust_source_roots(root) {
        collect_allow_attributes(root, &source_dir, &mut matches)?;
    }
    Ok(matches)
}

fn rust_source_roots(root: &Path) -> Vec<PathBuf> {
    [
        "stutter",
        "stutter-common",
        "stutter-config",
        "stutter-core",
        "stutter-ebpf",
        "stutter-report",
        "xtask",
    ]
    .into_iter()
    .map(|path| root.join(path))
    .collect()
}

fn collect_allow_attributes(
    root: &Path,
    dir: &Path,
    matches: &mut Vec<AllowAttributeMatch>,
) -> anyhow::Result<()> {
    if !dir.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry.with_context(|| format!("failed to read entry in {}", dir.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("failed to read file type for {}", path.display()))?;

        if file_type.is_dir() {
            collect_allow_attributes(root, &path, matches)?;
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            scan_allow_attribute_file(root, &path, matches)?;
        }
    }

    Ok(())
}

fn scan_allow_attribute_file(
    root: &Path,
    path: &Path,
    matches: &mut Vec<AllowAttributeMatch>,
) -> anyhow::Result<()> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let relative_path = path.strip_prefix(root).unwrap_or(path);

    for (line_index, line) in content.lines().enumerate() {
        let compact = line.split_whitespace().collect::<String>();
        if compact.contains("#![allow(")
            || compact.contains("#[allow(")
            || (compact.contains("cfg_attr(") && compact.contains("allow("))
        {
            matches.push(AllowAttributeMatch {
                path: relative_path.to_path_buf(),
                line: line_index + 1,
                text: line.to_owned(),
            });
        }
    }

    Ok(())
}

fn run_dependency_hygiene(root: &Path) -> anyhow::Result<()> {
    print_workflow_header(DEPENDENCY_HYGIENE_WORKFLOW);
    run_command_specs(root, DEPENDENCY_HYGIENE_WORKFLOW.commands)
        .with_context(|| workflow_failure_message(DEPENDENCY_HYGIENE_WORKFLOW))?;
    run_duplicate_dependency_check(root)
        .with_context(|| workflow_failure_message(DEPENDENCY_HYGIENE_WORKFLOW))
}

fn run_workflow(root: &Path, workflow: WorkflowSpec) -> anyhow::Result<()> {
    print_workflow_header(workflow);
    run_command_specs(root, workflow.commands).with_context(|| workflow_failure_message(workflow))
}

fn print_workflow_header(workflow: WorkflowSpec) {
    println!("xtask {}: {}", workflow.name, workflow.description);
    println!("xtask {} affected paths:", workflow.name);
    for path in workflow.affected_paths {
        println!("  - {path}");
    }
}

fn workflow_failure_message(workflow: WorkflowSpec) -> String {
    format!(
        "xtask {} failed while processing affected paths: {}",
        workflow.name,
        workflow.affected_paths.join(", ")
    )
}

fn run_command_specs(root: &Path, commands: &[CommandSpec]) -> anyhow::Result<()> {
    for command in commands {
        run_process(root, command.program, command.args)?;
    }
    Ok(())
}

fn run_duplicate_dependency_check(root: &Path) -> anyhow::Result<()> {
    let output = run_process_capture_stdout(
        root,
        DUPLICATE_DEPENDENCY_COMMAND.program,
        DUPLICATE_DEPENDENCY_COMMAND.args,
    )?;
    let duplicate_names = duplicate_package_names(&output);
    let approved_names = APPROVED_DUPLICATE_PACKAGES
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    let unexpected_names = duplicate_names
        .iter()
        .filter(|name| !approved_names.contains(name.as_str()))
        .cloned()
        .collect::<Vec<_>>();

    if !unexpected_names.is_empty() {
        bail!(
            "duplicate dependency check found unapproved duplicate packages: {}. Inspect `cargo tree -d` and either unify versions or add a deliberate allowlist entry in xtask/src/main.rs.",
            unexpected_names.join(", ")
        );
    }

    Ok(())
}

fn duplicate_package_names(output: &str) -> Vec<String> {
    let mut names = BTreeSet::new();

    for line in output.lines() {
        let Some(first) = line.chars().next() else {
            continue;
        };

        if first.is_whitespace() || matches!(first, '├' | '└' | '│') {
            continue;
        }

        let Some((name, version_tail)) = line.split_once(" v") else {
            continue;
        };

        if version_tail
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_digit())
        {
            names.insert(name.to_owned());
        }
    }

    names.into_iter().collect()
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

fn run_process_capture_stdout(root: &Path, program: &str, args: &[&str]) -> anyhow::Result<String> {
    let command_text = format_command(program, args);
    println!("--- STAGE: {command_text} ---");

    let output = ProcessCommand::new(program)
        .args(args)
        .current_dir(root)
        .env("RUSTUP_TOOLCHAIN", rustup_toolchain())
        .output()
        .with_context(|| format!("failed to start `{command_text}`"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();

    if !stdout.is_empty() {
        print!("{stdout}");
    }
    if !stderr.is_empty() {
        eprint!("{stderr}");
    }

    if !output.status.success() {
        bail!(
            "command `{command_text}` failed with status {}",
            output.status
        );
    }

    Ok(stdout)
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
        APPROVED_DUPLICATE_PACKAGES, CI_COMMANDS, Cli, DEPENDENCY_HYGIENE_COMMANDS,
        DEPENDENCY_HYGIENE_WORKFLOW, DUPLICATE_DEPENDENCY_COMMAND, FIXTURE_CHECK_COMMANDS,
        FIXTURE_CHECK_WORKFLOW, FIXTURE_UPDATE_COMMANDS, FIXTURE_UPDATE_WORKFLOW,
        REPORT_GOLDEN_UPDATE_COMMANDS, REPORT_GOLDEN_UPDATE_WORKFLOW, SCHEMA_CHECK_COMMANDS,
        SCHEMA_CHECK_WORKFLOW, SMOKE_COMMANDS, duplicate_package_names, format_command,
        scan_allow_attribute_file,
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
                "fixture-check",
                "fixture-update",
                "fmt",
                "generate-completions",
                "generate-man",
                "no-allow-attrs",
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
        std::fs::write(
            &source,
            "\
#![deny(warnings)]
#![allow(dead_code)]
#[allow(unused_imports)]
#[cfg_attr(test, allow(dead_code))]
pub fn live() {}
",
        )
        .expect("write temp scanner fixture");

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

    fn command_texts(commands: &[super::CommandSpec]) -> Vec<String> {
        commands
            .iter()
            .map(|command| format_command(command.program, command.args))
            .collect::<Vec<_>>()
    }
}
