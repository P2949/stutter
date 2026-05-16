use super::*;

#[derive(Args, Debug, Clone)]
pub struct ManArgs {
    #[arg(long = "output", short = 'o', value_name = "PATH")]
    pub output: Option<std::path::PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub struct CompletionsArgs {
    #[arg(value_enum)]
    pub shell: clap_complete::Shell,
}

#[derive(Args, Debug, Clone)]
pub struct ProbesArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct InspectTreeArgs {
    #[arg(long = "tree-pid", value_name = "PID")]
    pub tree_pid: u32,
}

#[derive(Args, Debug, Clone)]
pub struct ReportArgs {
    #[arg(
        long,
        help = "Output raw session JSON",
        conflicts_with_all = ["analysis_json", "json_summary", "html"]
    )]
    pub json: bool,

    #[arg(
        long = "flamegraph",
        alias = "latency-flamegraph",
        value_name = "SVG",
        help = "Write a latency attribution flamegraph SVG"
    )]
    pub flamegraph: Option<PathBuf>,

    #[arg(
        long = "analysis-json",
        help = "Output full analysis JSON (clusters, diagnoses, artifacts)",
        conflicts_with_all = ["json", "json_summary", "html", "batch"]
    )]
    pub analysis_json: bool,

    #[arg(
        long = "json-summary",
        help = "Output compact summary JSON",
        conflicts_with_all = ["json", "analysis_json", "html"]
    )]
    pub json_summary: bool,

    #[arg(
        long = "html",
        value_name = "PATH",
        help = "Generate HTML report",
        conflicts_with_all = ["json", "analysis_json", "json_summary", "batch"]
    )]
    pub html: Option<PathBuf>,

    #[arg(
        long = "batch",
        value_name = "DIR",
        help = "Run report on all sessions in DIR; outputs text summary or JSON summary if --json or --json-summary is set",
        conflicts_with_all = ["analysis_json", "html"]
    )]
    pub batch: Option<PathBuf>,

    #[arg(long, default_value_t = 10, value_name = "N")]
    pub top: usize,

    #[arg(long = "cluster-ms", default_value_t = 5, value_name = "MS")]
    pub cluster_window_ms: u64,

    #[arg(
        long = "diff",
        value_name = "PATH",
        help = "Compare session(s) against baseline session at PATH"
    )]
    pub diff: Option<PathBuf>,

    #[arg(long, value_name = "CLASS")]
    pub filter_class: Option<String>,

    #[arg(
        help = "Path to session directory or session.json",
        conflicts_with = "batch"
    )]
    pub path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub struct SummaryArgs {
    #[arg(long)]
    pub json: bool,

    #[arg(long, default_value_t = 10, value_name = "N")]
    pub top: usize,

    #[arg(long, value_name = "CLASS")]
    pub filter_class: Option<String>,

    pub path: PathBuf,
}

#[derive(Args, Debug, Clone)]
pub struct RestoreArgs {
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

#[derive(Args, Debug, Clone)]
pub struct ApplyProfileArgs {
    #[arg(long = "tree-pid", value_name = "PID")]
    pub tree_pid: u32,

    #[arg(long = "profile", value_name = "FILE")]
    pub profile: PathBuf,

    #[arg(long)]
    pub force: bool,

    #[arg(long = "dry-run")]
    pub dry_run: bool,

    #[arg(long = "allow-medium-risk")]
    pub allow_medium_risk: bool,

    #[arg(long)]
    pub watch: bool,

    #[arg(long = "keep-applied")]
    pub keep_applied: bool,

    #[arg(long = "refresh-ms", default_value_t = 1_000)]
    pub refresh_ms: u64,

    #[arg(long)]
    pub enforce: bool,
}

#[derive(Args, Debug, Clone)]
#[command(
    about = "Benchmark multiple profiles and select the best one",
    long_about = "Benchmark multiple profiles and select the best one. \
                  Warning: ranking is count-based and workload-sensitive. It assumes comparable route/scene/load \
                  across epochs and will reject profiles with major scored-sample or frame-count mismatches. \
                  Use --runs 3 or higher for reliable results."
)]
pub struct TuneArgs {
    #[arg(long = "tree-pid", value_name = "PID")]
    pub tree_pid: u32,

