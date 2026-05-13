use std::{ffi::OsString, path::PathBuf, sync::Arc, time::Duration};

use clap::{ArgAction, ArgMatches, Args, CommandFactory, Parser, Subcommand, parser::ValueSource};

use crate::{
    commands::input::{
        AdvisorCommandInput, AgentCommandInput, ApplyProfileCommandInput, AuditCommandInput,
        AutotuneCommandInput as AutotuneCommandDto, AutotuneGenerateProfilesCommandInput,
        AutotuneReplayCommandInput, AutotuneReplayHistoryCommandInput, AutotuneRestoreCommandInput,
        AutotuneStatusCommandInput, BenchCommandInput, CheckCommandInput, CompletionsCommandInput,
        DoctorCommandInput, InspectIrqsCommandInput, InspectTreeCommandInput, ManCommandInput,
        MonitorCommandInput, ProbesCommandInput, ProfileTemplateCommandInput,
        RecommendCommandInput, ReportCommandInput, RestoreCommandInput, RulesCommandInput,
        ScenarioCompareCommandInput, ScenarioCreateCommandInput, ScenarioListCommandInput,
        ScenarioPathCommandInput, ScenarioRunCommandInput, SummaryCommandInput, TuneCommandInput,
        ValidateCommandInput,
    },
    config::{
        CsvStreamTarget, FocusSource, ForegroundSource, TARGET_PIDS_MAX,
        effective::resolve_monitor_config_sources,
        layer::MonitorConfigLayer,
        merge::{CliOverrides, ConfigSources, DefaultConfig, PresetConfig},
        model::MonitorConfig,
    },
    process_tree::TaskClass,
};

#[derive(Parser, Debug)]
#[command(
    version = crate::metadata::build_version(),
    about = "Profile scheduler runnable latency for selected tasks"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    legacy_monitor: MonitorArgs,
}

#[cfg(test)]
mod version_tests {
    use super::*;

    #[test]
    fn clap_version_uses_build_version_metadata() {
        assert_eq!(
            Cli::command().get_version(),
            Some(crate::metadata::build_version())
        );
        assert_eq!(crate::metadata::build_git_rev(), env!("STUTTER_GIT_REV"));
    }

    #[test]
    fn autotune_cli_parses_washout_flags() {
        let cli = Cli::try_parse_from([
            "stutter",
            "autotune",
            "--washout-seconds",
            "30",
            "--washout-verify-interval-ms",
            "2000",
        ])
        .unwrap();

        let Some(Command::Autotune(args)) = cli.command else {
            panic!("expected autotune command");
        };

        assert_eq!(args.washout_seconds, 30);
        assert_eq!(args.washout_verify_interval_ms, 2_000);
        assert_eq!(
            args.min_focus_confidence,
            crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE
        );
    }

    #[test]
    fn autotune_cli_parses_min_focus_confidence() {
        let cli =
            Cli::try_parse_from(["stutter", "autotune", "--min-focus-confidence", "0.42"]).unwrap();

        let Some(Command::Autotune(args)) = cli.command else {
            panic!("expected autotune command");
        };

        assert_eq!(args.min_focus_confidence, 0.42);
    }
}

#[derive(Subcommand, Debug)]
enum Command {
    Monitor(MonitorArgs),
    Record(RecordArgs),
    Bench(BenchArgs),
    InspectTree(InspectTreeArgs),
    Report(ReportArgs),
    Summary(SummaryArgs),
    Validate(ValidateArgs),
    Restore(RestoreArgs),
    ApplyProfile(ApplyProfileArgs),
    Tune(TuneArgs),
    Recommend(RecommendArgs),
    Check(CheckArgs),
    Audit(AuditArgs),
    Advisor(AdvisorArgs),
    Doctor(DoctorArgs),
    ProfileTemplate(ProfileTemplateArgs),
    #[command(name = "inspect-irqs")]
    InspectIrqs(InspectIrqsArgs),
    Autotune(AutotuneArgs),
    #[command(name = "autotune-status")]
    AutotuneStatus(AutotuneStatusArgs),
    Agent(AgentArgs),
    #[command(name = "completions")]
    Completions(CompletionsArgs),
    #[command(name = "man")]
    Man(ManArgs),
    Probes(ProbesArgs),
    Rules(RulesArgs),
    Scenario(ScenarioArgs),
}

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
pub struct MonitorArgs {
    #[arg(long = "pid", short = 'p', value_name = "PID")]
    target_pids: Vec<u32>,

    #[arg(long = "tree-pid", value_name = "PID")]
    tree_pids: Vec<u32>,

    #[arg(long = "exclude-tree-pid", value_name = "PID")]
    exclude_tree_pids: Vec<u32>,

    #[arg(long = "summary-ms", value_name = "MS")]
    summary_period_ms: Option<u64>,

    #[arg(long = "epoch", value_name = "MS")]
    epoch_period_ms: Option<u64>,

    #[arg(long = "spike-us", value_name = "US")]
    spike_threshold_us: Option<u64>,

    #[arg(long = "alert-threshold-ms", value_name = "MS")]
    alert_threshold_ms: Option<u64>,

    #[arg(long = "alert-webhook-url", value_name = "URL")]
    alert_webhook_url: Option<String>,

    #[arg(long, short = 'v')]
    verbose: bool,

    #[arg(long = "run-name", value_name = "NAME")]
    run_name: Option<String>,

    #[arg(long = "out-dir", alias = "out", value_name = "PATH")]
    out_dir: Option<PathBuf>,

    #[arg(long = "include-comm", value_name = "PATTERN")]
    include_comm: Vec<String>,

    #[arg(long = "exclude-comm", value_name = "PATTERN")]
    exclude_comm: Vec<String>,

    #[arg(long = "keep-missing-pid")]
    keep_missing_pid: bool,

    #[arg(long = "watch-process", value_name = "COMM")]
    watch_process: Option<String>,

    #[arg(long)]
    persistent: bool,

    #[arg(long = "watch-poll-ms", default_value_t = 2_000)]
    watch_poll_ms: u64,

    #[arg(long = "watch-timeout-seconds", value_name = "SECONDS")]
    watch_timeout_seconds: Option<u64>,

    #[arg(long, value_name = "N")]
    max_tasks: Option<usize>,

    #[arg(long = "csv", value_name = "PATH")]
    csv_path: Option<PathBuf>,

    #[arg(
        long = "stream-csv",
        value_name = "PATH_OR_-",
        conflicts_with = "csv_path"
    )]
    stream_csv: Option<String>,

    #[arg(long = "irq-latency")]
    irq_latency: bool,

    #[arg(long = "irq", value_name = "IRQ")]
    irqs: Vec<u32>,

    #[arg(long = "hwmon", id = "hwmon", conflicts_with = "no_hwmon")]
    hwmon: bool,

    #[arg(long = "no-hwmon", help = "Disable GPU hwmon telemetry")]
    no_hwmon: bool,

    #[arg(long = "hwmon-root", value_name = "PATH", requires = "hwmon")]
    hwmon_root: Option<PathBuf>,

    #[arg(long = "hwmon-drm-card", value_name = "CARD", requires = "hwmon")]
    hwmon_drm_card: Option<String>,

    #[arg(long = "hwmon-render-node", value_name = "NODE", requires = "hwmon")]
    hwmon_render_node: Option<PathBuf>,

    #[arg(long = "mangohud-log", value_name = "PATH")]
    mangohud_log: Option<PathBuf>,

    #[arg(long = "mangohud-log-live", requires = "mangohud_log")]
    pub mangohud_log_live: bool,

    #[arg(long = "tui")]
    tui: bool,

    #[arg(long = "retain-intervals", value_name = "N")]
    retain_intervals: Option<usize>,

    #[arg(long = "no-record")]
    no_record: bool,

    #[arg(
        long = "cpu-freq",
        help = "Collect CPU frequency information (enabled by default for recording runs)",
        conflicts_with = "no_cpu_freq"
    )]
    cpu_freq: bool,

    #[arg(long = "no-cpu-freq", help = "Disable CPU frequency collection")]
    no_cpu_freq: bool,

    #[arg(long = "cgroupv2", value_name = "PATH")]
    cgroupv2: Option<PathBuf>,

    #[arg(long = "native-cgroup-filter", requires = "cgroupv2")]
    native_cgroup_filter: bool,

    #[arg(
        long = "follow-exec",
        default_value_t = true,
        action = ArgAction::SetTrue,
        conflicts_with = "no_follow_exec"
    )]
    follow_exec: bool,

    #[arg(long = "no-follow-exec", action = ArgAction::SetTrue)]
    no_follow_exec: bool,

    #[arg(long = "faults", conflicts_with = "no_faults")]
    faults: bool,

    #[arg(long = "no-faults", help = "Disable page fault collection")]
    no_faults: bool,

    #[arg(
        long = "cpu-perf",
        help = "Collect per-task CPU hardware counters for IPC/cache-miss diagnostics"
    )]
    cpu_perf: bool,

    #[arg(
        long = "cpu-perf-kernel",
        help = "Include kernel/hypervisor time in CPU perf counters; default is user-space only"
    )]
    cpu_perf_kernel: bool,

    #[arg(
        long = "cpu-perf-max-tasks",
        default_value_t = 128,
        value_name = "N",
        help = "Maximum active target tasks to attach CPU perf counters to"
    )]
    cpu_perf_max_tasks: usize,

    #[arg(
        long = "cpu-perf-cache-refs",
        help = "Also collect cache references so cache miss rate can be computed; otherwise only cache MPKI is computed"
    )]
    cpu_perf_cache_refs: bool,

    #[arg(long = "block-io", conflicts_with = "no_block_io")]
    block_io: bool,

    #[arg(long = "no-block-io", help = "Disable block I/O collection")]
    no_block_io: bool,

    #[arg(long = "stat-wait", conflicts_with = "no_stat_wait")]
    stat_wait: bool,

    #[arg(long = "no-stat-wait", help = "Disable stat-wait collection")]
    no_stat_wait: bool,

    #[arg(
        long = "runtime-slices",
        conflicts_with = "no_runtime_slices",
        help = "Collect per-thread CPU runtime/wait slices from procfs schedstat"
    )]
    runtime_slices: bool,

    #[arg(
        long = "no-runtime-slices",
        help = "Disable per-thread runtime-slice collection"
    )]
    no_runtime_slices: bool,

    #[arg(
        long = "runtime-slices-max-tasks",
        default_value_t = 256,
        value_name = "N"
    )]
    runtime_slices_max_tasks: usize,

    #[arg(
        long = "json-stream",
        help = "Emit scheduler spike events to stdout as newline-delimited JSON"
    )]
    pub json_stream: bool,

    #[arg(long = "metrics-port", value_name = "PORT")]
    pub metrics_port: Option<u16>,

    #[arg(
        long = "preset",
        value_name = "NAME",
        help = "Apply named monitor defaults: gaming, recording, diagnosis, lightweight"
    )]
    pub preset: Option<String>,

    #[arg(long = "ringbuf-size-kb", value_name = "KB")]
    pub ringbuf_size_kb: Option<u32>,

    #[arg(long = "wakeup-map-factor", value_name = "N")]
    pub wakeup_map_factor: Option<u32>,

    #[arg(long = "otlp-endpoint", value_name = "URL")]
    pub otlp_endpoint: Option<String>,

    #[arg(long = "otel-service-name", default_value = "stutter")]
    pub otel_service_name: String,

    #[arg(long = "auto-focus")]
    auto_focus: bool,

    #[arg(
        long = "focus-source",
        value_enum,
        default_value_t = FocusSource::Heuristic,
        help = "Auto-focus source: heuristic, foreground, or hybrid"
    )]
    focus_source: FocusSource,

    #[arg(
        long = "foreground-window",
        help = "Record foreground-window events even when explicit targets are used"
    )]
    foreground_window: bool,

    #[arg(
        long = "foreground-source",
        value_enum,
        default_value_t = ForegroundSource::Auto,
        help = "Foreground-window provider: auto, sway, hyprland, x11"
    )]
    foreground_source: ForegroundSource,

    #[arg(long = "foreground-poll-ms", default_value_t = 1000)]
    foreground_poll_ms: u64,

    #[arg(long = "foreground-max-stale-ms", default_value_t = 2500)]
    foreground_max_stale_ms: u64,

    #[arg(long = "foreground-include-title")]
    foreground_include_title: bool,

    #[arg(long = "auto-focus-poll-ms", default_value_t = 1000)]
    auto_focus_poll_ms: u64,

    #[arg(long = "auto-focus-min-confidence", default_value_t = 0.60)]
    auto_focus_min_confidence: f32,

    #[arg(long = "auto-focus-switch-cooldown-ms", default_value_t = 5000)]
    auto_focus_switch_cooldown_ms: u64,

    #[arg(long = "auto-focus-switch-margin", default_value_t = 0.20)]
    auto_focus_switch_margin: f32,

    #[arg(long = "auto-focus-required-polls", default_value_t = 2)]
    auto_focus_required_polls: u32,

    #[arg(long = "auto-focus-max-roots", default_value_t = 4)]
    auto_focus_max_roots: usize,

    #[arg(long = "remote", value_name = "URL")]
    pub remote: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct AutotuneArgs {
    #[command(subcommand)]
    pub command: Option<AutotuneCommand>,

    #[arg(long = "config", value_name = "PATH")]
    pub config: Option<PathBuf>,

    #[arg(long = "watch-process", value_name = "COMM")]
    pub watch_process: Option<String>,

    #[arg(long = "tree-pid", value_name = "PID")]
    pub tree_pid: Option<u32>,

    #[arg(long = "profiles", value_name = "FILE")]
    pub profiles: Option<PathBuf>,

    #[arg(
        long = "mode",
        default_value = "observe",
        help = "Autotune mode: observe, suggest, apply-low-risk, apply-medium-risk, or apply-high-risk. Live autotune currently supports observe, suggest, and apply-low-risk only; apply-low-risk applies CPU-affinity candidates only."
    )]
    pub mode: String,

    #[arg(long = "decision-log", value_name = "PATH")]
    pub decision_log: Option<PathBuf>,

    #[arg(long = "duration-seconds")]
    pub duration_seconds: Option<u64>,

    #[arg(
        long = "washout-seconds",
        default_value_t = crate::autotune::washout::DEFAULT_WASHOUT_SECONDS
    )]
    pub washout_seconds: u64,

    #[arg(
        long = "washout-verify-interval-ms",
        default_value_t = crate::autotune::washout::DEFAULT_WASHOUT_VERIFY_INTERVAL_MS
    )]
    pub washout_verify_interval_ms: u64,

    #[arg(long = "summary-ms", default_value_t = 1000)]
    pub summary_ms: u64,

    #[arg(long = "preset", default_value = "diagnosis")]
    pub preset: String,

    #[arg(long = "hwmon")]
    pub hwmon: bool,

    #[arg(long = "mangohud-log")]
    pub mangohud_log: Option<PathBuf>,

    #[arg(
        long = "auto-focus",
        help = "Allow autotune observe/suggest to classify the whole system and follow the selected focus group"
    )]
    pub auto_focus: bool,

    #[arg(
        long = "min-focus-confidence",
        default_value_t = crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE,
        help = "Minimum focus confidence required before live autotune can suggest or apply candidates"
    )]
    pub min_focus_confidence: f32,

    #[arg(
        long = "focus-source",
        value_enum,
        default_value_t = FocusSource::Hybrid,
        help = "Autotune focus source: heuristic, foreground, or hybrid"
    )]
    pub focus_source: FocusSource,

    #[arg(
        long = "foreground-window",
        help = "Collect foreground-window context for autotune focus classification"
    )]
    pub foreground_window: bool,

    #[arg(
        long = "foreground-source",
        value_enum,
        default_value_t = ForegroundSource::Auto,
        help = "Foreground-window provider for autotune focus: auto, sway, hyprland, x11"
    )]
    pub foreground_source: ForegroundSource,

    #[arg(long = "foreground-poll-ms", default_value_t = 1000)]
    pub foreground_poll_ms: u64,

    #[arg(long = "foreground-max-stale-ms", default_value_t = 2500)]
    pub foreground_max_stale_ms: u64,

    #[arg(
        long = "allow-system-wide-actions",
        help = "Reserved for future use; currently rejected so autotune cannot mutate arbitrary system processes"
    )]
    pub allow_system_wide_actions: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum AutotuneCommand {
    #[command(name = "generate-profiles")]
    GenerateProfiles(AutotuneGenerateProfilesArgs),
    Replay(AutotuneReplayArgs),

    ReplayHistory(AutotuneReplayHistoryArgs),

    Restore(AutotuneRestoreArgs),
}

