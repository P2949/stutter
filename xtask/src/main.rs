use std::{env, path::PathBuf};

use anyhow::{Context, bail};
use clap::{Parser, Subcommand};

pub mod capability_check;
pub mod coverage_maturity;
pub mod dependency_hygiene;
pub mod diagnosis_calibration;
pub mod diagnosis_study;
pub mod ebpf_manifest;
pub mod ebpf_smoke;
pub mod evidence_bundle;
mod evidence_metadata;
pub mod fixed_work;
pub mod fixture_coverage;
pub mod fixtures;
pub mod kernel_matrix;
mod live_acceptance;
pub mod local_ci;
pub mod local_ci_policy;
pub mod maturity_report;
pub mod mutation_campaign;
pub mod no_allow_attrs;
pub mod objective_docs;
pub mod ops_checks;
pub mod preflight;
pub mod process;
pub mod refresh_tune_recommendation;
pub mod release_output;
pub mod release_package;
pub mod score_evaluation;
pub mod score_matrix;
pub mod score_target;
pub mod soak;
pub mod validation_campaign;
pub mod workflow;

use crate::{
    dependency_hygiene::run_dependency_hygiene,
    ebpf_manifest::run_ebpf_manifest,
    ebpf_smoke::{EBPF_BUILD_COMMAND, run_privileged_ebpf_smoke, run_unprivileged_ebpf_smoke},
    fixture_coverage::run_fixture_coverage,
    fixtures::{
        FIXTURE_CHECK_WORKFLOW, FIXTURE_UPDATE_WORKFLOW, REPORT_GOLDEN_UPDATE_WORKFLOW,
        SCHEMA_CHECK_WORKFLOW,
    },
    maturity_report::run_maturity_report,
    no_allow_attrs::run_no_allow_attrs,
    ops_checks::{run_local_install_smoke, run_package_layout_check, run_service_smoke},
    preflight::run_preflight,
    process::run_cargo,
    refresh_tune_recommendation::refresh_tune_recommendation,
    workflow::{CommandSpec, run_command_specs, run_workflow},
};

#[derive(Debug, Parser)]
#[command(name = "xtask", about = "Stutter development workflow tasks")]
pub struct Cli {
    #[command(subcommand)]
    pub command: XtaskCommand,
}

