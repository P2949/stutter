use clap::CommandFactory;

use super::{CI_COMMANDS, Cli, SMOKE_COMMANDS, VALIDATION_COMMANDS};
use crate::{
    dependency_hygiene::{
        APPROVED_DUPLICATE_PACKAGES, DEPENDENCY_HYGIENE_COMMANDS, DEPENDENCY_HYGIENE_WORKFLOW,
        DUPLICATE_DEPENDENCY_COMMAND, duplicate_package_names,
    },
    ebpf_smoke::PRIVILEGED_EBPF_SMOKE_COMMANDS,
    fixtures::{
        FIXTURE_CHECK_COMMANDS, FIXTURE_CHECK_WORKFLOW, FIXTURE_UPDATE_COMMANDS,
        FIXTURE_UPDATE_WORKFLOW, REPORT_GOLDEN_UPDATE_COMMANDS, REPORT_GOLDEN_UPDATE_WORKFLOW,
        SCHEMA_CHECK_COMMANDS, SCHEMA_CHECK_WORKFLOW,
    },
    no_allow_attrs::scan_allow_attribute_file,
    process::format_command,
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
            "calibrate-scheduler",
            "capability-check",
            "ci",
            "clippy",
            "dependency-hygiene",
            "ebpf-manifest",
            "ebpf-smoke",
            "evidence-export",
            "evidence-index",
            "evidence-verify",
            "fixed-work",
            "fixture-check",
            "fixture-coverage",
            "fixture-update",
            "fmt",
            "generate-completions",
            "generate-man",
            "kernel-matrix",
            "live-acceptance",
            "live-burst",
            "live-workload",
            "local-install-smoke",
            "maturity-report",
            "no-allow-attrs",
            "objective-docs",
            "package",
            "package-layout-check",
            "preflight",
            "privileged-ebpf-smoke",
            "recovery-controller",
            "refresh-tune-recommendation",
            "release-package",
            "release-verify",
            "report-golden-update",
            "schema-check",
            "score-evaluate",
            "service-smoke",
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
        vec!["bash scripts/validate-privileged-ebpf-smoke.sh"]
    );
}

#[test]
fn unprivileged_ebpf_smoke_command_targets_build_only() {
    assert_eq!(
        command_texts(crate::ebpf_smoke::UNPRIVILEGED_EBPF_SMOKE_COMMANDS),
        vec!["cargo build -p stutter"]
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
        "cargo tree -d --color never"
    );
    assert_eq!(
        DEPENDENCY_HYGIENE_WORKFLOW.affected_paths,
        &[
            "Cargo.toml",
            "Cargo.lock",
            "deny.toml",
            "*/Cargo.toml",
            "xtask/src/dependency_hygiene.rs",
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
fn duplicate_dependency_parser_ignores_ansi_colored_tree_entries() {
    let output = "\
bitflags v1.3.2
\u{1b}[2m└──\u{1b}[0m example v0.1.0

bitflags v2.11.1 \u{1b}[33m\u{1b}[2m(*)\u{1b}[39m\u{1b}[22m
\u{1b}[2m├──\u{1b}[0m other v0.1.0
\u{1b}[2m└──\u{1b}[0m transitive v0.1.0
";

    assert_eq!(duplicate_package_names(output), vec!["bitflags".to_owned()]);
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
            "docs/examples/artifacts/v23/**",
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
            "cargo test -p stutter validation_corpus_tests::regenerate::regenerate_validation_corpus -- --ignored --exact",
            "cargo test -p stutter validation_corpus_tests::regenerate::regenerate_public_examples_v23 -- --ignored --exact",
        ]
    );
    assert_eq!(
        FIXTURE_UPDATE_WORKFLOW.affected_paths,
        &[
            "stutter/tests/fixtures/runs/**",
            "docs/examples/artifacts/v23/**",
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