#[derive(Args, Debug, Clone)]
pub struct AutotuneGenerateProfilesArgs {
    #[arg(long = "watch-process", value_name = "COMM")]
    pub watch_process: Option<String>,

    #[arg(long = "out", value_name = "PATH_OR_-")]
    pub out: PathBuf,

    #[arg(long = "allow-cpus", value_name = "CPU_LIST")]
    pub allow_cpus: Option<String>,

    #[arg(long = "deny-cpus", value_name = "CPU_LIST")]
    pub deny_cpus: Option<String>,

    #[arg(long = "min-render-cpus", default_value_t = 1)]
    pub min_render_cpus: usize,

    #[arg(long = "min-game-cpus", default_value_t = 1)]
    pub min_game_cpus: usize,

    #[arg(long = "min-compositor-cpus", default_value_t = 1)]
    pub min_compositor_cpus: usize,

    #[arg(long = "min-background-cpus", default_value_t = 2)]
    pub min_background_cpus: usize,
}

#[derive(Args, Debug, Clone)]
pub struct AutotuneRestoreArgs {
    #[arg(
        long = "journal",
        value_name = "PATH",
        help = "Path to autotune controller_journal.json; defaults to ~/.local/state/stutter/autotune/controller_journal.json"
    )]
    pub journal: Option<PathBuf>,

    #[arg(
        long = "audit",
        value_name = "PATH",
        help = "Path to audit JSONL output; defaults to the normal stutter audit log"
    )]
    pub audit: Option<PathBuf>,

    #[arg(
        long = "history",
        value_name = "PATH",
        help = "Path to autotune history JSONL output; defaults to the normal autotune history log"
    )]
    pub history: Option<PathBuf>,

    #[arg(long = "dry-run")]
    pub dry_run: bool,
}

#[derive(Args, Debug, Clone)]
pub struct AutotuneReplayHistoryArgs {
    #[arg(value_name = "HISTORY_JSONL")]
    pub history: PathBuf,
}

#[derive(Args, Debug, Clone)]
pub struct AutotuneReplayArgs {
    #[arg(long = "run", value_name = "RUN_DIR")]
    pub run: PathBuf,

    #[arg(long = "config", value_name = "AUTOTUNE_TOML")]
    pub config: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub struct AgentArgs {
    #[arg(long = "bind", default_value = "127.0.0.1:9899")]
    pub bind: std::net::SocketAddr,

    #[arg(long = "port", value_name = "PORT")]
    pub port: Option<u16>,

    #[arg(long = "runs-dir", value_name = "PATH")]
    pub runs_dir: Option<std::path::PathBuf>,

    #[arg(
        long = "allow-unsafe-bind",
        help = "Allow binding the agent to a non-loopback address. Dangerous unless the network is trusted."
    )]
    pub allow_unsafe_bind: bool,

    #[arg(
        long = "bearer-token-env",
        value_name = "ENV",
        default_value = "STUTTER_AGENT_TOKEN",
        help = "Environment variable containing bearer token for agent HTTP API"
    )]
    pub bearer_token_env: String,

    #[arg(
        long = "bearer-token-file",
        value_name = "PATH",
        help = "Read bearer token for agent HTTP API from this file"
    )]
    pub bearer_token_file: Option<PathBuf>,

    #[arg(
        long = "max-duration-seconds",
        default_value_t = crate::agent::DEFAULT_AGENT_MAX_DURATION_SECONDS,
        value_name = "SECONDS"
    )]
    pub max_duration_seconds: u64,

    #[arg(
        long = "max-targets",
        default_value_t = crate::agent::DEFAULT_AGENT_MAX_TARGETS,
        value_name = "N"
    )]
    pub max_targets: usize,

    #[arg(
        long = "max-concurrent-recordings",
        default_value_t = crate::agent::DEFAULT_AGENT_MAX_CONCURRENT_RECORDINGS,
        value_name = "N"
    )]
    pub max_concurrent_recordings: usize,
}

#[derive(Debug, Clone, Copy, Default)]
struct MonitorArgPresence {
    watch_poll_ms: bool,
    follow_exec: bool,
    cpu_perf_max_tasks: bool,
    runtime_slices_max_tasks: bool,
    otel_service_name: bool,
    focus_source: bool,
    foreground_source: bool,
    foreground_poll_ms: bool,
    foreground_max_stale_ms: bool,
    auto_focus_poll_ms: bool,
    auto_focus_min_confidence: bool,
    auto_focus_switch_cooldown_ms: bool,
    auto_focus_switch_margin: bool,
    auto_focus_required_polls: bool,
    auto_focus_max_roots: bool,
}

impl MonitorArgPresence {
    fn from_matches(matches: &ArgMatches) -> Self {
        fn command_line(matches: &ArgMatches, id: &str) -> bool {
            matches.value_source(id) == Some(ValueSource::CommandLine)
        }

        Self {
            watch_poll_ms: command_line(matches, "watch_poll_ms"),
            follow_exec: command_line(matches, "follow_exec"),
            cpu_perf_max_tasks: command_line(matches, "cpu_perf_max_tasks"),
            runtime_slices_max_tasks: command_line(matches, "runtime_slices_max_tasks"),
            otel_service_name: command_line(matches, "otel_service_name"),
            focus_source: command_line(matches, "focus_source"),
            foreground_source: command_line(matches, "foreground_source"),
            foreground_poll_ms: command_line(matches, "foreground_poll_ms"),
            foreground_max_stale_ms: command_line(matches, "foreground_max_stale_ms"),
            auto_focus_poll_ms: command_line(matches, "auto_focus_poll_ms"),
            auto_focus_min_confidence: command_line(matches, "auto_focus_min_confidence"),
            auto_focus_switch_cooldown_ms: command_line(matches, "auto_focus_switch_cooldown_ms"),
            auto_focus_switch_margin: command_line(matches, "auto_focus_switch_margin"),
            auto_focus_required_polls: command_line(matches, "auto_focus_required_polls"),
            auto_focus_max_roots: command_line(matches, "auto_focus_max_roots"),
        }
    }

    fn autotune_monitor_defaults() -> Self {
        Self {
            focus_source: true,
            auto_focus_min_confidence: true,
            auto_focus_required_polls: true,
            auto_focus_max_roots: true,
            ..Self::default()
        }
    }
}

impl MonitorArgs {
    fn into_monitor_config_layer(self, presence: MonitorArgPresence) -> MonitorConfigLayer {
        MonitorConfigLayer {
            target_pids: (!self.target_pids.is_empty()).then(|| self.target_pids.clone()),
            tree_pids: (!self.tree_pids.is_empty()).then(|| self.tree_pids.clone()),
            exclude_tree_pids: (!self.exclude_tree_pids.is_empty())
                .then(|| self.exclude_tree_pids.clone()),
            summary_period_ms: self.summary_period_ms,
            epoch_period_ms: self.epoch_period_ms.map(Some),
            spike_threshold_ns: self
                .spike_threshold_us
                .map(|value| value.saturating_mul(1_000)),
            alert_threshold_ns: self
                .alert_threshold_ms
                .map(|value| Some(value.saturating_mul(1_000_000))),
            alert_webhook_url: self.alert_webhook_url.clone().map(Some),
            verbose: self.verbose.then_some(true),
            watch_poll_ms: presence.watch_poll_ms.then_some(self.watch_poll_ms),
            watch_timeout: self
                .watch_timeout_seconds
                .map(|seconds| Some(Duration::from_secs(seconds))),
            include_comm: (!self.include_comm.is_empty()).then(|| self.include_comm.clone()),
            exclude_comm: (!self.exclude_comm.is_empty()).then(|| self.exclude_comm.clone()),
            keep_missing_pid: self.keep_missing_pid.then_some(true),
            watch_process: self.watch_process.clone().map(Some),
            persistent: self.persistent.then_some(true),
            max_tasks: self.max_tasks,
            csv_stream: match (&self.csv_path, &self.stream_csv) {
                (Some(path), None) => Some(Some(CsvStreamTarget::File(path.clone()))),
                (None, Some(value)) if value == "-" => Some(Some(CsvStreamTarget::Stdout)),
                (None, Some(value)) if value.trim().is_empty() => None,
                (None, Some(value)) => Some(Some(CsvStreamTarget::File(PathBuf::from(value)))),
                (None, None) => None,
                (Some(_), Some(_)) => None,
            },
            irq_latency: self.irq_latency.then_some(true),
            irqs: (!self.irqs.is_empty()).then(|| self.irqs.clone()),
            hwmon: if self.no_hwmon {
                Some(false)
            } else if self.hwmon {
                Some(true)
            } else {
                None
            },
            hwmon_root: self.hwmon_root.clone().map(Some),
            hwmon_drm_card: self.hwmon_drm_card.clone().map(Some),
            hwmon_render_node: self.hwmon_render_node.clone().map(Some),
            cpu_freq: if self.no_cpu_freq {
                Some(false)
            } else if self.cpu_freq {
                Some(true)
            } else {
                None
            },
            cgroupv2: self.cgroupv2.clone().map(Some),
            native_cgroup_filter: self.native_cgroup_filter.then_some(true),
            follow_exec: if self.no_follow_exec {
                Some(false)
            } else if presence.follow_exec {
                Some(self.follow_exec)
            } else {
                None
            },
            faults: if self.no_faults {
                Some(false)
            } else if self.faults {
                Some(true)
            } else {
                None
            },
            cpu_perf: self.cpu_perf.then_some(true),
            cpu_perf_kernel: self.cpu_perf_kernel.then_some(true),
            cpu_perf_max_tasks: presence
                .cpu_perf_max_tasks
                .then_some(self.cpu_perf_max_tasks),
            cpu_perf_cache_refs: self.cpu_perf_cache_refs.then_some(true),
            block_io: if self.no_block_io {
                Some(false)
            } else if self.block_io {
                Some(true)
            } else {
                None
            },
            stat_wait: if self.no_stat_wait {
                Some(false)
            } else if self.stat_wait {
                Some(true)
            } else {
                None
            },
            runtime_slices: if self.no_runtime_slices {
                Some(false)
            } else if self.runtime_slices {
                Some(true)
            } else {
                None
            },
            runtime_slices_max_tasks: presence
                .runtime_slices_max_tasks
                .then_some(self.runtime_slices_max_tasks),
            mangohud_log: self.mangohud_log.clone().map(Some),
            mangohud_log_live: self.mangohud_log_live.then_some(true),
            tui: self.tui.then_some(true),
            json_stream: self.json_stream.then_some(true),
            metrics_port: self.metrics_port.map(Some),
            ringbuf_size_kb: self.ringbuf_size_kb.map(Some),
            wakeup_map_factor: self.wakeup_map_factor.map(Some),
            otlp_endpoint: self.otlp_endpoint.clone().map(Some),
            otel_service_name: presence
                .otel_service_name
                .then(|| self.otel_service_name.clone()),
            auto_focus: self.auto_focus.then_some(true),
            focus_source: presence.focus_source.then_some(self.focus_source),
            foreground_window: self.foreground_window.then_some(true),
            foreground_source: presence.foreground_source.then_some(self.foreground_source),
            foreground_poll_ms: presence
                .foreground_poll_ms
                .then_some(self.foreground_poll_ms),
            foreground_max_stale_ms: presence
                .foreground_max_stale_ms
                .then_some(self.foreground_max_stale_ms),
            foreground_include_title: self.foreground_include_title.then_some(true),
            auto_focus_poll_ms: presence
                .auto_focus_poll_ms
                .then_some(self.auto_focus_poll_ms),
            auto_focus_min_confidence: presence
                .auto_focus_min_confidence
                .then_some(self.auto_focus_min_confidence),
            auto_focus_switch_cooldown_ms: presence
                .auto_focus_switch_cooldown_ms
                .then_some(self.auto_focus_switch_cooldown_ms),
            auto_focus_switch_margin: presence
                .auto_focus_switch_margin
                .then_some(self.auto_focus_switch_margin),
            auto_focus_required_polls: presence
                .auto_focus_required_polls
                .then_some(self.auto_focus_required_polls),
            auto_focus_max_roots: presence
                .auto_focus_max_roots
                .then_some(self.auto_focus_max_roots),
            retain_intervals: self.retain_intervals.map(Some),
            run_name: self.run_name.clone().map(Some),
            output_dir: self.out_dir.clone().map(Some),
            remote: self.remote.clone().map(Some),
            ..MonitorConfigLayer::default()
        }
    }
}

impl Default for MonitorArgs {
    fn default() -> Self {
        Self {
            target_pids: Vec::new(),
            tree_pids: Vec::new(),
            exclude_tree_pids: Vec::new(),
            summary_period_ms: None,
            epoch_period_ms: None,
            spike_threshold_us: None,
            alert_threshold_ms: None,
            alert_webhook_url: None,
            verbose: false,
            run_name: None,
            out_dir: None,
            include_comm: Vec::new(),
            exclude_comm: Vec::new(),
            keep_missing_pid: false,
            watch_process: None,
            persistent: false,
            watch_poll_ms: 2000,
            watch_timeout_seconds: None,
            max_tasks: None,
            csv_path: None,
            stream_csv: None,
            irq_latency: false,
            irqs: Vec::new(),
            hwmon: false,
            no_hwmon: false,
            hwmon_root: None,
            hwmon_drm_card: None,
            hwmon_render_node: None,
            mangohud_log: None,
            mangohud_log_live: false,
            tui: false,
            retain_intervals: None,
            no_record: false,
            cpu_freq: false,
            no_cpu_freq: false,
            cgroupv2: None,
            native_cgroup_filter: false,
            follow_exec: true,
            no_follow_exec: false,
            faults: false,
            no_faults: false,
            cpu_perf: false,
            cpu_perf_kernel: false,
            cpu_perf_max_tasks: 128,
            cpu_perf_cache_refs: false,
            block_io: false,
            no_block_io: false,
            stat_wait: false,
            no_stat_wait: false,
            runtime_slices: false,
            no_runtime_slices: false,
            runtime_slices_max_tasks: 256,
            json_stream: false,
            metrics_port: None,
            preset: None,
            ringbuf_size_kb: None,
            wakeup_map_factor: None,
            otlp_endpoint: None,
            otel_service_name: "stutter".to_owned(),
            auto_focus: false,
            focus_source: FocusSource::Heuristic,
            foreground_window: false,
            foreground_source: ForegroundSource::Auto,
            foreground_poll_ms: 1000,
            foreground_max_stale_ms: 2500,
            foreground_include_title: false,
            auto_focus_poll_ms: 1000,
            auto_focus_min_confidence: 0.60,
            auto_focus_switch_cooldown_ms: 5000,
            auto_focus_switch_margin: 0.20,
            auto_focus_required_polls: 2,
            auto_focus_max_roots: 4,
            remote: None,
        }
    }
}

