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

    #[arg(long = "expect", value_enum)]
    pub(super) expect: Option<crate::display_path_compare::DisplayPathExpectation>,

    #[arg(long = "strict")]
    pub(super) strict: bool,

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
mod tests;