#[derive(Clone, Debug, Eq, PartialEq, Subcommand)]
pub enum XtaskCommand {
    #[command(about = "Run canonical live observation acceptance and retain evidence")]
    LiveAcceptance {
        #[command(subcommand)]
        suite: live_acceptance::Suite,
    },
    #[command(hide = true)]
    LiveBurst {
        #[arg(long)]
        burst_ms: u64,
        #[arg(long)]
        period_ms: u64,
        #[arg(long)]
        seconds: u64,
    },
    #[command(hide = true)]
    LiveWorkload {
        #[arg(long)]
        descendants: bool,
        #[arg(long)]
        seconds: u64,
        #[arg(long)]
        ready: PathBuf,
    },
    #[command(hide = true)]
    RecoveryController {
        #[command(subcommand)]
        mode: live_acceptance::recovery::ControllerCommand,
    },
    #[command(
        name = "evidence-export",
        about = "Sanitize raw suite output into a portable bundle"
    )]
    EvidenceExport(evidence_bundle::ExportArgs),
    #[command(
        name = "evidence-verify",
        about = "Verify a portable evidence bundle (fails closed)"
    )]
    EvidenceVerify(evidence_bundle::VerifyArgs),
    #[command(
        name = "capability-check",
        about = "Fail if a capability is Supported ahead of its evidence"
    )]
    CapabilityCheck,
    #[command(
        name = "score-evaluate",
        about = "Evaluate StutterScore against controlled degradation recordings"
    )]
    ScoreEvaluate {
        /// `LEVEL_MS:RECORDING_DIR`, repeated (e.g. 0:run0 1:run1 ...).
        #[arg(long = "level", value_name = "MS:DIR", required = true)]
        levels: Vec<String>,
        /// Fail unless every level has a usable frame stream and an explicit recorded target
        /// (one `--tree-pid`/manual PID); implies strict target identity.
        #[arg(long = "require-frames")]
        require_frames: bool,
    },
    #[command(
        name = "calibrate",
        about = "Sweep one diagnosis family threshold over the frozen corpus"
    )]
    Calibrate {
        /// scheduler, io, irq or gpu.
        #[arg(long)]
        family: String,
        #[arg(long, default_value = "stutter/tests/fixtures/runs")]
        corpus: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    #[command(
        name = "validation-campaign",
        about = "Drive the resumable Core Validation campaign (init, status, continue, verify)"
    )]
    ValidationCampaign(validation_campaign::cli::CampaignArgs),
    #[command(
        name = "diagnosis-study",
        about = "Drive the post-v0.1 diagnosis evidence study (init, status, continue, verify, report)"
    )]
    DiagnosisStudy(diagnosis_study::StudyArgs),
    /// Generic mutation-graduation campaign (init/status/continue/verify/report/cancel).
    MutationCampaign(mutation_campaign::MutationArgs),
    #[command(
        name = "score-matrix",
        about = "Drive the StutterScore workload-matrix evidence study (init, status, continue, report)"
    )]
    ScoreMatrix(score_matrix::MatrixArgs),
    #[command(
        name = "soak",
        about = "Drive the real always-on soak of the graduated operations (init, run, status, verify)"
    )]
    Soak(soak::SoakArgs),
    #[command(
        name = "kernel-matrix",
        about = "Report observed kernel compatibility from the evidence index"
    )]
    KernelMatrix {
        #[arg(long, default_value_t = 0)]
        require: usize,
    },
    #[command(
        name = "evidence-index",
        about = "Print the evidence-index entry for a verified bundle"
    )]
    EvidenceIndex(evidence_bundle::VerifyArgs),
    #[command(
        name = "release-package",
        about = "Package the supported layout from the committed source"
    )]
    ReleasePackage(release_package::PackageArgs),
    #[command(
        name = "release-verify",
        about = "Verify a release directory independently"
    )]
    ReleaseVerify(release_package::VerifyArgs),
    #[command(
        name = "release-install-smoke",
        about = "Unpack a release package and smoke-test the installed binary on the exact-RC recording"
    )]
    ReleaseInstallSmoke(release_package::InstallSmokeArgs),
    #[command(
        name = "release-publish",
        about = "Verify a release and publish it to GitHub idempotently"
    )]
    ReleasePublish(release_package::PublishArgs),
    #[command(about = "Generate or check objective contract documentation")]
    ObjectiveDocs {
        #[arg(long, conflicts_with = "write", required_unless_present = "write")]
        check: bool,
        #[arg(long, conflicts_with = "check", required_unless_present = "check")]
        write: bool,
    },
    #[command(
        name = "fixed-work",
        about = "Time identical declared work and append validated start/completion markers"
    )]
    FixedWork {
        #[arg(long)]
        identity: PathBuf,
        #[arg(long)]
        markers: PathBuf,
        #[arg(long)]
        target_root_pid: Option<u32>,
    },
    #[command(
        about = "The full deterministic, non-root local CI (every stage; --stage reruns one)"
    )]
    Ci {
        #[arg(long)]
        stage: Option<String>,
        /// Print the stage names, one per line, and exit.
        #[arg(long, conflicts_with = "stage")]
        list: bool,
    },
    #[command(
        name = "verify-fast",
        about = "Cheap local admission gate: policy, fmt, lint contracts, diff check"
    )]
    VerifyFast,
    #[command(
        name = "ci-live",
        about = "The live smokes (privileged eBPF, tree churn, minimal recording) with non-interactive privilege"
    )]
    CiLive,
    #[command(name = "ci-all", about = "ci, then ci-live, then release-check")]
    CiAll {
        /// Caller-owned release output; it must not exist. Default: the managed target/release-check.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    #[command(
        name = "release-check",
        about = "Build, package and verify the release locally"
    )]
    ReleaseCheck {
        /// Caller-owned output; it must not exist. Default: the managed target/release-check,
        /// which a rerun replaces only after the new package verified.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Fail unless this tag is `v` plus the crate version.
        #[arg(long)]
        expected_tag: Option<String>,
    },
    #[command(
        name = "local-ci-policy",
        about = "Fail if hosted CI architecture or the hosted-runner protocol is present"
    )]
    LocalCiPolicy,
    #[command(
        name = "setup-local-ci",
        about = "Point this checkout's git hooks at the repository's .githooks"
    )]
    SetupLocalCi,
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
        about = "Build eBPF object and check toolchain without requiring root."
    )]
    EbpfSmoke,
    #[command(
        name = "ebpf-manifest",
        about = "Write a reproducible eBPF artifact manifest for a prebuilt object"
    )]
    EbpfManifest {
        #[arg(long = "object", value_name = "PATH")]
        object: PathBuf,
        #[arg(long = "out", value_name = "PATH")]
        out: PathBuf,
    },
    #[command(
        name = "privileged-ebpf-smoke",
        about = "Build and run privileged eBPF loader tests."
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
        name = "fixture-coverage",
        about = "Print validation corpus coverage matrix and fail if required cells disappear"
    )]
    FixtureCoverage {
        #[arg(long = "html", value_name = "PATH")]
        html: Option<PathBuf>,
    },
    #[command(
        name = "service-smoke",
        about = "Validate packaged service unit command shapes without enabling services"
    )]
    ServiceSmoke,
    #[command(
        name = "package-layout-check",
        about = "Validate tarball/distro packaging layout references"
    )]
    PackageLayoutCheck,
    #[command(
        name = "local-install-smoke",
        about = "Install stutter into a temporary prefix and run dry-run checks"
    )]
    LocalInstallSmoke,
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
    #[command(
        name = "refresh-tune-recommendation",
        about = "Regenerate tune recommendation JSON, Markdown, and HTML from a tuning summary"
    )]
    RefreshTuneRecommendation {
        #[arg(long = "summary", value_name = "PATH")]
        summary: PathBuf,
        #[arg(long = "baseline-profile", value_name = "NAME")]
        baseline_profile: Option<String>,
        #[arg(long = "out-dir", value_name = "DIR")]
        out_dir: PathBuf,
    },
    #[command(name = "generate-man", about = "Scaffold for man page generation")]
    GenerateMan,
    #[command(
        name = "generate-completions",
        about = "Scaffold for shell completion generation"
    )]
    GenerateCompletions,
    #[command(about = "Scaffold for package build workflow")]
    Package,
    #[command(name = "maturity-report", about = "Print maturity progress report")]
    MaturityReport,
}

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
        XtaskCommand::LiveAcceptance { suite } => live_acceptance::run(&root, suite),
        XtaskCommand::RecoveryController { mode } => live_acceptance::recovery::controller(mode),
        XtaskCommand::LiveBurst {
            burst_ms,
            period_ms,
            seconds,
        } => live_acceptance::burst(burst_ms, period_ms, seconds),
        XtaskCommand::LiveWorkload {
            seconds,
            ready,
            descendants,
        } => live_acceptance::workload(seconds, &ready, descendants),
        XtaskCommand::ReleasePackage(args) => release_package::package_release(&root, &args),
        XtaskCommand::ReleaseVerify(args) => release_package::verify_release(&args),
        XtaskCommand::ReleaseInstallSmoke(args) => release_package::install_smoke(&args),
        XtaskCommand::ReleasePublish(args) => release_package::publish_release(&root, &args),
        XtaskCommand::ValidationCampaign(args) => validation_campaign::run(&root, args),
        XtaskCommand::DiagnosisStudy(args) => diagnosis_study::run(&root, args),
        XtaskCommand::MutationCampaign(args) => mutation_campaign::run(&root, args),
        XtaskCommand::ScoreMatrix(args) => score_matrix::run(&root, args),
        XtaskCommand::Soak(args) => soak::run(&root, args),
        XtaskCommand::KernelMatrix { require } => kernel_matrix::run(&root, require),
        XtaskCommand::EvidenceIndex(args) => evidence_bundle::index_entry(&args.bundle),
        XtaskCommand::Calibrate {
            family,
            corpus,
            out,
        } => diagnosis_calibration::run(&root, &corpus, out.as_deref(), &family),
        XtaskCommand::ScoreEvaluate {
            levels,
            require_frames,
        } => {
            let parsed = levels
                .iter()
                .map(|l| {
                    let (ms, dir) = l.split_once(':').context("expected MS:DIR")?;
                    Ok((ms.parse::<f64>()?, PathBuf::from(dir)))
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            println!(
                "{}",
                serde_json::to_string_pretty(&score_evaluation::run_from_recordings(
                    &parsed,
                    require_frames
                )?)?
            );
            Ok(())
        }
        XtaskCommand::CapabilityCheck => capability_check::run(&root),
        XtaskCommand::EvidenceExport(args) => {
            evidence_bundle::export_bundle(&root, &args.raw, &args.out)
        }
        XtaskCommand::EvidenceVerify(args) => evidence_bundle::verify_bundle(&args.bundle),
        XtaskCommand::ObjectiveDocs { write, .. } => {
            objective_docs::run_at(&root.join("docs/AUTOTUNE_OBJECTIVES.md"), write)
        }
        XtaskCommand::FixedWork {
            identity,
            markers,
            target_root_pid,
        } => fixed_work::run(&identity, &markers, target_root_pid),
        XtaskCommand::Ci { stage, list } => {
            if list {
                local_ci::list_stages();
                Ok(())
            } else {
                local_ci::ci(&root, stage.as_deref())
            }
        }
        XtaskCommand::VerifyFast => local_ci::verify_fast(&root),
        XtaskCommand::CiLive => local_ci::ci_live(&root),
        XtaskCommand::CiAll { out } => local_ci::ci_all(&root, out.as_deref()),
        XtaskCommand::ReleaseCheck { out, expected_tag } => {
            local_ci::release_check(&root, out.as_deref(), expected_tag.as_deref())
        }
        XtaskCommand::LocalCiPolicy => local_ci_policy::run(&root),
        XtaskCommand::SetupLocalCi => local_ci::setup(&root),
        XtaskCommand::Fmt => run_cargo(&root, &["fmt", "--check"]),
        XtaskCommand::Clippy => {
            run_cargo(&root, &["clippy", "--all-targets", "--", "-D", "warnings"])
        }
        XtaskCommand::Smoke => run_command_specs(&root, SMOKE_COMMANDS),
        XtaskCommand::Preflight => run_preflight(),
        XtaskCommand::EbpfSmoke => run_unprivileged_ebpf_smoke(&root),
        XtaskCommand::EbpfManifest { object, out } => run_ebpf_manifest(&root, &object, &out),
        XtaskCommand::PrivilegedEbpfSmoke => run_privileged_ebpf_smoke(&root),
        XtaskCommand::Validate => {
            run_preflight()?;
            run_command_specs(&root, VALIDATION_COMMANDS)
        }
        XtaskCommand::DependencyHygiene => run_dependency_hygiene(&root),
        XtaskCommand::SchemaCheck => run_workflow(&root, SCHEMA_CHECK_WORKFLOW),
        XtaskCommand::NoAllowAttrs => run_no_allow_attrs(&root),
        XtaskCommand::FixtureCheck => {
            run_fixture_coverage(&root, None)?;
            run_workflow(&root, FIXTURE_CHECK_WORKFLOW)
        }
        XtaskCommand::FixtureCoverage { html } => run_fixture_coverage(&root, html.as_deref()),
        XtaskCommand::ServiceSmoke => run_service_smoke(&root),
        XtaskCommand::PackageLayoutCheck => run_package_layout_check(&root),
        XtaskCommand::LocalInstallSmoke => run_local_install_smoke(&root),
        XtaskCommand::FixtureUpdate => run_workflow(&root, FIXTURE_UPDATE_WORKFLOW),
        XtaskCommand::ReportGoldenUpdate => run_workflow(&root, REPORT_GOLDEN_UPDATE_WORKFLOW),
        XtaskCommand::RefreshTuneRecommendation {
            summary,
            baseline_profile,
            out_dir,
        } => refresh_tune_recommendation(&summary, baseline_profile.as_deref(), &out_dir),
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
        XtaskCommand::MaturityReport => run_maturity_report(&root),
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
#[path = "main_tests.rs"]
mod tests;