#[derive(Args, Debug, Clone)]
struct RecordArgs {
    #[command(flatten)]
    monitor: MonitorArgs,

    #[arg(long, value_name = "SECONDS")]
    duration: Option<u64>,
}

#[derive(Args, Debug, Clone)]
pub struct BenchArgs {
    #[command(flatten)]
    pub monitor: MonitorArgs,

    #[arg(long, value_name = "SECONDS")]
    pub duration: u64,

    #[arg(long = "scenario", value_name = "NAME")]
    pub scenario: String,

    #[arg(
        long = "role",
        value_name = "baseline|current",
        default_value = "baseline"
    )]
    pub role: String,
}

#[derive(Args, Debug, Clone)]
struct InspectTreeArgs {
    #[arg(long = "tree-pid", value_name = "PID")]
    tree_pid: u32,
}

#[derive(Args, Debug, Clone)]
struct ReportArgs {
    #[arg(
        long,
        help = "Output raw session JSON",
        conflicts_with_all = ["analysis_json", "json_summary", "html"]
    )]
    json: bool,

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
    analysis_json: bool,

    #[arg(
        long = "json-summary",
        help = "Output compact summary JSON",
        conflicts_with_all = ["json", "analysis_json", "html"]
    )]
    json_summary: bool,

    #[arg(
        long = "html",
        value_name = "PATH",
        help = "Generate HTML report",
        conflicts_with_all = ["json", "analysis_json", "json_summary", "batch"]
    )]
    html: Option<PathBuf>,

    #[arg(
        long = "batch",
        value_name = "DIR",
        help = "Run report on all sessions in DIR; outputs text summary or JSON summary if --json or --json-summary is set",
        conflicts_with_all = ["analysis_json", "html"]
    )]
    batch: Option<PathBuf>,

    #[arg(long, default_value_t = 10, value_name = "N")]
    top: usize,

    #[arg(long = "cluster-ms", default_value_t = 5, value_name = "MS")]
    cluster_window_ms: u64,

    #[arg(
        long = "diff",
        value_name = "PATH",
        help = "Compare session(s) against baseline session at PATH"
    )]
    diff: Option<PathBuf>,

    #[arg(long = "filter-class", value_name = "CLASS")]
    filter_class: Option<String>,

    #[arg(
        help = "Path to session directory or session.json",
        conflicts_with = "batch"
    )]
    path: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
struct SummaryArgs {
    #[arg(long)]
    json: bool,

    #[arg(long, default_value_t = 10, value_name = "N")]
    top: usize,

    #[arg(long = "filter-class", value_name = "CLASS")]
    filter_class: Option<String>,

    path: PathBuf,
}

#[derive(Args, Debug, Clone)]
pub struct ValidateArgs {
    #[arg(help = "Path to run directory or session.json")]
    pub path: PathBuf,

    #[arg(long)]
    pub json: bool,

    #[arg(long, help = "Treat warnings and medium-quality data as failures")]
    pub strict: bool,
}

#[derive(Args, Debug, Clone)]
struct RestoreArgs {
    #[arg(long = "dry-run")]
    dry_run: bool,
}

#[derive(Args, Debug, Clone)]
struct ApplyProfileArgs {
    #[arg(long = "tree-pid", value_name = "PID")]
    tree_pid: u32,

    #[arg(long = "profile", value_name = "FILE")]
    profile: PathBuf,

    #[arg(long)]
    force: bool,

    #[arg(long = "dry-run")]
    dry_run: bool,

    #[arg(long = "allow-medium-risk")]
    allow_medium_risk: bool,

    #[arg(long)]
    watch: bool,

    #[arg(long = "keep-applied")]
    keep_applied: bool,

    #[arg(long = "refresh-ms", default_value_t = 1_000)]
    refresh_ms: u64,

    #[arg(long)]
    enforce: bool,
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

    #[arg(long = "runs", short = 'n', default_value_t = 3, value_name = "N")]
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