    #[arg(long = "profiles", value_name = "FILE")]
    pub profiles: PathBuf,

    #[arg(long = "epoch-seconds", default_value_t = 120)]
    pub epoch_seconds: u64,

    #[arg(long = "warmup-seconds", default_value_t = 30)]
    pub warmup_seconds: u64,

    #[arg(long = "keep-best")]
    pub keep_best: bool,

    #[arg(long = "baseline-profile", value_name = "NAME")]
    pub baseline_profile: Option<String>,

    #[arg(long = "out-dir", value_name = "PATH")]
    pub out_dir: Option<PathBuf>,

    #[arg(long = "mangohud-log", value_name = "PATH")]
    pub mangohud_log: Option<PathBuf>,

    #[arg(long, short = 'n', default_value_t = 3, value_name = "N")]
    pub runs: u32,

    #[arg(long)]
    pub enforce: bool,

    #[arg(long = "hwmon", id = "hwmon")]
    pub hwmon: bool,
}

#[derive(Args, Debug, Clone)]
pub struct CheckArgs {
    #[arg(long = "baseline", value_name = "PATH")]
    pub baseline: PathBuf,

    #[arg(long = "current", value_name = "PATH")]
    pub current: PathBuf,

    #[arg(long = "max-regression-p99-ms", value_name = "MS")]
    pub max_regression_p99_ms: Option<f64>,

    #[arg(long = "max-max-regression-ms", value_name = "MS")]
    pub max_max_regression_ms: Option<f64>,

    #[arg(long)]
    pub json: bool,

    #[arg(long, default_value_t = 10, value_name = "N")]
    pub top: usize,

