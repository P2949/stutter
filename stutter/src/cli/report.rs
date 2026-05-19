use super::*;

#[derive(Args, Debug, Clone)]
pub(super) struct ManArgs {
    #[arg(long = "output", short = 'o', value_name = "PATH")]
    pub(super) output: Option<std::path::PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(super) struct CompletionsArgs {
    #[arg(value_enum)]
    pub(super) shell: clap_complete::Shell,
}

#[derive(Args, Debug, Clone)]
pub(super) struct ProbesArgs {
    #[arg(long)]
    pub(super) json: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct InspectTreeArgs {
    #[arg(long = "tree-pid", value_name = "PID")]
    pub(super) tree_pid: u32,
}

#[derive(Args, Debug, Clone)]
pub(super) struct ReportArgs {
    #[arg(
        long,
        help = "Output raw session JSON",
        conflicts_with_all = ["analysis_json", "json_summary", "html"]
    )]
    pub(super) json: bool,

    #[arg(
        long = "flamegraph",
        alias = "latency-flamegraph",
        value_name = "SVG",
        help = "Write a latency attribution flamegraph SVG"
    )]
    pub(super) flamegraph: Option<PathBuf>,

    #[arg(
        long = "analysis-json",
        help = "Output full analysis JSON (clusters, diagnoses, artifacts)",
        conflicts_with_all = ["json", "json_summary", "html", "batch"]
    )]
    pub(super) analysis_json: bool,

    #[arg(
        long = "json-summary",
        help = "Output compact summary JSON",
        conflicts_with_all = ["json", "analysis_json", "html"]
    )]
    pub(super) json_summary: bool,

    #[arg(
        long = "html",
        value_name = "PATH",
        help = "Generate HTML report",
        conflicts_with_all = ["json", "analysis_json", "json_summary", "batch"]
    )]
    pub(super) html: Option<PathBuf>,

    #[arg(
        long = "batch",
        value_name = "DIR",
        help = "Run report on all sessions in DIR; outputs text summary or JSON summary if --json or --json-summary is set",
        conflicts_with_all = ["analysis_json", "html"]
    )]
    pub(super) batch: Option<PathBuf>,

    #[arg(long, default_value_t = 10, value_name = "N")]
    pub(super) top: usize,

    #[arg(long = "cluster-ms", default_value_t = 5, value_name = "MS")]
    pub(super) cluster_window_ms: u64,

    #[arg(
        long = "diff",
        value_name = "PATH",
        help = "Compare session(s) against baseline session at PATH"
    )]
    pub(super) diff: Option<PathBuf>,

    #[arg(long, value_name = "CLASS")]
    pub(super) filter_class: Option<String>,

    #[arg(
        help = "Path to session directory or session.json",
        conflicts_with = "batch"
    )]
    pub(super) path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(super) struct SummaryArgs {
    #[arg(long)]
    pub(super) json: bool,

    #[arg(long, default_value_t = 10, value_name = "N")]
    pub(super) top: usize,

    #[arg(long, value_name = "CLASS")]
    pub(super) filter_class: Option<String>,

    pub(super) path: PathBuf,
}

#[derive(Args, Debug, Clone)]
pub(super) struct RestoreArgs {
    #[arg(long = "dry-run")]
    pub(super) dry_run: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct ApplyProfileArgs {
    #[arg(long = "tree-pid", value_name = "PID")]
    pub(super) tree_pid: u32,

    #[arg(long = "profile", value_name = "FILE")]
    pub(super) profile: PathBuf,

    #[arg(long)]
    pub(super) force: bool,

    #[arg(long = "dry-run")]
    pub(super) dry_run: bool,

    #[arg(long = "allow-medium-risk")]
    pub(super) allow_medium_risk: bool,

    #[arg(long)]
    pub(super) watch: bool,

    #[arg(long = "keep-applied")]
    pub(super) keep_applied: bool,

    #[arg(long = "refresh-ms", default_value_t = 1_000)]
    pub(super) refresh_ms: u64,

    #[arg(long)]
    pub(super) enforce: bool,
}

#[derive(Args, Debug, Clone)]
#[command(
    about = "Benchmark multiple profiles and select the best one",
    long_about = "Benchmark multiple profiles and select the best one. \
                  Warning: ranking is count-based and workload-sensitive. It assumes comparable route/scene/load \
                  across epochs and will reject profiles with major scored-sample or frame-count mismatches. \
                  Use --runs 3 or higher for reliable results."
)]
pub(super) struct TuneArgs {
    #[arg(long = "tree-pid", value_name = "PID")]
    pub(super) tree_pid: u32,

    #[arg(long = "profiles", value_name = "FILE")]
    pub(super) profiles: PathBuf,

    #[arg(long = "epoch-seconds", default_value_t = 120)]
    pub(super) epoch_seconds: u64,

    #[arg(long = "warmup-seconds", default_value_t = 30)]
    pub(super) warmup_seconds: u64,

    #[arg(long = "keep-best")]
    pub(super) keep_best: bool,