    #[arg(long = "filter-class", value_name = "CLASS")]
    pub filter_class: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct RecommendArgs {
    #[arg(long = "baseline", value_name = "PATH")]
    pub baseline: PathBuf,

    #[arg(long = "tune", value_name = "PATH")]
    pub tune: PathBuf,

    #[arg(long = "json")]
    pub json: bool,

    #[arg(long = "markdown", value_name = "PATH")]
    pub markdown: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub struct AutotuneStatusArgs {
    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct AuditArgs {
    #[arg(long = "path", value_name = "PATH")]
    pub path: Option<PathBuf>,

    #[arg(long = "tail", default_value_t = 20)]
    pub tail: usize,

    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct AdvisorArgs {
    #[arg(long = "run", value_name = "PATH")]
    pub run: Option<PathBuf>,

    #[arg(long = "profiles", value_name = "PATH")]
    pub profiles: Option<PathBuf>,

    #[arg(long = "json")]
    pub json: bool,

    #[arg(long = "watch-runs")]
    pub watch_runs: bool,

    #[arg(long = "runs-dir", value_name = "PATH")]
    pub runs_dir: Option<PathBuf>,

    #[arg(long = "poll-seconds", default_value_t = 10)]
    pub poll_seconds: u64,

    #[arg(long = "once")]
    pub once: bool,
}

#[derive(Args, Debug, Clone)]
struct DoctorArgs {
    #[arg(long)]
    json: bool,

    #[arg(long = "hwmon", id = "hwmon")]
    hwmon: bool,

    #[arg(long = "hwmon-root", value_name = "PATH", requires = "hwmon")]
    hwmon_root: Option<PathBuf>,

    #[arg(long = "hwmon-drm-card", value_name = "CARD", requires = "hwmon")]
    hwmon_drm_card: Option<String>,

    #[arg(long = "hwmon-render-node", value_name = "NODE", requires = "hwmon")]
    hwmon_render_node: Option<PathBuf>,

    #[arg(long = "irq-latency")]
    irq_latency: bool,

    #[arg(long = "irq", value_name = "IRQ")]
    irqs: Vec<u32>,

    #[arg(long = "block-io")]
    block_io: bool,

    #[arg(long = "faults")]
    faults: bool,

    #[arg(long = "cpu-perf")]
    cpu_perf: bool,

    #[arg(long = "mangohud-log", value_name = "PATH")]
    mangohud_log: Option<PathBuf>,
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

    #[arg(long = "top", default_value_t = 30)]
    pub top: usize,
}
#[derive(Args, Debug, Clone)]
pub struct RulesArgs {
    #[command(subcommand)]
    pub command: RulesCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum RulesCommand {
    Import(RulesImportArgs),
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
pub struct RulesImportArgs {
    #[arg(long = "source", value_name = "PATH")]
    pub source: PathBuf,

    #[arg(long = "name", default_value = "ananicy")]
    pub name: String,

    #[arg(long = "license", default_value = "GPL-3.0-only")]
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

#[derive(Debug, Clone, Copy)]
enum RecordingMode {
    Monitor,
    ForceRecording { max_duration: Option<Duration> },
}

impl RecordingMode {
    fn force_recording(self) -> bool {
        matches!(self, Self::ForceRecording { .. })
    }

    fn max_duration(self) -> Option<Duration> {
        match self {
            Self::Monitor => None,
            Self::ForceRecording { max_duration } => max_duration,
        }
    }
}

#[derive(Debug)]
pub enum AppCommand {
    Monitor(MonitorCommandInput),
    Bench(BenchCommandInput),
    Restore(RestoreCommandInput),
    ApplyProfile(ApplyProfileCommandInput),
    InspectTree(InspectTreeCommandInput),
    Summary(SummaryCommandInput),
    Validate(ValidateCommandInput),
    Report(ReportCommandInput),
    Tune(TuneCommandInput),
    Recommend(RecommendCommandInput),
    Check(CheckCommandInput),
    AutotuneGenerateProfiles(AutotuneGenerateProfilesCommandInput),
    Autotune(AutotuneCommandDto),
    AutotuneStatus(AutotuneStatusCommandInput),
    AutotuneReplayHistory(AutotuneReplayHistoryCommandInput),
    AutotuneRestore(AutotuneRestoreCommandInput),
    Audit(AuditCommandInput),
    AutotuneReplay(AutotuneReplayCommandInput),
    Advisor(AdvisorCommandInput),
    Doctor(DoctorCommandInput),
    Probes(ProbesCommandInput),
    ProfileTemplate(ProfileTemplateCommandInput),
    InspectIrqs(InspectIrqsCommandInput),
    Agent(AgentCommandInput),
    Completions(CompletionsCommandInput),
    Man(ManCommandInput),
    Rules(RulesCommandInput),
    ScenarioCreate(ScenarioCreateCommandInput),
    ScenarioRun(ScenarioRunCommandInput),
    ScenarioCompare(ScenarioCompareCommandInput),
    ScenarioPath(ScenarioPathCommandInput),
    ScenarioList(ScenarioListCommandInput),
}

impl FocusSource {
    fn parse_config_value(value: &str) -> anyhow::Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "heuristic" => Ok(Self::Heuristic),
            "foreground" => Ok(Self::Foreground),
            "hybrid" => Ok(Self::Hybrid),
            other => anyhow::bail!(
                "focus_source must be heuristic, foreground, or hybrid, got {other:?}"
            ),
        }
    }
}

impl ForegroundSource {
    fn parse_config_value(value: &str) -> anyhow::Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "sway" => Ok(Self::Sway),
            "hyprland" => Ok(Self::Hyprland),
            "x11" => Ok(Self::X11),
            other => anyhow::bail!(
                "foreground_source must be auto, sway, hyprland, or x11, got {other:?}"
            ),
        }
    }
}

fn validate_autotune_mode(mode: &str) -> anyhow::Result<()> {
    match mode {
        "observe" | "suggest" | "apply-low-risk" => Ok(()),
        _ => anyhow::bail!(
            "apply mode is not implemented yet; use --mode observe, --mode suggest, or --mode apply-low-risk"
        ),
    }
}

pub fn autotune_monitor_config(
    input: &crate::autotune::AutotuneCommandInput,
) -> anyhow::Result<Arc<MonitorConfig>> {
    if input.allow_system_wide_actions {
        anyhow::bail!(
            "autotune system-wide actions are intentionally disabled; use observe/suggest focus mode"
        );
    }

    let has_target = input.tree_pid.is_some() || input.watch_process.is_some();
    if !has_target && !input.auto_focus {
        anyhow::bail!("autotune requires --tree-pid, --watch-process, or --auto-focus");
    }

    let mut monitor = MonitorArgs {
        watch_process: input.watch_process.clone(),
        tree_pids: input.tree_pid.map_or(Vec::new(), |pid| vec![pid]),
        persistent: input.watch_process.is_some(),
        summary_period_ms: Some(input.summary_ms),
        preset: Some(input.preset.clone()),
        hwmon: input.hwmon,
        no_hwmon: !input.hwmon,
        mangohud_log: input.mangohud_log.clone(),
        no_record: true,
        run_name: Some("autotune-observe".to_owned()),
        auto_focus: input.auto_focus,
        focus_source: input.focus_source,
        foreground_window: input.foreground_window
            || input.auto_focus
            || matches!(
                input.focus_source,
                FocusSource::Foreground | FocusSource::Hybrid
            ),
        foreground_source: input.foreground_source,
        foreground_poll_ms: input.foreground_poll_ms,
        foreground_max_stale_ms: input.foreground_max_stale_ms,
        foreground_include_title: false,
        auto_focus_min_confidence: 0.70,
        auto_focus_required_polls: 3,
        auto_focus_switch_cooldown_ms: 5_000,
        auto_focus_switch_margin: 0.20,
        auto_focus_max_roots: 1,
        ..Default::default()
    };

    monitor.no_record = true;

    Ok(Arc::new(monitor_config_from_monitor_args_with_presence(
        monitor,
        RecordingMode::ForceRecording {
            max_duration: input.duration_seconds.map(Duration::from_secs),
        },
        MonitorArgPresence::autotune_monitor_defaults(),
    )?))
}

pub fn parse_app_command() -> anyhow::Result<AppCommand> {
    parse_app_command_from(std::env::args_os())
}

fn monitor_arg_presence_from_matches(
    matches: &ArgMatches,
    subcommand: Option<&str>,
) -> MonitorArgPresence {
    match subcommand {
        Some(expected) => match matches.subcommand() {
            Some((actual, sub_matches)) if actual == expected => {
                MonitorArgPresence::from_matches(sub_matches)
            }
            _ => MonitorArgPresence::default(),
        },
        None => MonitorArgPresence::from_matches(matches),
    }
}

pub fn parse_app_command_from<I, T>(args: I) -> anyhow::Result<AppCommand>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let argv: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let matches = Cli::command().try_get_matches_from(argv.clone())?;
    let cli = Cli::try_parse_from(argv)?;

    match cli.command {
        Some(Command::Monitor(args)) => Ok(AppCommand::Monitor(MonitorCommandInput {
            config: Arc::new(monitor_config_from_monitor_args_with_presence(
                args,
                RecordingMode::Monitor,
                monitor_arg_presence_from_matches(&matches, Some("monitor")),
            )?),
        })),
        Some(Command::Record(args)) => {
            if matches!(args.duration, Some(0)) {
                anyhow::bail!("--duration must be greater than zero");
            }

            if args.monitor.no_record {
                anyhow::bail!(
                    "record --no-record is contradictory; use 'monitor' for non-recording runs"
                );
            }

            let max_duration = args.duration.map(Duration::from_secs);
            Ok(AppCommand::Monitor(MonitorCommandInput {
                config: Arc::new(monitor_config_from_monitor_args_with_presence(
                    args.monitor,
                    RecordingMode::ForceRecording { max_duration },
                    monitor_arg_presence_from_matches(&matches, Some("record")),
                )?),
            }))
        }
        Some(Command::Bench(mut args)) => {
            if args.duration == 0 {
                anyhow::bail!("--duration must be greater than zero");
            }
            if args.scenario.trim().is_empty() {
                anyhow::bail!("--scenario must not be empty");
            }
            if !matches!(args.role.as_str(), "baseline" | "current") {
                anyhow::bail!("--role must be baseline or current");
            }
            if args.monitor.no_record {
                anyhow::bail!("bench --no-record is contradictory");
            }

            let run_name = format!("bench-{}-{}", args.role, args.scenario);
            args.monitor.run_name = Some(run_name.clone());
            let config = Arc::new(monitor_config_from_monitor_args_with_presence(
                args.monitor,
                RecordingMode::ForceRecording {
                    max_duration: Some(Duration::from_secs(args.duration)),
                },
                monitor_arg_presence_from_matches(&matches, Some("bench")),
            )?);
            Ok(AppCommand::Bench(BenchCommandInput {
                config,
                role: args.role,
                run_name,
            }))
        }
        Some(Command::InspectTree(args)) => {
            if args.tree_pid == 0 {
                anyhow::bail!("--tree-pid must be greater than zero");
            }
            Ok(AppCommand::InspectTree(InspectTreeCommandInput {
                tree_pid: args.tree_pid,
            }))
        }
        Some(Command::Report(args)) => {
            if args.top == 0 {
                anyhow::bail!("--top must be greater than zero");
            }
            if args.cluster_window_ms == 0 {
                anyhow::bail!("--cluster-ms must be greater than zero");
            }
            if args.batch.is_none() && args.path.is_none() {
                anyhow::bail!("report requires PATH unless --batch is set");
            }
            Ok(AppCommand::Report(ReportCommandInput {
                path: args.path,
                json: args.json,
                analysis_json: args.analysis_json,
                json_summary: args.json_summary,
                html: args.html,
                top: args.top,
                cluster_window_ms: args.cluster_window_ms,
                batch: args.batch,
                diff: args.diff,
                filter_class: parse_optional_task_class(args.filter_class.as_deref())?,
                flamegraph: args.flamegraph,
            }))
        }
        Some(Command::Summary(args)) => {
            if args.top == 0 {
                anyhow::bail!("--top must be greater than zero");
            }
            Ok(AppCommand::Summary(SummaryCommandInput {
                path: args.path,
                json: args.json,
                top: args.top,
                filter_class: parse_optional_task_class(args.filter_class.as_deref())?,
            }))
        }
        Some(Command::Validate(args)) => Ok(AppCommand::Validate(ValidateCommandInput {
            path: args.path,
            json: args.json,
            strict: args.strict,
        })),
        Some(Command::Restore(args)) => Ok(AppCommand::Restore(RestoreCommandInput {
            dry_run: args.dry_run,
        })),
        Some(Command::ApplyProfile(args)) => {
            if args.tree_pid == 0 {
                anyhow::bail!("--tree-pid must be greater than zero");
            }
            if args.refresh_ms == 0 {
                anyhow::bail!("--refresh-ms must be greater than zero");
            }
            if args.keep_applied && !args.watch {
                anyhow::bail!("--keep-applied requires --watch");
            }
            Ok(AppCommand::ApplyProfile(ApplyProfileCommandInput {
                tree_pid: args.tree_pid,
                profile: args.profile,
                force: args.force,
                dry_run: args.dry_run,
                allow_medium_risk: args.allow_medium_risk,
                watch: args.watch,
                keep_applied: args.keep_applied,
                refresh_ms: args.refresh_ms,
                enforce: args.enforce,
            }))
        }
        Some(Command::Tune(args)) => {
            if args.tree_pid == 0 {
                anyhow::bail!("--tree-pid must be greater than zero");
            }
            if args.epoch_seconds == 0 {
                anyhow::bail!("--epoch-seconds must be greater than zero");
            }
            if args.warmup_seconds >= args.epoch_seconds {
                anyhow::bail!("--warmup-seconds must be less than --epoch-seconds");
            }
            if args.runs == 0 {
                anyhow::bail!("--runs must be greater than zero");
            }
            Ok(AppCommand::Tune(TuneCommandInput {
                tree_pid: args.tree_pid,
                profiles: args.profiles,
                epoch_seconds: args.epoch_seconds,
                warmup_seconds: args.warmup_seconds,
                runs: args.runs,
                keep_best: args.keep_best,
                baseline_profile: args.baseline_profile,
                out_dir: args.out_dir,
                mangohud_log: args.mangohud_log,
                enforce: args.enforce,
                hwmon: args.hwmon,
            }))
        }
        Some(Command::Recommend(args)) => Ok(AppCommand::Recommend(RecommendCommandInput {
            baseline: args.baseline,
            tune: args.tune,
            json: args.json,
            markdown: args.markdown,
        })),
        Some(Command::Check(args)) => {
            if args.max_regression_p99_ms.is_none() && args.max_max_regression_ms.is_none() {
                anyhow::bail!(
                    "check requires at least one threshold: --max-regression-p99-ms or --max-max-regression-ms"
                );
            }
            if let Some(value) = args.max_regression_p99_ms
                && (!value.is_finite() || value < 0.0)
            {
                anyhow::bail!("--max-regression-p99-ms must be a finite non-negative value");
            }
            if let Some(value) = args.max_max_regression_ms
                && (!value.is_finite() || value < 0.0)
            {
                anyhow::bail!("--max-max-regression-ms must be a finite non-negative value");
            }
            if args.top == 0 {
                anyhow::bail!("--top must be greater than zero");
            }
            Ok(AppCommand::Check(CheckCommandInput {
                baseline: args.baseline,
                current: args.current,
                max_regression_p99_ms: args.max_regression_p99_ms,
                max_max_regression_ms: args.max_max_regression_ms,
                json: args.json,
                top: args.top,
                filter_class: parse_optional_task_class(args.filter_class.as_deref())?,
            }))
        }
        Some(Command::Autotune(args)) => {
            if let Some(cmd) = args.command {
                match cmd {
                    AutotuneCommand::GenerateProfiles(args) => {
                        Ok(AppCommand::AutotuneGenerateProfiles(
                            AutotuneGenerateProfilesCommandInput {
                                watch_process: args.watch_process,
                                out: args.out,
                                allow_cpus: args.allow_cpus,
                                deny_cpus: args.deny_cpus,
                                min_render_cpus: args.min_render_cpus,
                                min_game_cpus: args.min_game_cpus,
                                min_compositor_cpus: args.min_compositor_cpus,
                                min_background_cpus: args.min_background_cpus,
                            },
                        ))
                    }
                    AutotuneCommand::Replay(replay) => {
                        Ok(AppCommand::AutotuneReplay(AutotuneReplayCommandInput {
                            run: replay.run,
                            config: replay.config,
                        }))
                    }
                    AutotuneCommand::ReplayHistory(replay_args) => Ok(
                        AppCommand::AutotuneReplayHistory(AutotuneReplayHistoryCommandInput {
                            history: replay_args.history,
                        }),
                    ),
                    AutotuneCommand::Restore(args) => {
                        Ok(AppCommand::AutotuneRestore(AutotuneRestoreCommandInput {
                            journal: args.journal,
                            audit: args.audit,
                            history: args.history,
                            dry_run: args.dry_run,
                        }))
                    }
                }
            } else {
                if args.allow_system_wide_actions {
                    anyhow::bail!(
                        "--allow-system-wide-actions is reserved for future use and is intentionally rejected"
                    );
                }

                validate_autotune_mode(&args.mode)?;
                Ok(AppCommand::Autotune(AutotuneCommandDto {
                    input: crate::autotune::AutotuneCommandInput {
                        config: args.config,
                        watch_process: args.watch_process,
                        tree_pid: args.tree_pid,
                        profiles: args.profiles,
                        mode: args.mode,
                        decision_log: args.decision_log,
                        duration_seconds: args.duration_seconds,
                        washout_seconds: args.washout_seconds,
                        washout_verify_interval_ms: args.washout_verify_interval_ms,
                        summary_ms: args.summary_ms,
                        preset: args.preset,
                        hwmon: args.hwmon,
                        mangohud_log: args.mangohud_log,
                        auto_focus: args.auto_focus,
                        min_focus_confidence: args.min_focus_confidence,
                        focus_source: args.focus_source,
                        foreground_window: args.foreground_window,
                        foreground_source: args.foreground_source,
                        foreground_poll_ms: args.foreground_poll_ms,
                        foreground_max_stale_ms: args.foreground_max_stale_ms,
                        allow_system_wide_actions: args.allow_system_wide_actions,
                    },
                }))
            }
        }
        Some(Command::AutotuneStatus(args)) => {
            Ok(AppCommand::AutotuneStatus(AutotuneStatusCommandInput {
                json: args.json,
            }))
        }
        Some(Command::Audit(args)) => Ok(AppCommand::Audit(AuditCommandInput {
            path: args.path,
            tail: args.tail,
            json: args.json,
        })),
        Some(Command::Advisor(args)) => {
            if args.watch_runs && args.run.is_some() {
                anyhow::bail!("--watch-runs conflicts with --run");
            }
            if !args.watch_runs && args.run.is_none() {
                anyhow::bail!("advisor requires --run unless --watch-runs is set");
            }
            if args.poll_seconds == 0 {
                anyhow::bail!("--poll-seconds must be greater than zero");
            }
            Ok(AppCommand::Advisor(AdvisorCommandInput {
                run: args.run,
                profiles: args.profiles,
                json: args.json,
                watch_runs: args.watch_runs,
                runs_dir: args.runs_dir,
                poll_seconds: args.poll_seconds,
                once: args.once,
            }))
        }
        Some(Command::Doctor(args)) => Ok(AppCommand::Doctor(DoctorCommandInput {
            input: crate::doctor::DoctorInput {
                json: args.json,
                hwmon: args.hwmon,
                hwmon_root: args.hwmon_root,
                hwmon_drm_card: args.hwmon_drm_card,
                hwmon_render_node: args.hwmon_render_node,
                irq_latency: args.irq_latency,
                irqs: args.irqs,
                block_io: args.block_io,
                faults: args.faults,
                cpu_perf: args.cpu_perf,
                mangohud_log: args.mangohud_log,
            },
        })),
        Some(Command::ProfileTemplate(args)) => {
            Ok(AppCommand::ProfileTemplate(ProfileTemplateCommandInput {
                topology: args.topology,
            }))
        }
        Some(Command::InspectIrqs(args)) => {
            if args.top == 0 {
                anyhow::bail!("--top must be greater than zero");
            }
            Ok(AppCommand::InspectIrqs(InspectIrqsCommandInput {
                json: args.json,
                filter: args.filter.clone(),
                top: args.top,
            }))
        }
        None => Ok(AppCommand::Monitor(MonitorCommandInput {
            config: Arc::new(monitor_config_from_monitor_args_with_presence(
                cli.legacy_monitor,
                RecordingMode::Monitor,
                monitor_arg_presence_from_matches(&matches, None),
            )?),
        })),
        Some(Command::Agent(args)) => {
            if args.max_duration_seconds == 0 {
                anyhow::bail!("--max-duration-seconds must be greater than zero");
            }
            if args.max_targets == 0 {
                anyhow::bail!("--max-targets must be greater than zero");
            }
            if args.max_concurrent_recordings == 0 {
                anyhow::bail!("--max-concurrent-recordings must be greater than zero");
            }
            if args.max_concurrent_recordings > 1 {
                anyhow::bail!("agent currently supports at most 1 concurrent recording");
            }

            let bind = if let Some(port) = args.port {
                std::net::SocketAddr::from(([127, 0, 0, 1], port))
            } else {
                args.bind
            };
            Ok(AppCommand::Agent(AgentCommandInput {
                bind,
                runs_dir: args.runs_dir,
                allow_unsafe_bind: args.allow_unsafe_bind,
                bearer_token_env: args.bearer_token_env,
                bearer_token_file: args.bearer_token_file,
                max_duration_seconds: args.max_duration_seconds,
                max_targets: args.max_targets,
                max_concurrent_recordings: args.max_concurrent_recordings,
            }))
        }
        Some(Command::Completions(args)) => Ok(AppCommand::Completions(CompletionsCommandInput {
            shell: args.shell,
        })),
        Some(Command::Man(args)) => Ok(AppCommand::Man(ManCommandInput {
            output: args.output,
        })),
        Some(Command::Probes(args)) => {
            Ok(AppCommand::Probes(ProbesCommandInput { json: args.json }))
        }
        Some(Command::Rules(args)) => Ok(AppCommand::Rules(RulesCommandInput {
            command: args.command,
        })),
        Some(Command::Scenario(args)) => match args.command {
            ScenarioCommand::Create(args) => {
                if args.name.trim().is_empty() {
                    anyhow::bail!("scenario name must not be empty");
                }
                if args.duration == 0 {
                    anyhow::bail!("scenario duration must be greater than zero");
                }
                Ok(AppCommand::ScenarioCreate(ScenarioCreateCommandInput {
                    name: args.name,
                    force: args.force,
                    watch_process: args.watch_process,
                    duration: args.duration,
                    preset: args.preset,
                    mangohud_log: args.mangohud_log,
                    notes: args.notes,
                }))
            }
            ScenarioCommand::Run(args) => {
                if args.name.trim().is_empty() {
                    anyhow::bail!("scenario name must not be empty");
                }
                if !matches!(args.role.as_str(), "baseline" | "current") {
                    anyhow::bail!("--role must be baseline or current");
                }
                Ok(AppCommand::ScenarioRun(ScenarioRunCommandInput {
                    name: args.name,
                    role: args.role,
                    dry_run: args.dry_run,
                    out_dir: args.out_dir,
                    mangohud_log_override: args.mangohud_log_override,
                }))
            }
            ScenarioCommand::Compare(args) => {
                if args.name.trim().is_empty() {
                    anyhow::bail!("scenario name must not be empty");
                }
                if args.top == 0 {
                    anyhow::bail!("--top must be greater than zero");
                }
                Ok(AppCommand::ScenarioCompare(ScenarioCompareCommandInput {
                    name: args.name,
                    baseline: args.baseline,
                    current: args.current,
                    top: args.top,
                    json_summary: args.json_summary,
                    validate: args.validate,
                }))
            }
            ScenarioCommand::Path(args) => {
                if args.name.trim().is_empty() {
                    anyhow::bail!("scenario name must not be empty");
                }
                Ok(AppCommand::ScenarioPath(ScenarioPathCommandInput {
                    name: args.name,
                }))
            }
            ScenarioCommand::List => Ok(AppCommand::ScenarioList(ScenarioListCommandInput)),
        },
    }
}

#[cfg(test)]
mod auto_focus_cli_tests {
    use super::*;

    #[test]
    fn monitor_args_default_contains_auto_focus_defaults() {
        let args = MonitorArgs::default();

        assert!(!args.auto_focus);
        assert_eq!(args.auto_focus_poll_ms, 1000);
        assert_eq!(args.auto_focus_min_confidence, 0.60);
        assert_eq!(args.auto_focus_switch_cooldown_ms, 5000);
        assert_eq!(args.auto_focus_switch_margin, 0.20);
        assert_eq!(args.auto_focus_required_polls, 2);
        assert_eq!(args.auto_focus_max_roots, 4);
    }

    #[test]
    fn monitor_cli_parses_auto_focus_fields() {
        let cli = Cli::parse_from([
            "stutter",
            "monitor",
            "--auto-focus",
            "--auto-focus-poll-ms",
            "250",
            "--auto-focus-min-confidence",
            "0.75",
            "--auto-focus-switch-cooldown-ms",
            "7500",
            "--auto-focus-switch-margin",
            "0.30",
            "--auto-focus-required-polls",
            "3",
            "--auto-focus-max-roots",
            "2",
        ]);

        let Command::Monitor(args) = cli.command.unwrap() else {
            panic!("expected monitor command");
        };

        assert!(args.auto_focus);
        assert_eq!(args.auto_focus_poll_ms, 250);
        assert_eq!(args.auto_focus_min_confidence, 0.75);
        assert_eq!(args.auto_focus_switch_cooldown_ms, 7500);
        assert_eq!(args.auto_focus_switch_margin, 0.30);
        assert_eq!(args.auto_focus_required_polls, 3);
        assert_eq!(args.auto_focus_max_roots, 2);
    }
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

pub fn command() -> clap::Command {
    Cli::command()
}

fn merge_bool(
    builtin: bool,
    file_value: Option<bool>,
    preset_value: Option<bool>,
    cli_positive: bool,
    cli_negative: bool,
) -> bool {
    if cli_negative {
        false
    } else if cli_positive {
        true
    } else if let Some(value) = preset_value {
        value
    } else if let Some(value) = file_value {
        value
    } else {
        builtin
    }
}

#[allow(dead_code)]
fn monitor_config_from_monitor_args(
    args: MonitorArgs,
    recording_mode: RecordingMode,
) -> anyhow::Result<MonitorConfig> {
    let file_config = crate::config_file::load_user_config()?;
    monitor_config_from_monitor_args_with_file_and_presence(
        args,
        file_config,
        recording_mode,
        MonitorArgPresence::default(),
    )
}

fn monitor_config_from_monitor_args_with_presence(
    args: MonitorArgs,
    recording_mode: RecordingMode,
    cli_presence: MonitorArgPresence,
) -> anyhow::Result<MonitorConfig> {
    let file_config = crate::config_file::load_user_config()?;
    monitor_config_from_monitor_args_with_file_and_presence(
        args,
        file_config,
        recording_mode,
        cli_presence,
    )
}

#[allow(dead_code)]
fn monitor_config_from_monitor_args_with_file(
    args: MonitorArgs,
    file_config: Option<crate::config_file::UserConfigFile>,
    recording_mode: RecordingMode,
) -> anyhow::Result<MonitorConfig> {
    monitor_config_from_monitor_args_with_file_and_presence(
        args,
        file_config,
        recording_mode,
        MonitorArgPresence::default(),
    )
}

fn monitor_config_from_monitor_args_with_file_and_presence(
    mut args: MonitorArgs,
    file_config: Option<crate::config_file::UserConfigFile>,
    recording_mode: RecordingMode,
    cli_presence: MonitorArgPresence,
) -> anyhow::Result<MonitorConfig> {
    let user_file = file_config;
    let file_config = user_file.clone().unwrap_or_default();

    let preset = match args.preset.as_deref() {
        Some(name) => Some(name.parse::<crate::presets::Preset>()?),
        None => None,
    };

    let preset_defaults = preset.map(|preset| preset.defaults()).unwrap_or_default();

    let summary_period_ms = args
        .summary_period_ms
        .or(file_config.summary_ms)
        .unwrap_or(1_000);
    let spike_threshold_us = args
        .spike_threshold_us
        .or(file_config.spike_us)
        .unwrap_or(1_000);
    let max_tasks = args.max_tasks.or(file_config.max_tasks).unwrap_or(1024);

    if !args.include_comm.is_empty() {
        // use CLI
    } else if let Some(config_include) = file_config.include_comm.clone() {
        args.include_comm = config_include;
    }

    if !args.exclude_comm.is_empty() {
        // use CLI
    } else if let Some(config_exclude) = file_config.exclude_comm.clone() {
        args.exclude_comm = config_exclude;
    }

    let _hwmon = merge_bool(
        false,
        file_config.hwmon,
        preset_defaults.hwmon,
        args.hwmon,
        args.no_hwmon,
    );

    let cpu_freq_config = merge_bool(
        false,
        file_config.cpu_freq.or(file_config.no_cpu_freq.map(|n| !n)),
        preset_defaults.cpu_freq,
        args.cpu_freq,
        args.no_cpu_freq,
    );

    let faults = merge_bool(
        false,
        None,
        preset_defaults.faults,
        args.faults,
        args.no_faults,
    );

    let stat_wait = merge_bool(
        false,
        None,
        preset_defaults.stat_wait,
        args.stat_wait,
        args.no_stat_wait,
    );

    let block_io = merge_bool(
        false,
        None,
        preset_defaults.block_io,
        args.block_io,
        args.no_block_io,
    );
    let runtime_slices = merge_bool(
        false,
        None,
        preset_defaults.runtime_slices,
        args.runtime_slices,
        args.no_runtime_slices,
    );

    let irq_latency = merge_bool(
        false,
        None,
        preset_defaults.irq_latency,
        args.irq_latency,
        false,
    );

    if !args.foreground_window
        && let Some(foreground_window) = file_config.foreground_window
    {
        args.foreground_window = foreground_window;
    }

    if !cli_presence.focus_source
        && let Some(focus_source) = file_config.focus_source.as_deref()
    {
        args.focus_source = FocusSource::parse_config_value(focus_source)?;
    }

    if !cli_presence.foreground_source
        && let Some(foreground_source) = file_config.foreground_source.as_deref()
    {
        args.foreground_source = ForegroundSource::parse_config_value(foreground_source)?;
    }

    if !cli_presence.foreground_poll_ms
        && let Some(foreground_poll_ms) = file_config.foreground_poll_ms
    {
        args.foreground_poll_ms = foreground_poll_ms;
    }

    if !cli_presence.foreground_max_stale_ms
        && let Some(foreground_max_stale_ms) = file_config.foreground_max_stale_ms
    {
        args.foreground_max_stale_ms = foreground_max_stale_ms;
    }

    if !args.foreground_include_title
        && let Some(foreground_include_title) = file_config.foreground_include_title
    {
        args.foreground_include_title = foreground_include_title;
    }

    validate_foreground_title_monitor_args(&args)?;
    normalize_foreground_monitor_args(&mut args);
    validate_foreground_monitor_args(&args)?;

    validate_pids("--pid", &args.target_pids)?;
    validate_pids("--tree-pid", &args.tree_pids)?;
    validate_pids("--exclude-tree-pid", &args.exclude_tree_pids)?;

    #[allow(clippy::collapsible_if)]
    if let Some(kb) = args.ringbuf_size_kb {
        if !(64..=16 * 1024).contains(&kb) {
            anyhow::bail!("--ringbuf-size-kb must be between 64 and 16384");
        }
    }

    #[allow(clippy::collapsible_if)]
    if let Some(factor) = args.wakeup_map_factor {
        if factor == 0 || factor > 64 {
            anyhow::bail!("--wakeup-map-factor must be between 1 and 64");
        }
    }

    if args.otlp_endpoint.is_some() && !cfg!(feature = "otel") {
        anyhow::bail!("OpenTelemetry support was not compiled in. Rebuild with --features otel.");
    }

    if args.otel_service_name.trim().is_empty() {
        anyhow::bail!("--otel-service-name must not be empty");
    }

    #[allow(clippy::collapsible_if)]
    if let Some(endpoint) = &args.otlp_endpoint {
        if endpoint.trim().is_empty() {
            anyhow::bail!("--otlp-endpoint must not be empty");
        }
    }

    if summary_period_ms == 0 {
        anyhow::bail!("--summary-ms must be greater than zero");
    }
    if matches!(args.epoch_period_ms, Some(0)) {
        anyhow::bail!("--epoch must be greater than zero");
    }

    if spike_threshold_us == 0 {
        anyhow::bail!("--spike-us must be greater than zero");
    }
    if matches!(args.alert_threshold_ms, Some(0)) {
        anyhow::bail!("--alert-threshold-ms must be greater than zero");
    }
    if args.watch_poll_ms == 0 {
        anyhow::bail!("--watch-poll-ms must be greater than zero");
    }
    if matches!(args.watch_timeout_seconds, Some(0)) {
        anyhow::bail!("--watch-timeout-seconds must be greater than zero");
    }
    if max_tasks == 0 {
        anyhow::bail!("--max-tasks must be greater than zero");
    }
    if args.cpu_perf_max_tasks == 0 {
        anyhow::bail!("--cpu-perf-max-tasks must be greater than zero");
    }
    if args.runtime_slices_max_tasks == 0 {
        anyhow::bail!("--runtime-slices-max-tasks must be greater than zero");
    }

    args.target_pids.sort_unstable();
    args.target_pids.dedup();
    args.tree_pids.sort_unstable();
    args.tree_pids.dedup();
    args.exclude_tree_pids.sort_unstable();
    args.exclude_tree_pids.dedup();
    args.include_comm.sort();
    args.include_comm.dedup();
    args.exclude_comm.sort();
    args.exclude_comm.dedup();
    args.irqs.sort_unstable();
    args.irqs.dedup();

    validate_comm_patterns("--include-comm", &args.include_comm)?;
    validate_comm_patterns("--exclude-comm", &args.exclude_comm)?;

    if matches!(args.watch_process.as_deref(), Some("")) {
        anyhow::bail!("--watch-process must not be empty");
    }
    if args.persistent && args.watch_process.is_none() {
        anyhow::bail!("--persistent requires --watch-process");
    }
    if args.irq_latency && args.irqs.is_empty() {
        anyhow::bail!(
            "--irq-latency requires at least one explicit --irq <N>; inspect /proc/interrupts to find the IRQ number for your GPU or device"
        );
    }
    if matches!(args.hwmon_drm_card.as_deref(), Some("")) {
        anyhow::bail!("--hwmon-drm-card must not be empty");
    }
    if matches!(args.alert_webhook_url.as_deref(), Some("")) {
        anyhow::bail!("--alert-webhook-url must not be empty");
    }

    if args.target_pids.len() > TARGET_PIDS_MAX {
        anyhow::bail!(
            "too many unique target PIDs: got {}, but TARGET_PIDS supports at most {}",
            args.target_pids.len(),
            TARGET_PIDS_MAX
        );
    }

    let spike_threshold_ns = spike_threshold_us
        .checked_mul(1_000)
        .ok_or_else(|| anyhow::anyhow!("--spike-us value is too large"))?;
    let summary_period_ms = args.epoch_period_ms.unwrap_or(summary_period_ms);
    let alert_threshold_ns = args
        .alert_threshold_ms
        .map(|threshold_ms| {
            threshold_ms
                .checked_mul(1_000_000)
                .ok_or_else(|| anyhow::anyhow!("--alert-threshold-ms value is too large"))
        })
        .transpose()?;

    match (&args.csv_path, &args.stream_csv) {
        (None, Some(value)) if value.trim().is_empty() => {
            anyhow::bail!("--stream-csv path must not be empty");
        }
        (Some(_), Some(_)) => {
            anyhow::bail!("--stream-csv conflicts with --csv");
        }
        _ => {}
    }

    let alert_webhook_url = if alert_threshold_ns.is_some() {
        args.alert_webhook_url.clone().or_else(|| {
            std::env::var("STUTTER_ALERT_WEBHOOK_URL")
                .ok()
                .filter(|url| !url.is_empty())
        })
    } else {
        args.alert_webhook_url.clone()
    };

    let mut layer = args.clone().into_monitor_config_layer(cli_presence);
    layer.alert_webhook_url = alert_webhook_url.map(Some);
    if let Some(max_duration) = recording_mode.max_duration() {
        layer.max_duration = Some(Some(max_duration));
    }
    if let Some(epoch) = args.epoch_period_ms {
        layer.summary_period_ms = Some(epoch);
    }

    let is_recording = if args.no_record {
        false
    } else {
        recording_mode.force_recording() || args.run_name.is_some() || args.out_dir.is_some()
    };

    if is_recording {
        let run_name = args.run_name.or_else(|| {
            recording_mode
                .force_recording()
                .then(|| "record".to_owned())
        });
        layer.run_name = run_name.map(Some);
        layer.output_dir = args.out_dir.map(Some);
    }

    let cpu_freq = (cpu_freq_config || is_recording) && !args.no_cpu_freq;
    if cpu_freq {
        layer.cpu_freq = Some(true);
    }

    let resolved = resolve_monitor_config_sources(ConfigSources {
        defaults: DefaultConfig {
            config: MonitorConfig::default(),
        },
        user_file,
        preset: preset.map(|preset| PresetConfig {
            layer: MonitorConfigLayer::from_preset_defaults(preset.defaults()),
        }),
        overrides: CliOverrides { layer }.into(),
    })?;
    let mut config = resolved.config;

    config.timing.summary_period_ms = summary_period_ms;
    config.timing.spike_threshold_ns = spike_threshold_ns;
    config.alerts.threshold_ns = alert_threshold_ns;

    config.probes.faults = faults;
    config.probes.stat_wait = stat_wait;
    config.probes.block_io = block_io;
    config.probes.runtime_slices = runtime_slices;
    config.probes.irq_latency = irq_latency;

    if config.csv_streams_to_stdout() && config.outputs.json_stream {
        anyhow::bail!(
            "--stream-csv - cannot be used with --json-stream because both write to stdout"
        );
    }

    Ok(config)
}

fn validate_pids(flag: &str, pids: &[u32]) -> anyhow::Result<()> {
    if pids.contains(&0) {
        anyhow::bail!("{flag} must be greater than zero");
    }
    Ok(())
}

fn validate_comm_patterns(flag: &str, patterns: &[String]) -> anyhow::Result<()> {
    for pattern in patterns {
        if pattern.is_empty() {
            anyhow::bail!("{flag} patterns must not be empty");
        }
    }
    Ok(())
}

fn normalize_foreground_monitor_args(args: &mut MonitorArgs) {
    if args.focus_source != FocusSource::Heuristic {
        args.foreground_window = true;
    }
}

fn validate_foreground_title_monitor_args(args: &MonitorArgs) -> anyhow::Result<()> {
    let foreground_focus_requested = args.auto_focus
        && matches!(
            args.focus_source,
            FocusSource::Foreground | FocusSource::Hybrid
        );

    if args.foreground_include_title && !args.foreground_window && !foreground_focus_requested {
        anyhow::bail!(
            "--foreground-include-title requires --foreground-window or --auto-focus with --focus-source foreground or hybrid"
        );
    }

    Ok(())
}

fn validate_foreground_monitor_args(args: &MonitorArgs) -> anyhow::Result<()> {
    if args.foreground_poll_ms < 100 {
        anyhow::bail!("--foreground-poll-ms must be >= 100");
    }

    if args.foreground_max_stale_ms < args.foreground_poll_ms {
        eprintln!(
            "warning: foreground max stale is lower than poll interval; provider errors may clear focus quickly"
        );
    }

    Ok(())
}

#[cfg(test)]
fn parse_monitor_config_for_phase15<const N: usize>(
    args: [&str; N],
) -> anyhow::Result<Arc<crate::config::model::MonitorConfig>> {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    match parse_app_command_from(args.iter().map(OsString::from))? {
        AppCommand::Monitor(input) => Ok(input.config.clone()),
        other => anyhow::bail!("expected AppCommand::Monitor, got {other:?}"),
    }
}

#[cfg(test)]
#[test]
fn cli_accepts_auto_focus_foreground_source() {
    let config = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--auto-focus",
        "--focus-source",
        "foreground",
        "--foreground-source",
        "sway",
    ])
    .unwrap();

    assert!(config.focus.auto_focus);
    assert_eq!(config.focus.focus_source, FocusSource::Foreground);
    assert!(config.focus.foreground_window);
    assert_eq!(config.focus.foreground_source, ForegroundSource::Sway);
}

#[cfg(test)]
#[test]
fn foreground_include_title_requires_foreground_window_or_auto_focus_foreground() {
    let err =
        parse_monitor_config_for_phase15(["stutter", "monitor", "--foreground-include-title"])
            .unwrap_err()
            .to_string();

    assert!(err.contains(
        "--foreground-include-title requires --foreground-window or --auto-focus with --focus-source foreground or hybrid"
    ));

    let foreground_window = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--foreground-window",
        "--foreground-include-title",
    ])
    .unwrap();
    assert!(foreground_window.focus.foreground_window);
    assert!(foreground_window.focus.foreground_include_title);

    let foreground_focus = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--auto-focus",
        "--focus-source",
        "foreground",
        "--foreground-include-title",
    ])
    .unwrap();
    assert!(foreground_focus.focus.auto_focus);
    assert_eq!(foreground_focus.focus.focus_source, FocusSource::Foreground);
    assert!(foreground_focus.focus.foreground_window);
    assert!(foreground_focus.focus.foreground_include_title);
}