    #[arg(long, value_name = "CLASS")]
    pub filter_class: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct RecommendArgs {
    #[arg(long = "baseline", value_name = "PATH")]
    pub baseline: PathBuf,

    #[arg(long = "tune", value_name = "PATH")]
    pub tune: PathBuf,

    #[arg(long)]
    pub json: bool,

    #[arg(long, value_name = "PATH")]
    pub markdown: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub struct ReleaseArgs {
    #[command(subcommand)]
    pub command: ReleaseCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ReleaseCommand {
    Check(ReleaseCheckArgs),
}

#[derive(Args, Debug, Clone)]
pub struct ReleaseCheckArgs {
    #[arg(long = "channel", default_value = "experimental")]
    pub channel: String,

    #[arg(long = "apply-actions-enabled")]
    pub apply_actions_enabled: bool,

    #[arg(long = "soak-tests")]
    pub soak_tests: bool,

    #[arg(long = "stronger-tests")]
    pub stronger_tests: bool,

    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub enforce: bool,
}

#[derive(Args, Debug, Clone)]
pub struct AuditArgs {
    #[arg(long = "path", value_name = "PATH")]
    pub path: Option<PathBuf>,

    #[arg(long, default_value_t = 20)]
    pub tail: usize,

    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct AdvisorArgs {
    #[arg(long = "run", value_name = "PATH")]
    pub run: Option<PathBuf>,

    #[arg(long = "profiles", value_name = "PATH")]
    pub profiles: Option<PathBuf>,

    #[arg(long)]
    pub json: bool,

    #[arg(long = "watch-runs")]
    pub watch_runs: bool,

    #[arg(long = "runs-dir", value_name = "PATH")]
    pub runs_dir: Option<PathBuf>,

    #[arg(long = "poll-seconds", default_value_t = 10)]
    pub poll_seconds: u64,

    #[arg(long)]
    pub once: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DoctorArgs {
    #[arg(long)]
    pub json: bool,

    #[arg(long = "hwmon", id = "hwmon")]
    pub hwmon: bool,

    #[arg(long = "hwmon-root", value_name = "PATH", requires = "hwmon")]
    pub hwmon_root: Option<PathBuf>,

    #[arg(long = "hwmon-drm-card", value_name = "CARD", requires = "hwmon")]
    pub hwmon_drm_card: Option<String>,

    #[arg(long = "hwmon-render-node", value_name = "NODE", requires = "hwmon")]
    pub hwmon_render_node: Option<PathBuf>,

    #[arg(long = "irq-latency")]
    pub irq_latency: bool,

    #[arg(long = "irq", value_name = "IRQ")]
    pub irqs: Vec<u32>,

    #[arg(long = "block-io")]
    pub block_io: bool,

    #[arg(long = "faults")]
    pub faults: bool,

    #[arg(long = "cpu-perf")]
    pub cpu_perf: bool,

    #[arg(long = "mangohud-log", value_name = "PATH")]
    pub mangohud_log: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub struct ProfileTemplateArgs {
    #[arg(long = "topology")]
    pub topology: bool,
}

#[derive(Args, Debug, Clone)]
pub struct InspectIrqsArgs {
    #[arg(long)]
    pub json: bool,

    #[arg(long = "filter", value_name = "TEXT")]
    pub filter: Vec<String>,

    #[arg(long, default_value_t = 30)]
    pub top: usize,
}

#[derive(Args, Debug, Clone)]
pub struct RulesArgs {
    #[command(subcommand)]
    pub command: RulesCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum RulesCommand {
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
pub struct RulesCheckArgs {
    #[arg(long = "source", value_name = "PATH", conflicts_with = "generated")]
    pub source: Option<PathBuf>,

    #[arg(long = "generated", value_name = "PATH", conflicts_with = "source")]
    pub generated: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(crate) struct CliRulesImportArgs {
    #[arg(long = "source", value_name = "PATH")]
    pub source: PathBuf,

    #[arg(long, default_value = "ananicy")]
    pub name: String,

    #[arg(long, default_value = "GPL-3.0-only")]
    pub license: String,

    #[arg(long = "source-repo")]
    pub source_repo: Option<String>,

    #[arg(long = "source-commit")]
    pub source_commit: Option<String>,

    #[arg(long = "out", value_name = "PATH")]
    pub out: Option<PathBuf>,

    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

#[derive(Args, Debug, Clone)]
pub struct RulesListArgs {}

#[derive(Args, Debug, Clone)]
pub struct RulesStatusArgs {}

#[derive(Args, Debug, Clone)]
pub struct RulesEnableArgs {
    #[arg(long = "name", default_value = "ananicy")]
    pub name: String,
}

#[derive(Args, Debug, Clone)]
pub struct RulesDisableArgs {}

#[derive(Args, Debug, Clone)]
pub struct RulesRemoveArgs {
    #[arg(long = "name", default_value = "ananicy")]
    pub name: String,

    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

#[derive(Args, Debug, Clone)]
pub struct ScenarioArgs {
    #[command(subcommand)]
    pub command: ScenarioCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum ScenarioCommand {
    Create(ScenarioCreateArgs),
    Run(ScenarioRunArgs),
    Compare(ScenarioCompareArgs),
    Path(ScenarioPathArgs),
    List,
}

#[derive(Args, Debug, Clone)]
pub struct ScenarioCreateArgs {
    pub name: String,

    #[arg(long = "force")]
    pub force: bool,

    #[arg(long = "watch-process", value_name = "COMM")]
    pub watch_process: Option<String>,

    #[arg(long = "duration", default_value_t = 180)]
    pub duration: u64,

    #[arg(long = "preset", default_value = "diagnosis")]
    pub preset: String,

    #[arg(long = "mangohud-log", value_name = "PATH")]
    pub mangohud_log: Option<PathBuf>,

    #[arg(long = "notes")]
    pub notes: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct ScenarioRunArgs {
    pub name: String,

    #[arg(long = "role", value_name = "baseline|current")]
    pub role: String,

    #[arg(long = "dry-run")]
    pub dry_run: bool,

    #[arg(long = "out-dir", value_name = "PATH")]
    pub out_dir: Option<PathBuf>,

    #[arg(long = "mangohud-log", value_name = "PATH")]
    pub mangohud_log_override: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub struct ScenarioCompareArgs {
    pub name: String,

    #[arg(long = "baseline", value_name = "RUN_DIR")]
    pub baseline: Option<PathBuf>,

    #[arg(long = "current", value_name = "RUN_DIR")]
    pub current: Option<PathBuf>,

    #[arg(long, default_value_t = 10)]
    pub top: usize,

    #[arg(long = "json-summary")]
    pub json_summary: bool,

    #[arg(long = "validate")]
    pub validate: bool,
}

#[derive(Args, Debug, Clone)]
pub struct ScenarioPathArgs {
    pub name: String,
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