    #[arg(long = "baseline-profile", value_name = "NAME")]
    pub(super) baseline_profile: Option<String>,

    #[arg(long = "out-dir", value_name = "PATH")]
    pub(super) out_dir: Option<PathBuf>,

    #[arg(long = "mangohud-log", value_name = "PATH")]
    pub(super) mangohud_log: Option<PathBuf>,

    #[arg(long, short = 'n', default_value_t = 3, value_name = "N")]
    pub(super) runs: u32,

    #[arg(long)]
    pub(super) enforce: bool,

    #[arg(long = "hwmon", id = "hwmon")]
    pub(super) hwmon: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct CheckArgs {
    #[arg(long = "baseline", value_name = "PATH")]
    pub(super) baseline: PathBuf,

    #[arg(long = "current", value_name = "PATH")]
    pub(super) current: PathBuf,

    #[arg(long = "max-regression-p99-ms", value_name = "MS")]
    pub(super) max_regression_p99_ms: Option<f64>,

    #[arg(long = "max-max-regression-ms", value_name = "MS")]
    pub(super) max_max_regression_ms: Option<f64>,

    #[arg(long)]
    pub(super) json: bool,

    #[arg(long, default_value_t = 10, value_name = "N")]
    pub(super) top: usize,

    #[arg(long, value_name = "CLASS")]
    pub(super) filter_class: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub(super) struct RecommendArgs {
    #[arg(long = "baseline", value_name = "PATH")]
    pub(super) baseline: PathBuf,

    #[arg(long = "tune", value_name = "PATH")]
    pub(super) tune: PathBuf,

    #[arg(long)]
    pub(super) json: bool,

    #[arg(long, value_name = "PATH")]
    pub(super) markdown: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(super) struct ReleaseArgs {
    #[command(subcommand)]
    pub(super) command: ReleaseCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub(super) enum ReleaseCommand {
    Check(ReleaseCheckArgs),
}

#[derive(Args, Debug, Clone)]
pub(super) struct ReleaseCheckArgs {
    #[arg(long = "channel", default_value = "experimental")]
    pub(super) channel: String,

    #[arg(long = "apply-actions-enabled")]
    pub(super) apply_actions_enabled: bool,

    #[arg(long = "soak-tests")]
    pub(super) soak_tests: bool,

    #[arg(long = "stronger-tests")]
    pub(super) stronger_tests: bool,

    #[arg(long)]
    pub(super) json: bool,

    #[arg(long)]
    pub(super) enforce: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct AuditArgs {
    #[arg(long = "path", value_name = "PATH")]
    pub(super) path: Option<PathBuf>,

    #[arg(long, default_value_t = 20)]
    pub(super) tail: usize,

    #[arg(long)]
    pub(super) json: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct AdvisorArgs {
    #[arg(long = "run", value_name = "PATH")]
    pub(super) run: Option<PathBuf>,

    #[arg(long = "profiles", value_name = "PATH")]
    pub(super) profiles: Option<PathBuf>,

    #[arg(long)]
    pub(super) json: bool,

    #[arg(long = "watch-runs")]
    pub(super) watch_runs: bool,

    #[arg(long = "runs-dir", value_name = "PATH")]
    pub(super) runs_dir: Option<PathBuf>,

    #[arg(long = "poll-seconds", default_value_t = 10)]
    pub(super) poll_seconds: u64,

    #[arg(long)]
    pub(super) once: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DoctorArgs {
    #[arg(long)]
    pub(super) json: bool,

    #[arg(long = "hwmon", id = "hwmon")]
    pub(super) hwmon: bool,

    #[arg(long = "hwmon-root", value_name = "PATH", requires = "hwmon")]
    pub(super) hwmon_root: Option<PathBuf>,

    #[arg(long = "hwmon-drm-card", value_name = "CARD", requires = "hwmon")]
    pub(super) hwmon_drm_card: Option<String>,

    #[arg(long = "hwmon-render-node", value_name = "NODE", requires = "hwmon")]
    pub(super) hwmon_render_node: Option<PathBuf>,

    #[arg(long = "irq-latency")]
    pub(super) irq_latency: bool,

    #[arg(long = "irq", value_name = "IRQ")]
    pub(super) irqs: Vec<u32>,

    #[arg(long = "block-io")]
    pub(super) block_io: bool,

    #[arg(long = "kms-timing")]
    pub(super) kms_timing: bool,

    #[arg(long = "faults")]
    pub(super) faults: bool,

    #[arg(long = "cpu-perf")]
    pub(super) cpu_perf: bool,

    #[arg(long = "mangohud-log", value_name = "PATH")]
    pub(super) mangohud_log: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(super) struct ProfileTemplateArgs {
    #[arg(long = "topology")]
    pub(super) topology: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct InspectIrqsArgs {
    #[arg(long)]
    pub(super) json: bool,

    #[arg(long = "filter", value_name = "TEXT")]
    pub(super) filter: Vec<String>,

    #[arg(long, default_value_t = 30)]
    pub(super) top: usize,
}

#[derive(Args, Debug, Clone)]
pub(super) struct InspectDrmTracepointsArgs {
    #[arg(long)]
    pub(super) json: bool,

    #[arg(long = "events-root", value_name = "PATH", hide = true)]
    pub(super) events_root: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(super) struct WaylandProbeArgs {
    #[arg(long = "duration", default_value_t = 30, value_name = "SECONDS")]
    pub(super) duration_secs: u64,

    #[arg(long = "output", value_name = "NAME")]
    pub(super) output: Option<String>,

    #[arg(long = "fullscreen")]
    pub(super) fullscreen: bool,

    #[arg(long = "out-dir", value_name = "DIR")]
    pub(super) out_dir: PathBuf,
}

#[derive(Args, Debug, Clone)]
pub(super) struct CompareArgs {
    #[command(subcommand)]
    pub(super) command: CompareCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub(super) enum CompareCommand {
    #[command(name = "display-path")]
    DisplayPath(DisplayPathCompareArgs),
}

#[derive(Args, Debug, Clone)]
pub(super) struct DisplayPathCompareArgs {
    #[arg(long = "baseline", value_name = "RUN")]
    pub(super) baseline: PathBuf,

    #[arg(long = "test", value_name = "RUN")]
    pub(super) test: PathBuf,

    #[arg(long = "json")]
    pub(super) json: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct RulesArgs {
    #[command(subcommand)]
    pub(super) command: RulesCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub(super) enum RulesCommand {
    Import(CliRulesImportArgs),
    List(RulesListArgs),
    Status(RulesStatusArgs),
    Enable(RulesEnableArgs),
    Disable(RulesDisableArgs),
    Check(RulesCheckArgs),
    Remove(RulesRemoveArgs),
}

#[derive(Args, Debug, Clone)]
#[command(group(
    clap::ArgGroup::new("rules_check_input")
        .required(true)
        .args(["source", "generated"])
))]
pub(super) struct RulesCheckArgs {
    #[arg(long = "source", value_name = "PATH", conflicts_with = "generated")]
    pub(super) source: Option<PathBuf>,

    #[arg(long = "generated", value_name = "PATH", conflicts_with = "source")]
    pub(super) generated: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(super) struct CliRulesImportArgs {
    #[arg(long = "source", value_name = "PATH")]
    pub(super) source: PathBuf,

    #[arg(long, default_value = "ananicy")]
    pub(super) name: String,

    #[arg(long, default_value = "GPL-3.0-only")]
    pub(super) license: String,

    #[arg(long = "source-repo")]
    pub(super) source_repo: Option<String>,

    #[arg(long = "source-commit")]
    pub(super) source_commit: Option<String>,

    #[arg(long = "out", value_name = "PATH")]
    pub(super) out: Option<PathBuf>,

    #[arg(long = "dry-run")]
    pub(super) dry_run: bool,
}

impl From<CliRulesImportArgs> for crate::commands::input::RulesImportCommandInput {
    fn from(args: CliRulesImportArgs) -> Self {
        Self {
            source: args.source,
            name: args.name,
            source_repo: args.source_repo,
            source_commit: args.source_commit,
            license: args.license,
            out: args.out,
            dry_run: args.dry_run,
        }
    }
}

impl From<RulesCheckArgs> for crate::commands::input::RulesCheckArgs {
    fn from(args: RulesCheckArgs) -> Self {
        Self {
            source: args.source,
            generated: args.generated,
        }
    }
}

#[derive(Args, Debug, Clone)]
pub(super) struct RulesListArgs {}

#[derive(Args, Debug, Clone)]
pub(super) struct RulesStatusArgs {}

#[derive(Args, Debug, Clone)]
pub(super) struct RulesEnableArgs {
    #[arg(long = "name", default_value = "ananicy")]
    pub(super) name: String,
}

#[derive(Args, Debug, Clone)]
pub(super) struct RulesDisableArgs {}

#[derive(Args, Debug, Clone)]
pub(super) struct RulesRemoveArgs {
    #[arg(long = "name", default_value = "ananicy")]
    pub(super) name: String,

    #[arg(long = "dry-run")]
    pub(super) dry_run: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct ScenarioArgs {
    #[command(subcommand)]
    pub(super) command: ScenarioCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub(super) enum ScenarioCommand {
    Create(ScenarioCreateArgs),
    Run(ScenarioRunArgs),
    Compare(ScenarioCompareArgs),
    Path(ScenarioPathArgs),
    List,
}

#[derive(Args, Debug, Clone)]
pub(super) struct ScenarioCreateArgs {
    pub(super) name: String,

    #[arg(long = "force")]
    pub(super) force: bool,

    #[arg(long = "watch-process", value_name = "COMM")]
    pub(super) watch_process: Option<String>,

    #[arg(long = "duration", default_value_t = 180)]
    pub(super) duration: u64,

    #[arg(long = "preset", default_value = "diagnosis")]
    pub(super) preset: String,

    #[arg(long = "mangohud-log", value_name = "PATH")]
    pub(super) mangohud_log: Option<PathBuf>,

    #[arg(long = "notes")]
    pub(super) notes: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub(super) struct ScenarioRunArgs {
    pub(super) name: String,

    #[arg(long = "role", value_name = "baseline|current")]
    pub(super) role: String,

    #[arg(long = "dry-run")]
    pub(super) dry_run: bool,

    #[arg(long = "out-dir", value_name = "PATH")]
    pub(super) out_dir: Option<PathBuf>,

    #[arg(long = "mangohud-log", value_name = "PATH")]
    pub(super) mangohud_log_override: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(super) struct ScenarioCompareArgs {
    pub(super) name: String,

    #[arg(long = "baseline", value_name = "RUN_DIR")]
    pub(super) baseline: Option<PathBuf>,

    #[arg(long = "current", value_name = "RUN_DIR")]
    pub(super) current: Option<PathBuf>,

    #[arg(long, default_value_t = 10)]
    pub(super) top: usize,

    #[arg(long = "json-summary")]
    pub(super) json_summary: bool,

    #[arg(long = "validate")]
    pub(super) validate: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct ScenarioPathArgs {
    pub(super) name: String,
}

#[cfg(test)]
mod rules_cli_tests {
    use super::*;

    #[test]
    fn rules_check_requires_source_or_generated() {
        let result = Cli::try_parse_from(["stutter", "rules", "check"]);
        assert!(result.is_err());
    }

    #[test]
    fn rules_check_accepts_source_path() {
        let cli = Cli::try_parse_from([
            "stutter",
            "rules",
            "check",
            "--source",
            "/tmp/ananicy-rules",
        ])
        .unwrap();

        let command = cli.command.unwrap();
        match command {
            Command::Rules(args) => match args.command {
                RulesCommand::Check(check) => {
                    assert_eq!(check.source, Some(PathBuf::from("/tmp/ananicy-rules")));
                    assert_eq!(check.generated, None);
                }
                other => panic!("expected rules check command, got {other:?}"),
            },
            other => panic!("expected rules command, got {other:?}"),
        }
    }

    #[test]
    fn rules_check_accepts_generated_path() {
        let cli = Cli::try_parse_from([
            "stutter",
            "rules",
            "check",
            "--generated",
            "/tmp/ananicy.generated.json",
        ])
        .unwrap();

        let command = cli.command.unwrap();
        match command {
            Command::Rules(args) => match args.command {
                RulesCommand::Check(check) => {
                    assert_eq!(check.source, None);
                    assert_eq!(
                        check.generated,
                        Some(PathBuf::from("/tmp/ananicy.generated.json"))
                    );
                }
                other => panic!("expected rules check command, got {other:?}"),
            },
            other => panic!("expected rules command, got {other:?}"),
        }
    }

    #[test]
    fn rules_import_requires_source() {
        let result = Cli::try_parse_from(["stutter", "rules", "import"]);
        assert!(result.is_err());
    }

    #[test]
    fn rules_import_accepts_out_path() {
        let cli = Cli::try_parse_from([
            "stutter",
            "rules",
            "import",
            "--source",
            "/tmp/ananicy-rules",
            "--out",
            "/tmp/ananicy.generated.json",
        ])
        .unwrap();

        let command = cli.command.unwrap();
        match command {
            Command::Rules(args) => match args.command {
                RulesCommand::Import(import) => {
                    assert_eq!(import.source, PathBuf::from("/tmp/ananicy-rules"));
                    assert_eq!(
                        import.out,
                        Some(PathBuf::from("/tmp/ananicy.generated.json"))
                    );
                }
                other => panic!("expected rules import command, got {other:?}"),
            },
            other => panic!("expected rules command, got {other:?}"),
        }
    }

    #[test]
    fn rules_import_default_name_is_ananicy() {
        let cli = Cli::try_parse_from([
            "stutter",
            "rules",
            "import",
            "--source",
            "/tmp/ananicy-rules",
        ])
        .unwrap();

        let command = cli.command.unwrap();
        match command {
            Command::Rules(args) => match args.command {
                RulesCommand::Import(import) => {
                    assert_eq!(import.name, "ananicy");
                    assert_eq!(import.license, "GPL-3.0-only");
                    assert!(!import.dry_run);
                }
                other => panic!("expected rules import command, got {other:?}"),
            },
            other => panic!("expected rules command, got {other:?}"),
        }
    }

    #[test]
    fn rules_import_dry_run_does_not_write() {
        let cli = Cli::try_parse_from([
            "stutter",
            "rules",
            "import",
            "--source",
            "/tmp/ananicy-rules",
            "--dry-run",
        ])
        .unwrap();

        let command = cli.command.unwrap();
        match command {
            Command::Rules(args) => match args.command {
                RulesCommand::Import(import) => {
                    assert!(import.dry_run);
                    assert_eq!(import.out, None);
                }
                other => panic!("expected rules import command, got {other:?}"),
            },
            other => panic!("expected rules command, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod report_family_cli_tests {
    use super::*;
    use crate::{
        commands::input::{
            AppCommand, RulesCommand as RulesCommandDto, ScenarioCommand as ScenarioCommandDto,
        },
        process_tree::TaskClass,
        release::ReleaseChannel,
    };

    fn parse_report_command<const N: usize>(args: [&str; N]) -> anyhow::Result<AppCommand> {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        crate::cli::parse_app_command_from(args)
    }

    #[test]
    fn parses_report_cluster_window_and_top() {
        let command = parse_report_command([
            "stutter",
            "report",
            "--html",
            "/tmp/report.html",
            "--cluster-ms",
            "5",
            "--top",
            "25",
            "/tmp/run",
        ])
        .unwrap();

        let AppCommand::Report(input) = command else {
            panic!("expected report command");
        };

        assert_eq!(input.top, 25);
        assert_eq!(input.html, Some(PathBuf::from("/tmp/report.html")));
        assert_eq!(input.cluster_window_ms, 5);
        assert_eq!(input.path, Some(PathBuf::from("/tmp/run")));
    }

    #[test]
    fn rejects_zero_report_cluster_window() {
        let err = parse_report_command(["stutter", "report", "--cluster-ms", "0", "/tmp/run"])
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("--cluster-ms must be greater than zero")
        );
    }

    #[test]
    fn report_requires_path_unless_batch_is_set() {
        let err = parse_report_command(["stutter", "report"]).unwrap_err();

        assert!(
            err.to_string()
                .contains("report requires PATH unless --batch is set")
        );
    }

    #[test]
    fn report_flag_conflicts() {
        assert!(
            parse_report_command(["stutter", "report", "--html", "r.html", "--batch", "dir"])
                .is_err()
        );
        assert!(
            parse_report_command(["stutter", "report", "--json", "--json-summary", "run"]).is_err()
        );
        assert!(
            parse_report_command(["stutter", "report", "--analysis-json", "--batch", "dir"])
                .is_err()
        );
        assert!(parse_report_command(["stutter", "report", "--batch", "dir", "run"]).is_err());
    }

    #[test]
    fn parses_report_batch_json_summary() {
        let command = parse_report_command([
            "stutter",
            "report",
            "--batch",
            "/tmp/runs",
            "--json-summary",
            "--top",
            "4",
        ])
        .unwrap();

        let AppCommand::Report(input) = command else {
            panic!("expected report command");
        };

        assert_eq!(input.batch, Some(PathBuf::from("/tmp/runs")));
        assert!(input.json_summary);
        assert_eq!(input.top, 4);
        assert_eq!(input.path, None);
    }

    #[test]
    fn rejects_zero_report_top() {
        let err =
            parse_report_command(["stutter", "report", "--top", "0", "/tmp/run"]).unwrap_err();

        assert!(err.to_string().contains("--top must be greater than zero"));
    }

    #[test]
    fn parses_summary_command() {
        let command = parse_report_command([
            "stutter",
            "summary",
            "--json",
            "--top",
            "3",
            "--filter-class",
            "Game",
            "/tmp/run",
        ])
        .unwrap();

        let AppCommand::Summary(input) = command else {
            panic!("expected summary command");
        };

        assert_eq!(input.path, PathBuf::from("/tmp/run"));
        assert!(input.json);
        assert_eq!(input.top, 3);
        assert_eq!(input.filter_class, Some(TaskClass::Game));
    }

    #[test]
    fn rejects_summary_top_zero() {
        let err =
            parse_report_command(["stutter", "summary", "--top", "0", "/tmp/run"]).unwrap_err();

        assert!(err.to_string().contains("--top must be greater than zero"));
    }

    #[test]
    fn parses_release_check_command() {
        let command = parse_report_command([
            "stutter",
            "release",
            "check",
            "--channel",
            "low-risk-stable",
            "--soak-tests",
            "--json",
        ])
        .unwrap();

        let AppCommand::ReleaseCheck(input) = command else {
            panic!("expected release check command");
        };

        assert_eq!(input.channel, ReleaseChannel::LowRiskStable);
        assert!(input.inputs.soak_tests);
        assert!(input.json);
        assert!(!input.enforce);
    }

    #[test]
    fn parses_release_check_full_flags() {
        let command = parse_report_command([
            "stutter",
            "release",
            "check",
            "--channel",
            "experimental",
            "--apply-actions-enabled",
            "--soak-tests",
            "--stronger-tests",
            "--json",
            "--enforce",
        ])
        .unwrap();

        let AppCommand::ReleaseCheck(input) = command else {
            panic!("expected release check command");
        };

        assert_eq!(input.channel, ReleaseChannel::Experimental);
        assert!(input.inputs.apply_actions_enabled);
        assert!(input.inputs.soak_tests);
        assert!(input.inputs.stronger_tests);
        assert!(input.json);
        assert!(input.enforce);
    }

    #[test]
    fn parses_restore_and_apply_profile_commands() {
        let restore = parse_report_command(["stutter", "restore"]).unwrap();
        assert!(matches!(restore, AppCommand::Restore(input) if !input.dry_run));

        let apply = parse_report_command([
            "stutter",
            "apply-profile",
            "--tree-pid",
            "42",
            "--profile",
            "/tmp/profile.toml",
        ])
        .unwrap();

        let AppCommand::ApplyProfile(input) = apply else {
            panic!("expected apply profile command");
        };

        assert_eq!(input.tree_pid, 42);
        assert_eq!(input.profile, PathBuf::from("/tmp/profile.toml"));
        assert!(!input.force);
        assert!(!input.dry_run);
        assert!(!input.allow_medium_risk);
        assert!(!input.watch);
        assert!(!input.keep_applied);
        assert_eq!(input.refresh_ms, 1_000);
        assert!(!input.enforce);
    }

    #[test]
    fn parses_restore_dry_run() {
        let command = parse_report_command(["stutter", "restore", "--dry-run"]).unwrap();

        assert!(matches!(command, AppCommand::Restore(input) if input.dry_run));
    }

    #[test]
    fn parses_apply_profile_force_watch_and_refresh() {
        let command = parse_report_command([
            "stutter",
            "apply-profile",
            "--tree-pid",
            "42",
            "--profile",
            "/tmp/profile.toml",
            "--force",
            "--allow-medium-risk",
            "--watch",
            "--keep-applied",
            "--refresh-ms",
            "250",
            "--dry-run",
            "--enforce",
        ])
        .unwrap();

        let AppCommand::ApplyProfile(input) = command else {
            panic!("expected apply profile command");
        };

        assert_eq!(input.tree_pid, 42);
        assert_eq!(input.profile, PathBuf::from("/tmp/profile.toml"));
        assert!(input.force);
        assert!(input.allow_medium_risk);
        assert!(input.watch);
        assert!(input.keep_applied);
        assert_eq!(input.refresh_ms, 250);
        assert!(input.dry_run);
        assert!(input.enforce);
    }

    #[test]
    fn apply_profile_rejects_zero_tree_pid() {
        let err = parse_report_command([
            "stutter",
            "apply-profile",
            "--tree-pid",
            "0",
            "--profile",
            "/tmp/profile.toml",
        ])
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("--tree-pid must be greater than zero")
        );
    }

    #[test]
    fn apply_profile_keep_applied_requires_watch() {
        let err = parse_report_command([
            "stutter",
            "apply-profile",
            "--tree-pid",
            "42",
            "--profile",
            "/tmp/profile.toml",
            "--keep-applied",
        ])
        .unwrap_err();

        assert!(err.to_string().contains("--keep-applied requires --watch"));
    }

    #[test]
    fn apply_profile_rejects_zero_refresh_ms() {
        let err = parse_report_command([
            "stutter",
            "apply-profile",
            "--tree-pid",
            "42",
            "--profile",
            "/tmp/profile.toml",
            "--watch",
            "--refresh-ms",
            "0",
        ])
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("--refresh-ms must be greater than zero")
        );
    }

    #[test]
    fn parses_tune_command() {
        let command = parse_report_command([
            "stutter",
            "tune",
            "--tree-pid",
            "42",
            "--profiles",
            "/tmp/profiles.toml",
            "--epoch-seconds",
            "60",
            "--warmup-seconds",
            "10",
            "--keep-best",
            "--mangohud-log",
            "/tmp/tune-mango.csv",
        ])
        .unwrap();

        let AppCommand::Tune(input) = command else {
            panic!("expected tune command");
        };

        assert_eq!(input.tree_pid, 42);
        assert_eq!(input.profiles, PathBuf::from("/tmp/profiles.toml"));
        assert_eq!(input.epoch_seconds, 60);
        assert_eq!(input.warmup_seconds, 10);
        assert_eq!(input.runs, 3);
        assert!(input.keep_best);
        assert_eq!(input.baseline_profile, None);
        assert_eq!(input.out_dir, None);
        assert_eq!(
            input.mangohud_log,
            Some(PathBuf::from("/tmp/tune-mango.csv"))
        );
        assert!(!input.enforce);
        assert!(!input.hwmon);
    }

    #[test]
    fn parses_tune_optional_flags() {
        let command = parse_report_command([
            "stutter",
            "tune",
            "--tree-pid",
            "42",
            "--profiles",
            "/tmp/profiles.toml",
            "--epoch-seconds",
            "90",
            "--warmup-seconds",
            "15",
            "--runs",
            "5",
            "--baseline-profile",
            "stock",
            "--out-dir",
            "/tmp/tune-out",
            "--enforce",
            "--hwmon",
        ])
        .unwrap();

        let AppCommand::Tune(input) = command else {
            panic!("expected tune command");
        };

        assert_eq!(input.epoch_seconds, 90);
        assert_eq!(input.warmup_seconds, 15);
        assert_eq!(input.runs, 5);
        assert_eq!(input.baseline_profile.as_deref(), Some("stock"));
        assert_eq!(input.out_dir, Some(PathBuf::from("/tmp/tune-out")));
        assert!(input.enforce);
        assert!(input.hwmon);
    }

    #[test]
    fn tune_rejects_invalid_values() {
        for (args, expected) in [
            (
                vec![
                    "stutter",
                    "tune",
                    "--tree-pid",
                    "0",
                    "--profiles",
                    "/tmp/profiles.toml",
                ],
                "--tree-pid must be greater than zero",
            ),
            (
                vec![
                    "stutter",
                    "tune",
                    "--tree-pid",
                    "42",
                    "--profiles",
                    "/tmp/profiles.toml",
                    "--epoch-seconds",
                    "0",
                ],
                "--epoch-seconds must be greater than zero",
            ),
            (
                vec![
                    "stutter",
                    "tune",
                    "--tree-pid",
                    "42",
                    "--profiles",
                    "/tmp/profiles.toml",
                    "--epoch-seconds",
                    "10",
                    "--warmup-seconds",
                    "10",
                ],
                "--warmup-seconds must be less than --epoch-seconds",
            ),
            (
                vec![
                    "stutter",
                    "tune",
                    "--tree-pid",
                    "42",
                    "--profiles",
                    "/tmp/profiles.toml",
                    "--runs",
                    "0",
                ],
                "--runs must be greater than zero",
            ),
        ] {
            let err = crate::cli::parse_app_command_from(args).unwrap_err();

            assert!(
                err.to_string().contains(expected),
                "expected error containing {expected:?}, got {err:#}"
            );
        }
    }

    #[test]
    fn parses_rules_import_command_conversion() {
        let command = parse_report_command([
            "stutter",
            "rules",
            "import",
            "--source",
            "/tmp/ananicy-rules",
            "--name",
            "custom",
            "--license",
            "MIT",
            "--source-repo",
            "example/rules",
            "--source-commit",
            "abc123",
            "--out",
            "/tmp/ananicy.generated.json",
            "--dry-run",
        ])
        .unwrap();

        let AppCommand::Rules(input) = command else {
            panic!("expected rules command");
        };
        let RulesCommandDto::Import(import) = input.command else {
            panic!("expected rules import command");
        };

        assert_eq!(import.source, PathBuf::from("/tmp/ananicy-rules"));
        assert_eq!(import.name, "custom");
        assert_eq!(import.license, "MIT");
        assert_eq!(import.source_repo.as_deref(), Some("example/rules"));
        assert_eq!(import.source_commit.as_deref(), Some("abc123"));
        assert_eq!(
            import.out,
            Some(PathBuf::from("/tmp/ananicy.generated.json"))
        );
        assert!(import.dry_run);
    }

    #[test]
    fn parses_rules_check_command_conversion() {
        let source = parse_report_command([
            "stutter",
            "rules",
            "check",
            "--source",
            "/tmp/ananicy-rules",
        ])
        .unwrap();

        let AppCommand::Rules(input) = source else {
            panic!("expected rules command");
        };
        let RulesCommandDto::Check(check) = input.command else {
            panic!("expected rules check command");
        };

        assert_eq!(check.source, Some(PathBuf::from("/tmp/ananicy-rules")));
        assert_eq!(check.generated, None);

        let generated = parse_report_command([
            "stutter",
            "rules",
            "check",
            "--generated",
            "/tmp/ananicy.generated.json",
        ])
        .unwrap();

        let AppCommand::Rules(input) = generated else {
            panic!("expected rules command");
        };
        let RulesCommandDto::Check(check) = input.command else {
            panic!("expected rules check command");
        };

        assert_eq!(check.source, None);
        assert_eq!(
            check.generated,
            Some(PathBuf::from("/tmp/ananicy.generated.json"))
        );
    }

    #[test]
    fn parses_rules_list_status_enable_disable_and_remove_conversion() {
        let list = parse_report_command(["stutter", "rules", "list"]).unwrap();
        assert!(matches!(
            list,
            AppCommand::Rules(crate::commands::input::RulesCommandInput {
                command: RulesCommandDto::List
            })
        ));

        let status = parse_report_command(["stutter", "rules", "status"]).unwrap();
        assert!(matches!(
            status,
            AppCommand::Rules(crate::commands::input::RulesCommandInput {
                command: RulesCommandDto::Status
            })
        ));

        let enable =
            parse_report_command(["stutter", "rules", "enable", "--name", "custom"]).unwrap();
        let AppCommand::Rules(input) = enable else {
            panic!("expected rules command");
        };
        let RulesCommandDto::Enable(enable) = input.command else {
            panic!("expected rules enable command");
        };
        assert_eq!(enable.name, "custom");

        let disable = parse_report_command(["stutter", "rules", "disable"]).unwrap();
        assert!(matches!(
            disable,
            AppCommand::Rules(crate::commands::input::RulesCommandInput {
                command: RulesCommandDto::Disable
            })
        ));

        let remove = parse_report_command([
            "stutter",
            "rules",
            "remove",
            "--name",
            "custom",
            "--dry-run",
        ])
        .unwrap();
        let AppCommand::Rules(input) = remove else {
            panic!("expected rules command");
        };
        let RulesCommandDto::Remove(remove) = input.command else {
            panic!("expected rules remove command");
        };
        assert_eq!(remove.name, "custom");
        assert!(remove.dry_run);
    }

    #[test]
    fn scenario_create_parses() {
        let command = parse_report_command([
            "stutter",
            "scenario",
            "create",
            "kcd-route",
            "--duration",
            "60",
            "--watch-process",
            "KingdomCome.exe",
            "--preset",
            "diagnosis",
            "--mangohud-log",
            "/tmp/mango.csv",
            "--notes",
            "forest route",
            "--force",
        ])
        .unwrap();

        let AppCommand::Scenario(input) = command else {
            panic!("expected scenario command");
        };
        let ScenarioCommandDto::Create(input) = input.command else {
            panic!("expected scenario create command");
        };

        assert_eq!(input.name, "kcd-route");
        assert!(input.force);
        assert_eq!(input.duration, 60);
        assert_eq!(input.watch_process, Some("KingdomCome.exe".to_owned()));
        assert_eq!(input.preset, "diagnosis");
        assert_eq!(input.mangohud_log, Some(PathBuf::from("/tmp/mango.csv")));
        assert_eq!(input.notes.as_deref(), Some("forest route"));
    }

    #[test]
    fn scenario_create_rejects_zero_duration() {
        let err =
            parse_report_command(["stutter", "scenario", "create", "test", "--duration", "0"])
                .unwrap_err();

        assert!(
            err.to_string()
                .contains("scenario duration must be greater than zero")
        );
    }

    #[test]
    fn scenario_run_parses_baseline() {
        let command = parse_report_command([
            "stutter",
            "scenario",
            "run",
            "kcd-route",
            "--role",
            "baseline",
            "--dry-run",
            "--out-dir",
            "/tmp/out",
            "--mangohud-log",
            "/tmp/override.csv",
        ])
        .unwrap();

        let AppCommand::Scenario(input) = command else {
            panic!("expected scenario command");
        };
        let ScenarioCommandDto::Run(input) = input.command else {
            panic!("expected scenario run command");
        };

        assert_eq!(input.name, "kcd-route");
        assert_eq!(input.role, "baseline");
        assert!(input.dry_run);
        assert_eq!(input.out_dir, Some(PathBuf::from("/tmp/out")));
        assert_eq!(
            input.mangohud_log_override,
            Some(PathBuf::from("/tmp/override.csv"))
        );
    }

    #[test]
    fn scenario_run_parses_current() {
        let command = parse_report_command([
            "stutter",
            "scenario",
            "run",
            "kcd-route",
            "--role",
            "current",
        ])
        .unwrap();

        let AppCommand::Scenario(input) = command else {
            panic!("expected scenario command");
        };
        let ScenarioCommandDto::Run(input) = input.command else {
            panic!("expected scenario run command");
        };

        assert_eq!(input.name, "kcd-route");
        assert_eq!(input.role, "current");
    }

    #[test]
    fn scenario_run_rejects_bad_role() {
        let err = parse_report_command(["stutter", "scenario", "run", "test", "--role", "other"])
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("--role must be baseline or current")
        );
    }

    #[test]
    fn scenario_compare_parses() {
        let command = parse_report_command([
            "stutter",
            "scenario",
            "compare",
            "kcd-route",
            "--baseline",
            "/tmp/base",
            "--current",
            "/tmp/current",
            "--top",
            "5",
            "--json-summary",
            "--validate",
        ])
        .unwrap();

        let AppCommand::Scenario(input) = command else {
            panic!("expected scenario command");
        };
        let ScenarioCommandDto::Compare(input) = input.command else {
            panic!("expected scenario compare command");
        };

        assert_eq!(input.name, "kcd-route");
        assert_eq!(input.baseline, Some(PathBuf::from("/tmp/base")));
        assert_eq!(input.current, Some(PathBuf::from("/tmp/current")));
        assert_eq!(input.top, 5);
        assert!(input.json_summary);
        assert!(input.validate);
    }

    #[test]
    fn scenario_compare_rejects_top_zero() {
        let err = parse_report_command(["stutter", "scenario", "compare", "test", "--top", "0"])
            .unwrap_err();

        assert!(err.to_string().contains("--top must be greater than zero"));
    }

    #[test]
    fn scenario_path_and_list_parse() {
        let path = parse_report_command(["stutter", "scenario", "path", "kcd-route"]).unwrap();

        let AppCommand::Scenario(input) = path else {
            panic!("expected scenario command");
        };
        let ScenarioCommandDto::Path(input) = input.command else {
            panic!("expected scenario path command");
        };
        assert_eq!(input.name, "kcd-route");

        let list = parse_report_command(["stutter", "scenario", "list"]).unwrap();
        assert!(matches!(
            list,
            AppCommand::Scenario(crate::commands::input::ScenarioCommandInput {
                command: ScenarioCommandDto::List
            })
        ));
    }
}