#[cfg(test)]
mod tests {
    fn parse_app_command_from_inner<I, T>(args: I) -> anyhow::Result<AppCommand>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        super::parse_app_command_from(args)
    }

    fn parse_app_command_from<I, T>(args: I) -> anyhow::Result<AppCommand>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        parse_app_command_from_inner(args)
    }

    fn parse_monitor_config_from_inner<const N: usize>(
        args: [&str; N],
    ) -> anyhow::Result<Arc<crate::config::model::MonitorConfig>> {
        match parse_app_command_from_inner(args.iter().map(OsString::from))? {
            AppCommand::Monitor(input) => Ok(input.config.clone()),
            other => anyhow::bail!("expected AppCommand::Monitor, got {other:?}"),
        }
    }

    fn parse_monitor_config_from<const N: usize>(
        args: [&str; N],
    ) -> anyhow::Result<Arc<crate::config::model::MonitorConfig>> {
        match parse_app_command_from(args.iter().map(OsString::from))? {
            AppCommand::Monitor(input) => Ok(input.config.clone()),
            other => anyhow::bail!("expected AppCommand::Monitor, got {other:?}"),
        }
    }
    use super::*;

    #[test]
    fn rejects_zero_duration_record() {
        let err = parse_app_command_from(["stutter", "record", "--pid", "42", "--duration", "0"])
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("--duration must be greater than zero")
        );
    }

    #[test]
    fn completions_cli_parses_bash() {
        let cli = Cli::try_parse_from(["stutter", "completions", "bash"]).unwrap();

        match cli.command {
            Some(Command::Completions(args)) => {
                assert_eq!(args.shell, clap_complete::Shell::Bash);
            }
            other => panic!("expected completions command, got {other:?}"),
        }
    }

    #[test]
    fn man_cli_parses_output_path() {
        let cli = Cli::try_parse_from(["stutter", "man", "--output", "stutter.1"]).unwrap();

        match cli.command {
            Some(Command::Man(args)) => {
                assert_eq!(args.output, Some(PathBuf::from("stutter.1")));
            }
            other => panic!("expected man command, got {other:?}"),
        }
    }

    #[test]
    fn man_page_renders_troff_header() {
        let cmd = super::command();
        let man = clap_mangen::Man::new(cmd);

        let mut out = Vec::new();
        man.render(&mut out).unwrap();

        let text = String::from_utf8(out).unwrap();
        assert!(text.contains(".TH"));
    }

    #[test]
    fn parses_report_cluster_window_and_top() {
        let command = parse_app_command_from([
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
    }
    #[test]
    fn report_flag_conflicts() {
        // html vs batch
        assert!(
            parse_app_command_from(["stutter", "report", "--html", "r.html", "--batch", "dir"])
                .is_err()
        );
        // json vs json-summary
        assert!(
            parse_app_command_from(["stutter", "report", "--json", "--json-summary", "run"])
                .is_err()
        );
        // analysis-json vs batch
        assert!(
            parse_app_command_from(["stutter", "report", "--analysis-json", "--batch", "dir"])
                .is_err()
        );
        // path vs batch
        assert!(parse_app_command_from(["stutter", "report", "--batch", "dir", "run"]).is_err());
    }

    #[test]
    fn parses_summary_command() {
        let command = parse_app_command_from([
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
    fn validate_requires_path() {
        assert!(parse_app_command_from(["stutter", "validate"]).is_err());
    }

    #[test]
    fn validate_accepts_path() {
        let command = parse_app_command_from(["stutter", "validate", "/tmp/run"]).unwrap();

        let AppCommand::Validate(input) = command else {
            panic!("expected validate command");
        };

        assert_eq!(input.path, PathBuf::from("/tmp/run"));
        assert!(!input.json);
        assert!(!input.strict);
    }

    #[test]
    fn validate_accepts_json() {
        let command =
            parse_app_command_from(["stutter", "validate", "--json", "/tmp/run"]).unwrap();

        let AppCommand::Validate(input) = command else {
            panic!("expected validate command");
        };

        assert_eq!(input.path, PathBuf::from("/tmp/run"));
        assert!(input.json);
        assert!(!input.strict);
    }

    #[test]
    fn validate_accepts_strict() {
        let command =
            parse_app_command_from(["stutter", "validate", "--strict", "/tmp/run"]).unwrap();

        let AppCommand::Validate(input) = command else {
            panic!("expected validate command");
        };

        assert_eq!(input.path, PathBuf::from("/tmp/run"));
        assert!(!input.json);
        assert!(input.strict);
    }

    #[test]
    fn parses_report_batch_json_summary() {
        let command = parse_app_command_from([
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
    fn parses_include_and_exclude_comm_filters() {
        let command = parse_app_command_from([
            "stutter",
            "record",
            "--tree-pid",
            "42",
            "--include-comm",
            "RenderThread",
            "--exclude-comm",
            "steamwebhelper",
        ])
        .unwrap();

        let AppCommand::Monitor(input) = command else {
            panic!("expected monitor command");
        };

        assert_eq!(
            input.config.target.include_comm,
            vec!["RenderThread".to_owned()]
        );
        assert_eq!(
            input.config.target.exclude_comm,
            vec!["steamwebhelper".to_owned()]
        );
    }

    #[test]
    fn parses_exclude_tree_pids() {
        let command = parse_app_command_from([
            "stutter",
            "monitor",
            "--tree-pid",
            "42",
            "--exclude-tree-pid",
            "100",
            "--exclude-tree-pid",
            "100",
            "--exclude-tree-pid",
            "7",
        ])
        .unwrap();

        let AppCommand::Monitor(input) = command else {
            panic!("expected monitor command");
        };

        assert_eq!(input.config.target.tree_pids, vec![42]);
        assert_eq!(input.config.target.exclude_tree_pids, vec![7, 100]);
    }

    #[test]
    fn parses_alert_threshold_and_webhook() {
        let command = parse_app_command_from([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--alert-threshold-ms",
            "250",
            "--alert-webhook-url",
            "https://example.invalid/stutter",
        ])
        .unwrap();

        let AppCommand::Monitor(input) = command else {
            panic!("expected monitor command");
        };

        assert_eq!(input.config.alerts.threshold_ns, Some(250_000_000));
        assert_eq!(
            input.config.alerts.webhook_url.as_deref(),
            Some("https://example.invalid/stutter")
        );
    }

    #[test]
    fn parses_epoch_as_summary_period_override() {
        let command = parse_app_command_from([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--summary-ms",
            "100",
            "--epoch",
            "5000",
        ])
        .unwrap();

        let AppCommand::Monitor(input) = command else {
            panic!("expected monitor command");
        };

        assert_eq!(input.config.timing.epoch_period_ms, Some(5_000));
        assert_eq!(input.config.timing.summary_period_ms, 5_000);
    }

    #[test]
    fn parses_watch_process_without_explicit_pid() {
        let command = parse_app_command_from([
            "stutter",
            "monitor",
            "--watch-process",
            "KingdomCome",
            "--csv",
            "/tmp/stutter.csv",
        ])
        .unwrap();

        let AppCommand::Monitor(input) = command else {
            panic!("expected monitor command");
        };

        assert_eq!(
            input.config.target.watch_process.as_deref(),
            Some("KingdomCome")
        );
        assert_eq!(
            input.config.streams.csv,
            Some(CsvStreamTarget::File(PathBuf::from("/tmp/stutter.csv")))
        );
    }

    #[test]
    fn follow_exec_defaults_on_and_can_be_disabled() {
        let default_command =
            parse_app_command_from(["stutter", "monitor", "--pid", "42"]).unwrap();
        let AppCommand::Monitor(default_input) = default_command else {
            panic!("expected monitor command");
        };
        assert!(default_input.config.safety.follow_exec);

        let disabled_command =
            parse_app_command_from(["stutter", "monitor", "--pid", "42", "--no-follow-exec"])
                .unwrap();
        let AppCommand::Monitor(disabled_input) = disabled_command else {
            panic!("expected monitor command");
        };
        assert!(!disabled_input.config.safety.follow_exec);
    }

    #[test]
    fn native_cgroup_filter_requires_cgroupv2() {
        let result = parse_app_command_from([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--native-cgroup-filter",
        ]);

        assert!(result.is_err());
    }

    #[test]
    fn native_cgroup_filter_sets_config() {
        let command = parse_app_command_from([
            "stutter",
            "monitor",
            "--cgroupv2",
            "/sys/fs/cgroup/test.slice",
            "--native-cgroup-filter",
        ])
        .unwrap();

        let AppCommand::Monitor(input) = command else {
            panic!("expected monitor command");
        };

        assert_eq!(
            input.config.target.cgroupv2.as_deref(),
            Some(std::path::Path::new("/sys/fs/cgroup/test.slice"))
        );
        assert!(input.config.safety.native_cgroup_filter);
    }

    #[test]
    fn native_cgroup_filter_defaults_false() {
        let command = parse_app_command_from([
            "stutter",
            "monitor",
            "--cgroupv2",
            "/sys/fs/cgroup/test.slice",
        ])
        .unwrap();

        let AppCommand::Monitor(input) = command else {
            panic!("expected monitor command");
        };

        assert!(!input.config.safety.native_cgroup_filter);
    }

    #[test]
    fn rejects_zero_report_cluster_window() {
        let err = parse_app_command_from(["stutter", "report", "--cluster-ms", "0", "/tmp/run"])
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("--cluster-ms must be greater than zero")
        );
    }

    #[test]
    fn rejects_zero_max_tasks() {
        let err = parse_app_command_from(["stutter", "monitor", "--pid", "42", "--max-tasks", "0"])
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("--max-tasks must be greater than zero")
        );
    }

    #[test]
    fn parses_cpu_perf_monitor_flags() {
        let command = parse_app_command_from([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--cpu-perf",
            "--cpu-perf-kernel",
            "--cpu-perf-max-tasks",
            "16",
        ])
        .unwrap();

        let AppCommand::Monitor(input) = command else {
            panic!("expected monitor command");
        };

        assert!(input.config.probes.cpu_perf);
        assert!(input.config.cpu_perf.include_kernel);
        assert_eq!(input.config.cpu_perf.max_tasks, 16);
        assert!(!input.config.cpu_perf.collect_cache_refs);
    }

    #[test]
    fn parses_cpu_perf_cache_refs_for_recording() {
        let command = parse_app_command_from([
            "stutter",
            "record",
            "--pid",
            "42",
            "--cpu-perf",
            "--cpu-perf-cache-refs",
        ])
        .unwrap();

        let AppCommand::Monitor(input) = command else {
            panic!("expected monitor command");
        };

        assert!(input.config.probes.cpu_perf);
        assert!(input.config.cpu_perf.collect_cache_refs);
        assert!(
            input.config.recording.output_dir.is_some()
                || input.config.recording.run_name.is_some()
        );
    }

    #[test]
    fn rejects_zero_cpu_perf_max_tasks() {
        let err = parse_app_command_from([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--cpu-perf-max-tasks",
            "0",
        ])
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("--cpu-perf-max-tasks must be greater than zero")
        );
    }

    #[test]
    fn parses_extended_check_command() {
        let command = parse_app_command_from([
            "stutter",
            "check",
            "--baseline",
            "/tmp/base",
            "--current",
            "/tmp/current",
            "--max-regression-p99-ms",
            "0.5",
            "--max-max-regression-ms",
            "2.0",
            "--json",
            "--top",
            "5",
            "--filter-class",
            "Game",
        ])
        .unwrap();

        let AppCommand::Check(input) = command else {
            panic!("expected check command");
        };

        assert_eq!(input.baseline, PathBuf::from("/tmp/base"));
        assert_eq!(input.current, PathBuf::from("/tmp/current"));
        assert_eq!(input.max_regression_p99_ms, Some(0.5));
        assert_eq!(input.max_max_regression_ms, Some(2.0));
        assert!(input.json);
        assert_eq!(input.top, 5);
        assert_eq!(input.filter_class, Some(TaskClass::Game));
    }

    #[test]
    fn rejects_check_without_thresholds() {
        let err = parse_app_command_from([
            "stutter",
            "check",
            "--baseline",
            "/tmp/base",
            "--current",
            "/tmp/current",
        ])
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("check requires at least one threshold")
        );
    }

    #[test]
    fn parses_restore_and_apply_profile_commands() {
        let restore = parse_app_command_from(["stutter", "restore"]).unwrap();
        assert!(matches!(restore, AppCommand::Restore(input) if !input.dry_run));

        let apply = parse_app_command_from([
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
    fn parses_apply_profile_force_watch_and_refresh() {
        let command = parse_app_command_from([
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
        ])
        .unwrap();

        let AppCommand::ApplyProfile(input) = command else {
            panic!("expected apply profile command");
        };

        assert!(input.force);
        assert!(input.allow_medium_risk);
        assert!(input.watch);
        assert!(input.keep_applied);
        assert_eq!(input.refresh_ms, 250);
    }

    #[test]
    fn parses_keep_missing_pid_and_watch_controls() {
        let command = parse_app_command_from([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--keep-missing-pid",
            "--watch-poll-ms",
            "500",
            "--watch-timeout-seconds",
            "3",
        ])
        .unwrap();

        let AppCommand::Monitor(input) = command else {
            panic!("expected monitor command");
        };

        assert!(input.config.target.keep_missing_pid);
        assert_eq!(input.config.watch.poll_ms, 500);
        assert_eq!(input.config.watch.timeout, Some(Duration::from_secs(3)));
    }

    #[test]
    fn parses_restore_dry_run() {
        let command = parse_app_command_from(["stutter", "restore", "--dry-run"]).unwrap();
        assert!(matches!(command, AppCommand::Restore(input) if input.dry_run));
    }

    #[test]
    fn parses_correlation_flags_and_tui() {
        let command = parse_app_command_from([
            "stutter",
            "record",
            "--pid",
            "42",
            "--irq-latency",
            "--irq",
            "137",
            "--hwmon",
            "--hwmon-drm-card",
            "card1",
            "--hwmon-render-node",
            "/dev/dri/renderD129",
            "--mangohud-log",
            "/tmp/mango.csv",
            "--tui",
        ])
        .unwrap();

        let AppCommand::Monitor(input) = command else {
            panic!("expected monitor command");
        };

        assert!(input.config.probes.irq_latency);
        assert_eq!(input.config.probes.irqs, vec![137]);
        assert!(input.config.probes.hwmon);
        assert_eq!(input.config.hwmon.drm_card.as_deref(), Some("card1"));
        assert_eq!(
            input.config.hwmon.render_node,
            Some(PathBuf::from("/dev/dri/renderD129"))
        );
        assert_eq!(
            input.config.mangohud.log,
            Some(PathBuf::from("/tmp/mango.csv"))
        );
        assert!(input.config.ui.tui);
    }

    #[test]
    fn parses_tune_command() {
        let command = parse_app_command_from([
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
    fn parses_bench_baseline() {
        let command = parse_app_command_from([
            "stutter",
            "bench",
            "--watch-process",
            "Game.exe",
            "--persistent",
            "--duration",
            "180",
            "--scenario",
            "route-a",
            "--role",
            "baseline",
        ])
        .unwrap();

        let AppCommand::Bench(input) = command else {
            panic!("expected bench command");
        };

        assert_eq!(input.role, "baseline");
        assert_eq!(input.run_name, "bench-baseline-route-a");
        assert_eq!(
            input.config.timing.max_duration,
            Some(Duration::from_secs(180))
        );
        assert_eq!(
            input.config.recording.run_name.as_deref(),
            Some("bench-baseline-route-a")
        );
        assert_eq!(
            input.config.target.watch_process.as_deref(),
            Some("Game.exe")
        );
        assert!(input.config.target.persistent);
    }

    #[test]
    fn rejects_zero_bench_duration() {
        let err = parse_app_command_from([
            "stutter",
            "bench",
            "--duration",
            "0",
            "--scenario",
            "route-a",
        ])
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("--duration must be greater than zero")
        );
    }

    #[test]
    fn rejects_invalid_bench_role() {
        let err = parse_app_command_from([
            "stutter",
            "bench",
            "--duration",
            "1",
            "--scenario",
            "route-a",
            "--role",
            "candidate",
        ])
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("--role must be baseline or current")
        );
    }

    #[test]
    fn bench_preserves_monitor_flags() {
        let command = parse_app_command_from([
            "stutter",
            "bench",
            "--watch-process",
            "Game.exe",
            "--hwmon",
            "--mangohud-log",
            "/tmp/mango.csv",
            "--duration",
            "10",
            "--scenario",
            "route-a",
        ])
        .unwrap();

        let AppCommand::Bench(input) = command else {
            panic!("expected bench command");
        };

        assert_eq!(
            input.config.target.watch_process.as_deref(),
            Some("Game.exe")
        );
        assert!(input.config.probes.hwmon);
        assert_eq!(
            input.config.mangohud.log,
            Some(PathBuf::from("/tmp/mango.csv"))
        );
    }

    #[test]
    fn parses_doctor_command() {
        let command = parse_app_command_from(["stutter", "doctor"]).unwrap();

        let AppCommand::Doctor(input) = command else {
            panic!("expected doctor command");
        };

        assert!(!input.input.json);
        assert!(!input.input.hwmon);
        assert!(!input.input.irq_latency);
    }

    #[test]
    fn probes_command_parses() {
        let command = parse_app_command_from(["stutter", "probes"]).unwrap();

        let AppCommand::Probes(input) = command else {
            panic!("expected probes command");
        };

        assert!(!input.json);
    }

    #[test]
    fn probes_json_parses() {
        let command = parse_app_command_from(["stutter", "probes", "--json"]).unwrap();

        let AppCommand::Probes(input) = command else {
            panic!("expected probes command");
        };

        assert!(input.json);
    }

    #[test]
    fn parses_audit_command() {
        let command = parse_app_command_from([
            "stutter",
            "audit",
            "--tail",
            "50",
            "--json",
            "--path",
            "/tmp/actions.jsonl",
        ])
        .unwrap();

        let AppCommand::Audit(input) = command else {
            panic!("expected audit command");
        };

        assert_eq!(input.path, Some(PathBuf::from("/tmp/actions.jsonl")));
        assert_eq!(input.tail, 50);
        assert!(input.json);
    }

    #[test]
    fn parses_advisor_run_command() {
        let command = parse_app_command_from([
            "stutter",
            "advisor",
            "--run",
            "/tmp/run",
            "--profiles",
            "profiles.toml",
            "--json",
        ])
        .unwrap();

        let AppCommand::Advisor(input) = command else {
            panic!("expected advisor command");
        };

        assert_eq!(input.run, Some(PathBuf::from("/tmp/run")));
        assert_eq!(input.profiles, Some(PathBuf::from("profiles.toml")));
        assert!(input.json);
        assert!(!input.watch_runs);
    }

    #[test]
    fn parses_advisor_watch_once_command() {
        let command = parse_app_command_from([
            "stutter",
            "advisor",
            "--watch-runs",
            "--runs-dir",
            "/tmp/runs",
            "--poll-seconds",
            "1",
            "--once",
        ])
        .unwrap();

        let AppCommand::Advisor(input) = command else {
            panic!("expected advisor command");
        };

        assert_eq!(input.run, None);
        assert!(input.watch_runs);
        assert_eq!(input.runs_dir, Some(PathBuf::from("/tmp/runs")));
        assert_eq!(input.poll_seconds, 1);
        assert!(input.once);
    }

    #[test]
    fn rejects_advisor_watch_runs_with_run() {
        let err =
            parse_app_command_from(["stutter", "advisor", "--watch-runs", "--run", "/tmp/run"])
                .unwrap_err();

        assert!(
            err.to_string()
                .contains("--watch-runs conflicts with --run")
        );
    }

    #[test]
    fn parses_doctor_json_hwmon_root() {
        let command = parse_app_command_from([
            "stutter",
            "doctor",
            "--json",
            "--hwmon",
            "--hwmon-root",
            "/tmp/fake",
        ])
        .unwrap();

        let AppCommand::Doctor(input) = command else {
            panic!("expected doctor command");
        };

        assert!(input.input.json);
        assert!(input.input.hwmon);
        assert_eq!(input.input.hwmon_root, Some(PathBuf::from("/tmp/fake")));
    }

    #[test]
    fn parses_doctor_irq_latency_without_irq() {
        let command = parse_app_command_from(["stutter", "doctor", "--irq-latency"]).unwrap();

        let AppCommand::Doctor(input) = command else {
            panic!("expected doctor command");
        };

        assert!(input.input.irq_latency);
        assert!(input.input.irqs.is_empty());
    }

    #[test]
    fn rejects_irq_latency_without_irq() {
        let err = parse_app_command_from(["stutter", "monitor", "--pid", "42", "--irq-latency"])
            .unwrap_err();

        assert!(err.to_string().contains("--irq-latency requires"));
        assert!(err.to_string().contains("/proc/interrupts"));
    }

    #[test]
    fn rejects_persistent_without_watch_process() {
        let err = parse_app_command_from(["stutter", "monitor", "--pid", "42", "--persistent"])
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("--persistent requires --watch-process")
        );
    }

    #[test]
    fn rejects_hwmon_args_without_hwmon_flag() {
        for arg in ["--hwmon-root", "--hwmon-drm-card", "--hwmon-render-node"] {
            let val = if arg == "--hwmon-drm-card" {
                "card0"
            } else {
                "/dev/null"
            };
            let err = parse_app_command_from(["stutter", "monitor", "--pid", "42", arg, val])
                .unwrap_err();

            // Clap error message for missing requirements
            assert!(
                err.to_string()
                    .contains("required arguments were not provided"),
                "expected clap requirement error for {arg}, got: {err:?}"
            );
        }
    }

    #[test]
    fn rejects_zero_exclude_tree_pid() {
        let err = parse_app_command_from([
            "stutter",
            "monitor",
            "--tree-pid",
            "42",
            "--exclude-tree-pid",
            "0",
        ])
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("--exclude-tree-pid must be greater than zero")
        );
    }

    #[test]
    fn rejects_zero_alert_threshold() {
        let err = parse_app_command_from([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--alert-threshold-ms",
            "0",
        ])
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("--alert-threshold-ms must be greater than zero")
        );
    }

    #[test]
    fn rejects_zero_epoch() {
        let err = parse_app_command_from(["stutter", "monitor", "--pid", "42", "--epoch", "0"])
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("--epoch must be greater than zero")
        );
    }
    #[test]
    fn parses_check_command() {
        let command = parse_app_command_from([
            "stutter",
            "check",
            "--baseline",
            "run1/",
            "--current",
            "run2/",
            "--max-regression-p99-ms",
            "2.5",
        ])
        .unwrap();

        let AppCommand::Check(input) = command else {
            panic!("expected check command");
        };

        assert_eq!(input.baseline, PathBuf::from("run1/"));
        assert_eq!(input.current, PathBuf::from("run2/"));
        assert_eq!(input.max_regression_p99_ms, Some(2.5));
        assert_eq!(input.max_max_regression_ms, None);
        assert!(!input.json);
        assert_eq!(input.top, 10);
        assert_eq!(input.filter_class, None);
    }

    #[test]
    fn rejects_invalid_regression_threshold() {
        for val in ["NaN", "inf", "-1.0"] {
            let err = parse_app_command_from([
                "stutter",
                "check",
                "--baseline",
                "run1/",
                "--current",
                "run2/",
                &format!("--max-regression-p99-ms={val}"),
            ])
            .unwrap_err();

            assert!(
                err.to_string()
                    .contains("--max-regression-p99-ms must be a finite non-negative value"),
                "expected failure for {val}, got {err}"
            );
        }
    }

    #[test]
    fn parses_correlation_flags() {
        let command = parse_app_command_from([
            "stutter",
            "monitor",
            "--pid",
            "42",
            "--faults",
            "--block-io",
            "--stat-wait",
        ])
        .unwrap();

        let AppCommand::Monitor(input) = command else {
            panic!("expected monitor command");
        };

        assert!(input.config.probes.faults);
        assert!(input.config.probes.block_io);
        assert!(input.config.probes.stat_wait);
    }

    #[test]
    fn record_enables_cpu_freq_by_default_but_can_be_disabled() {
        let command = parse_app_command_from(["stutter", "record", "--pid", "42"]).unwrap();
        let AppCommand::Monitor(input) = command else {
            panic!("expected monitor command");
        };
        assert!(input.config.probes.cpu_freq);

        let command =
            parse_app_command_from(["stutter", "record", "--pid", "42", "--no-cpu-freq"]).unwrap();
        let AppCommand::Monitor(input) = command else {
            panic!("expected monitor command");
        };
        assert!(!input.config.probes.cpu_freq);
    }

    #[test]
    fn monitor_disables_cpu_freq_by_default_but_can_be_enabled() {
        let command = parse_app_command_from(["stutter", "monitor", "--pid", "42"]).unwrap();
        let AppCommand::Monitor(input) = command else {
            panic!("expected monitor command");
        };
        assert!(!input.config.probes.cpu_freq);

        let command =
            parse_app_command_from(["stutter", "monitor", "--pid", "42", "--cpu-freq"]).unwrap();
        let AppCommand::Monitor(input) = command else {
            panic!("expected monitor command");
        };
        assert!(input.config.probes.cpu_freq);
    }

    #[test]
    fn monitor_json_stream_flag_sets_config() {
        let command =
            parse_app_command_from(["stutter", "monitor", "--pid", "42", "--json-stream"]).unwrap();
        let AppCommand::Monitor(input) = command else {
            panic!("expected monitor command");
        };
        assert!(input.config.outputs.json_stream);
        assert!(input.config.streams.json_stream);

        let command = parse_app_command_from(["stutter", "monitor", "--pid", "42"]).unwrap();
        let AppCommand::Monitor(input) = command else {
            panic!("expected monitor command");
        };
        assert!(!input.config.outputs.json_stream);
        assert!(!input.config.streams.json_stream);
    }

    #[test]
    fn config_file_sets_summary_when_cli_omitted() {
        let args = MonitorArgs {
            watch_poll_ms: 2000,
            cpu_perf_max_tasks: 128,
            ..MonitorArgs::default()
        };
        let file_config = crate::config_file::UserConfigFile {
            summary_ms: Some(500),
            ..Default::default()
        };
        let config = monitor_config_from_monitor_args_with_file(
            args,
            Some(file_config),
            RecordingMode::Monitor,
        )
        .unwrap();
        assert_eq!(config.timing.summary_period_ms, 500);
    }

    #[test]
    fn cli_overrides_config() {
        let mut args = MonitorArgs {
            watch_poll_ms: 2000,
            cpu_perf_max_tasks: 128,
            ..MonitorArgs::default()
        };
        args.summary_period_ms = Some(200);
        let file_config = crate::config_file::UserConfigFile {
            summary_ms: Some(500),
            ..Default::default()
        };
        let config = monitor_config_from_monitor_args_with_file(
            args,
            Some(file_config),
            RecordingMode::Monitor,
        )
        .unwrap();
        assert_eq!(config.timing.summary_period_ms, 200);
    }

    #[test]
    fn include_comm_from_config_used_when_cli_omitted() {
        let args = MonitorArgs {
            watch_poll_ms: 2000,
            cpu_perf_max_tasks: 128,
            ..MonitorArgs::default()
        };
        let file_config = crate::config_file::UserConfigFile {
            include_comm: Some(vec!["Game".to_owned(), "Render".to_owned()]),
            ..Default::default()
        };
        let config = monitor_config_from_monitor_args_with_file(
            args,
            Some(file_config),
            RecordingMode::Monitor,
        )
        .unwrap();
        assert_eq!(config.target.include_comm.len(), 2);
        // They get sorted in monitor_config_from_monitor_args
        assert_eq!(config.target.include_comm[0], "Game");
        assert_eq!(config.target.include_comm[1], "Render");
    }

    #[test]
    fn cli_include_comm_overrides_config_list() {
        let mut args = MonitorArgs {
            watch_poll_ms: 2000,
            cpu_perf_max_tasks: 128,
            ..MonitorArgs::default()
        };
        args.include_comm = vec!["RenderThread".to_owned()];
        let file_config = crate::config_file::UserConfigFile {
            include_comm: Some(vec!["Game".to_owned()]),
            ..Default::default()
        };
        let config = monitor_config_from_monitor_args_with_file(
            args,
            Some(file_config),
            RecordingMode::Monitor,
        )
        .unwrap();
        assert_eq!(config.target.include_comm.len(), 1);
        assert_eq!(config.target.include_comm[0], "RenderThread");
    }

    #[test]
    fn diagnosis_preset_enables_expected_fields() {
        let mut args = MonitorArgs {
            watch_poll_ms: 2000,
            cpu_perf_max_tasks: 128,
            preset: Some("diagnosis".to_owned()),
            ..MonitorArgs::default()
        };
        args.target_pids = vec![1234];
        let config =
            monitor_config_from_monitor_args_with_file(args, None, RecordingMode::Monitor).unwrap();

        assert!(config.probes.hwmon);
        assert!(config.probes.cpu_freq);
        assert!(config.probes.faults);
        assert!(config.probes.stat_wait);
        assert!(config.probes.block_io);
        assert!(config.probes.runtime_slices);
        assert!(!config.probes.irq_latency);
    }

    #[test]
    fn explicit_no_cpu_freq_wins() {
        let mut args = MonitorArgs {
            watch_poll_ms: 2000,
            cpu_perf_max_tasks: 128,
            preset: Some("diagnosis".to_owned()),
            no_cpu_freq: true,
            ..MonitorArgs::default()
        };
        args.target_pids = vec![1234];
        let config =
            monitor_config_from_monitor_args_with_file(args, None, RecordingMode::Monitor).unwrap();
        assert!(!config.probes.cpu_freq);
    }

    #[test]
    fn lightweight_disables_optional_collectors() {
        let mut args = MonitorArgs {
            watch_poll_ms: 2000,
            cpu_perf_max_tasks: 128,
            preset: Some("lightweight".to_owned()),
            ..MonitorArgs::default()
        };
        args.target_pids = vec![1234];
        let config =
            monitor_config_from_monitor_args_with_file(args, None, RecordingMode::Monitor).unwrap();

        assert!(!config.probes.hwmon);
        assert!(!config.probes.cpu_freq);
        assert!(!config.probes.faults);
        assert!(!config.probes.stat_wait);
        assert!(!config.probes.block_io);
        assert!(!config.probes.runtime_slices);
    }

    #[test]
    fn explicit_positive_flag_overrides_lightweight() {
        let mut args = MonitorArgs {
            watch_poll_ms: 2000,
            cpu_perf_max_tasks: 128,
            preset: Some("lightweight".to_owned()),
            faults: true,
            ..MonitorArgs::default()
        };
        args.target_pids = vec![1234];
        let config =
            monitor_config_from_monitor_args_with_file(args, None, RecordingMode::Monitor).unwrap();
        assert!(config.probes.faults);
    }

    #[test]
    fn stream_csv_path_sets_file_target() {
        let args = MonitorArgs {
            target_pids: vec![1234],
            stream_csv: Some("out.csv".to_owned()),
            ..Default::default()
        };
        let config = monitor_config_from_monitor_args(args, RecordingMode::Monitor).unwrap();

        assert!(matches!(
            config.streams.csv,
            Some(CsvStreamTarget::File(ref path)) if path == std::path::Path::new("out.csv")
        ));
    }

    #[test]
    fn stream_csv_dash_sets_stdout_target() {
        let args = MonitorArgs {
            target_pids: vec![1234],
            stream_csv: Some("-".to_owned()),
            ..Default::default()
        };
        let config = monitor_config_from_monitor_args(args, RecordingMode::Monitor).unwrap();

        assert!(matches!(config.streams.csv, Some(CsvStreamTarget::Stdout)));
    }

    #[test]
    fn stream_csv_stdout_conflicts_with_json_stream() {
        let args = MonitorArgs {
            target_pids: vec![1234],
            stream_csv: Some("-".to_owned()),
            json_stream: true,
            ..Default::default()
        };
        let err = monitor_config_from_monitor_args(args, RecordingMode::Monitor).unwrap_err();
        assert!(err.to_string().contains("stdout"));
    }

    #[test]
    fn agent_accepts_allow_unsafe_bind() {
        let command = parse_app_command_from(["stutter", "agent", "--allow-unsafe-bind"]).unwrap();
        let AppCommand::Agent(input) = command else {
            panic!("expected agent command");
        };
        assert!(input.allow_unsafe_bind);
    }

    #[test]
    fn agent_accepts_bearer_token_file() {
        let command =
            parse_app_command_from(["stutter", "agent", "--bearer-token-file", "/tmp/token"])
                .unwrap();
        let AppCommand::Agent(input) = command else {
            panic!("expected agent command");
        };
        assert_eq!(input.bearer_token_file, Some(PathBuf::from("/tmp/token")));
    }

    #[test]
    fn agent_accepts_bearer_token_env() {
        let command =
            parse_app_command_from(["stutter", "agent", "--bearer-token-env", "MY_TOKEN"]).unwrap();
        let AppCommand::Agent(input) = command else {
            panic!("expected agent command");
        };
        assert_eq!(input.bearer_token_env, "MY_TOKEN");
    }

    #[test]
    fn agent_rejects_zero_limits() {
        assert!(
            parse_app_command_from(["stutter", "agent", "--max-duration-seconds", "0"]).is_err()
        );
        assert!(parse_app_command_from(["stutter", "agent", "--max-targets", "0"]).is_err());
        assert!(
            parse_app_command_from(["stutter", "agent", "--max-concurrent-recordings", "0"])
                .is_err()
        );
    }

    #[test]
    fn agent_rejects_max_concurrent_recordings_above_one() {
        let err = parse_app_command_from(["stutter", "agent", "--max-concurrent-recordings", "2"])
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("agent currently supports at most 1 concurrent recording")
        );
    }

    #[test]
    fn scenario_create_parses() {
        let command = parse_app_command_from([
            "stutter",
            "scenario",
            "create",
            "kcd-route",
            "--duration",
            "60",
            "--watch-process",
            "KingdomCome.exe",
        ])
        .unwrap();

        let AppCommand::ScenarioCreate(input) = command else {
            panic!("expected ScenarioCreate command");
        };

        assert_eq!(input.name, "kcd-route");
        assert_eq!(input.duration, 60);
        assert_eq!(input.watch_process, Some("KingdomCome.exe".to_owned()));
    }

    #[test]
    fn scenario_create_rejects_zero_duration() {
        assert!(
            parse_app_command_from(["stutter", "scenario", "create", "test", "--duration", "0"])
                .is_err()
        );
    }

    #[test]
    fn scenario_run_parses_baseline() {
        let command = parse_app_command_from([
            "stutter",
            "scenario",
            "run",
            "kcd-route",
            "--role",
            "baseline",
        ])
        .unwrap();
        let AppCommand::ScenarioRun(input) = command else {
            panic!("expected ScenarioRun command");
        };
        assert_eq!(input.name, "kcd-route");
        assert_eq!(input.role, "baseline");
    }

    #[test]
    fn scenario_run_parses_current() {
        let command = parse_app_command_from([
            "stutter",
            "scenario",
            "run",
            "kcd-route",
            "--role",
            "current",
        ])
        .unwrap();
        let AppCommand::ScenarioRun(input) = command else {
            panic!("expected ScenarioRun command");
        };
        assert_eq!(input.role, "current");
    }

    #[test]
    fn scenario_run_rejects_bad_role() {
        assert!(
            parse_app_command_from(["stutter", "scenario", "run", "test", "--role", "other"])
                .is_err()
        );
    }

    #[test]
    fn scenario_compare_parses() {
        let command =
            parse_app_command_from(["stutter", "scenario", "compare", "kcd-route", "--top", "5"])
                .unwrap();
        let AppCommand::ScenarioCompare(input) = command else {
            panic!("expected ScenarioCompare command");
        };
        assert_eq!(input.name, "kcd-route");
        assert_eq!(input.top, 5);
    }

    #[test]
    fn scenario_compare_rejects_top_zero() {
        assert!(
            parse_app_command_from(["stutter", "scenario", "compare", "test", "--top", "0"])
                .is_err()
        );
    }

    #[test]
    fn scenario_path_parses() {
        let command = parse_app_command_from(["stutter", "scenario", "path", "kcd-route"]).unwrap();
        let AppCommand::ScenarioPath(input) = command else {
            panic!("expected ScenarioPath command");
        };
        assert_eq!(input.name, "kcd-route");
    }

    #[test]
    fn monitor_defaults_keep_heuristic_focus_and_no_foreground_window() {
        let config = parse_monitor_config_from(["stutter", "monitor"]).unwrap();

        assert!(!config.focus.auto_focus);
        assert_eq!(config.focus.focus_source, FocusSource::Heuristic);
        assert!(!config.focus.foreground_window);
        assert_eq!(config.focus.foreground_source, ForegroundSource::Auto);
        assert_eq!(config.focus.foreground_poll_ms, 1000);
        assert_eq!(config.focus.foreground_max_stale_ms, 2500);
        assert!(!config.focus.foreground_include_title);
    }

    #[test]
    fn foreground_window_records_context_without_changing_explicit_targets() {
        let config = parse_monitor_config_from([
            "stutter",
            "monitor",
            "--pid",
            "1234",
            "--tree-pid",
            "5678",
            "--foreground-window",
            "--foreground-source",
            "sway",
            "--foreground-poll-ms",
            "750",
            "--foreground-max-stale-ms",
            "3000",
        ])
        .unwrap();

        assert_eq!(config.target.target_pids, vec![1234]);
        assert_eq!(config.target.tree_pids, vec![5678]);
        assert!(!config.focus.auto_focus);
        assert_eq!(config.focus.focus_source, FocusSource::Heuristic);
        assert!(config.focus.foreground_window);
        assert_eq!(config.focus.foreground_source, ForegroundSource::Sway);
        assert_eq!(config.focus.foreground_poll_ms, 750);
        assert_eq!(config.focus.foreground_max_stale_ms, 3000);
    }

    #[test]
    fn cli_accepts_auto_focus_foreground_source() {
        let config = parse_monitor_config_from([
            "stutter",
            "monitor",
            "--auto-focus",
            "--focus-source",
            "foreground",
            "--foreground-source",
            "sway",
        ])
        .unwrap();

        assert!(config.focus.auto_focus);
        assert_eq!(config.focus.focus_source, FocusSource::Foreground);
        assert!(config.focus.foreground_window);
        assert_eq!(config.focus.foreground_source, ForegroundSource::Sway);
    }

    #[test]
    fn foreground_include_title_requires_foreground_window_or_auto_focus_foreground() {
        let err = parse_monitor_config_from(["stutter", "monitor", "--foreground-include-title"])
            .unwrap_err()
            .to_string();

        assert!(err.contains(
            "--foreground-include-title requires --foreground-window or --auto-focus with --focus-source foreground or hybrid"
        ));

        let foreground_window = parse_monitor_config_from([
            "stutter",
            "monitor",
            "--foreground-window",
            "--foreground-include-title",
        ])
        .unwrap();
        assert!(foreground_window.focus.foreground_window);
        assert!(foreground_window.focus.foreground_include_title);

        let foreground_focus = parse_monitor_config_from([
            "stutter",
            "monitor",
            "--auto-focus",
            "--focus-source",
            "foreground",
            "--foreground-include-title",
        ])
        .unwrap();
        assert!(foreground_focus.focus.auto_focus);
        assert_eq!(foreground_focus.focus.focus_source, FocusSource::Foreground);
        assert!(foreground_focus.focus.foreground_window);
        assert!(foreground_focus.focus.foreground_include_title);
    }

    #[test]
    fn auto_focus_foreground_enables_foreground_config_without_foreground_window_flag() {
        let config = parse_monitor_config_from([
            "stutter",
            "monitor",
            "--auto-focus",
            "--focus-source",
            "foreground",
            "--foreground-source",
            "x11",
            "--foreground-include-title",
        ])
        .unwrap();

        assert!(config.focus.auto_focus);
        assert_eq!(config.focus.focus_source, FocusSource::Foreground);
        assert!(
            config.focus.foreground_window,
            "non-heuristic focus_source must normalize foreground_window to true"
        );
        assert_eq!(config.focus.foreground_source, ForegroundSource::X11);
        assert!(config.focus.foreground_include_title);
    }

    #[test]
    fn auto_focus_hybrid_enables_foreground_with_heuristic_fallback_mode() {
        let config = parse_monitor_config_from([
            "stutter",
            "monitor",
            "--auto-focus",
            "--focus-source",
            "hybrid",
        ])
        .unwrap();

        assert!(config.focus.auto_focus);
        assert_eq!(config.focus.focus_source, FocusSource::Hybrid);
        assert!(
            config.focus.foreground_window,
            "hybrid focus_source must normalize foreground_window to true"
        );
    }

    struct EnvGuard {
        key: &'static str,
        old: Option<String>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let old = std::env::var(key).ok();
            unsafe {
                std::env::set_var(key, value);
            }
            Self { key, old }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(old) = &self.old {
                unsafe {
                    std::env::set_var(self.key, old);
                }
            } else {
                unsafe {
                    std::env::remove_var(self.key);
                }
            }
        }
    }

    fn temp_dir(name: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("stutter-test-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn config_file_foreground_fields_merge_into_monitor_config() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let dir = temp_dir("config-file-foreground-fields");
        let config_path = dir.join("config.toml");
        std::fs::write(
            &config_path,
            r#"
foreground_window = true
focus_source = "foreground"
foreground_source = "sway"
foreground_poll_ms = 750
foreground_max_stale_ms = 3000
foreground_include_title = true
"#,
        )
        .unwrap();

        let _guard = EnvGuard::set("STUTTER_CONFIG", config_path.to_str().unwrap());

        let config = parse_monitor_config_from_inner(["stutter", "monitor"]).unwrap();

        assert_eq!(config.focus.focus_source, FocusSource::Foreground);
        assert!(config.focus.foreground_window);
        assert_eq!(config.focus.foreground_source, ForegroundSource::Sway);
        assert_eq!(config.focus.foreground_poll_ms, 750);
        assert_eq!(config.focus.foreground_max_stale_ms, 3000);
        assert!(config.focus.foreground_include_title);

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn config_file_invalid_focus_source_is_rejected() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let dir = temp_dir("config-file-invalid-focus-source");
        let config_path = dir.join("config.toml");
        std::fs::write(&config_path, r#"focus_source = "dbus""#).unwrap();

        let _guard = EnvGuard::set("STUTTER_CONFIG", config_path.to_str().unwrap());

        let err = parse_monitor_config_from_inner(["stutter", "monitor"])
            .unwrap_err()
            .to_string();

        assert!(err.contains("focus_source must be heuristic, foreground, or hybrid"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn config_file_invalid_foreground_source_is_rejected() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let dir = temp_dir("config-file-invalid-foreground-source");
        let config_path = dir.join("config.toml");
        std::fs::write(&config_path, r#"foreground_source = "gnome""#).unwrap();

        let _guard = EnvGuard::set("STUTTER_CONFIG", config_path.to_str().unwrap());

        let err = parse_monitor_config_from_inner(["stutter", "monitor"])
            .unwrap_err()
            .to_string();

        assert!(err.contains("foreground_source must be auto, sway, hyprland, or x11"));

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn foreground_poll_ms_below_minimum_is_rejected() {
        let err = parse_monitor_config_from([
            "stutter",
            "monitor",
            "--foreground-window",
            "--foreground-poll-ms",
            "99",
        ])
        .unwrap_err()
        .to_string();

        assert!(err.contains("--foreground-poll-ms must be >= 100"));
    }

    #[test]
    fn foreground_max_stale_below_poll_interval_is_allowed_with_warning() {
        let config = parse_monitor_config_from([
            "stutter",
            "monitor",
            "--foreground-window",
            "--foreground-poll-ms",
            "1000",
            "--foreground-max-stale-ms",
            "500",
        ])
        .unwrap();

        assert!(config.focus.foreground_window);
        assert_eq!(config.focus.foreground_poll_ms, 1000);
        assert_eq!(config.focus.foreground_max_stale_ms, 500);
    }

    #[test]
    fn explicit_cli_false_overrides_user_file_true_after_monitor_config_resolution() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let dir = temp_dir("explicit-cli-false-overrides-user-file");
        let config_path = dir.join("config.toml");
        std::fs::write(
            &config_path,
            r#"
            hwmon = true
            cpu_freq = true
            "#,
        )
        .unwrap();

        let _guard = EnvGuard::set("STUTTER_CONFIG", config_path.to_str().unwrap());

        let config =
            parse_monitor_config_from_inner(["stutter", "monitor", "--no-hwmon", "--no-cpu-freq"])
                .unwrap();

        assert!(!config.probes.hwmon);
        assert!(!config.probes.cpu_freq);

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn explicit_cli_default_like_values_override_user_file_values() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let dir = temp_dir("explicit-cli-default-override");
        let config_path = dir.join("config.toml");
        std::fs::write(&config_path, "summary_ms = 750\n").unwrap();

        let _guard = EnvGuard::set("STUTTER_CONFIG", config_path.to_str().unwrap());

        let config =
            parse_monitor_config_from_inner(["stutter", "monitor", "--summary-ms", "1000"])
                .unwrap();

        assert_eq!(config.timing.summary_period_ms, 1000);

        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn explicit_cli_default_focus_values_override_user_file_values() {
        let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
        let dir = temp_dir("explicit-cli-default-focus-override");
        let config_path = dir.join("config.toml");
        std::fs::write(
            &config_path,
            r#"
            focus_source = "foreground"
            foreground_source = "sway"
            foreground_poll_ms = 777
            foreground_max_stale_ms = 3000
            "#,
        )
        .unwrap();

        let _guard = EnvGuard::set("STUTTER_CONFIG", config_path.to_str().unwrap());

        let config = parse_monitor_config_from_inner([
            "stutter",
            "monitor",
            "--focus-source",
            "heuristic",
            "--foreground-source",
            "auto",
            "--foreground-poll-ms",
            "1000",
            "--foreground-max-stale-ms",
            "2500",
        ])
        .unwrap();

        assert_eq!(config.focus.focus_source, FocusSource::Heuristic);
        assert_eq!(config.focus.foreground_source, ForegroundSource::Auto);
        assert_eq!(config.focus.foreground_poll_ms, 1000);
        assert_eq!(config.focus.foreground_max_stale_ms, 2500);
        assert!(!config.focus.foreground_window);

        std::fs::remove_dir_all(dir).ok();
    }
}

fn parse_optional_task_class(value: Option<&str>) -> anyhow::Result<Option<TaskClass>> {
    value
        .map(|s| {
            TaskClass::from_str_opt(s).ok_or_else(|| anyhow::anyhow!("unknown task class: {s}"))
        })
        .transpose()
}
