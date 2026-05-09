use std::{ffi::OsString, path::PathBuf, sync::Arc, time::Duration};

use clap::{ArgAction, Args, CommandFactory, Parser, Subcommand};
use serde::{Deserialize, Serialize};

pub const TARGET_PIDS_MAX: usize = 1024;

use crate::process_tree::{CompiledPattern, TaskClass, TaskFilters};

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Profile scheduler runnable latency for selected tasks"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    legacy_monitor: MonitorArgs,
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
        default_value_t = ForegroundSourceArg::Auto,
        help = "Foreground-window provider: auto, sway, hyprland, x11"
    )]
    foreground_source: ForegroundSourceArg,

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

    #[arg(long = "mode", default_value = "observe")]
    pub mode: String,

    #[arg(long = "decision-log", value_name = "PATH")]
    pub decision_log: Option<PathBuf>,

    #[arg(long = "duration-seconds")]
    pub duration_seconds: Option<u64>,

    #[arg(long = "summary-ms", default_value_t = 1000)]
    pub summary_ms: u64,

    #[arg(long = "preset", default_value = "diagnosis")]
    pub preset: String,

    #[arg(long = "hwmon")]
    pub hwmon: bool,

    #[arg(long = "mangohud-log")]
    pub mangohud_log: Option<PathBuf>,
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
            foreground_source: ForegroundSourceArg::Auto,
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
    Remove(RulesRemoveArgs),
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

#[derive(Debug)]
pub enum AppCommand {
    Monitor(Arc<Config>),
    Bench {
        config: Arc<Config>,
        role: String,
        run_name: String,
    },
    Restore {
        dry_run: bool,
    },
    ApplyProfile {
        tree_pid: u32,
        profile: PathBuf,
        force: bool,
        dry_run: bool,
        allow_medium_risk: bool,
        watch: bool,
        keep_applied: bool,
        refresh_ms: u64,
        enforce: bool,
    },
    InspectTree {
        tree_pid: u32,
    },
    Report {
        path: Option<PathBuf>,
        json: bool,
        analysis_json: bool,
        json_summary: bool,
        html: Option<PathBuf>,
        top: usize,
        cluster_window_ms: u64,
        batch: Option<PathBuf>,
        diff: Option<PathBuf>,
        filter_class: Option<TaskClass>,
        flamegraph: Option<PathBuf>,
    },
    Summary {
        path: PathBuf,
        json: bool,
        top: usize,
        filter_class: Option<TaskClass>,
    },
    Validate {
        path: PathBuf,
        json: bool,
        strict: bool,
    },
    Tune {
        tree_pid: u32,
        profiles: PathBuf,
        epoch_seconds: u64,
        warmup_seconds: u64,
        runs: u32,
        keep_best: bool,
        baseline_profile: Option<String>,
        out_dir: Option<PathBuf>,
        mangohud_log: Option<PathBuf>,
        enforce: bool,
        hwmon: bool,
    },
    Recommend {
        baseline: PathBuf,
        tune: PathBuf,
        json: bool,
        markdown: Option<PathBuf>,
    },
    Check {
        baseline: PathBuf,
        current: PathBuf,
        max_regression_p99_ms: Option<f64>,
        max_max_regression_ms: Option<f64>,
        json: bool,
        top: usize,
        filter_class: Option<TaskClass>,
    },
    Autotune {
        input: crate::autotune::AutotuneCommandInput,
    },
    Audit {
        path: Option<PathBuf>,
        tail: usize,
        json: bool,
    },
    AutotuneReplay {
        run: PathBuf,
        config: Option<PathBuf>,
    },
    Probes {
        json: bool,
    },
    Rules {
        command: RulesCommand,
    },
    ScenarioCreate {
        name: String,
        force: bool,
        watch_process: Option<String>,
        duration: u64,
        preset: String,
        mangohud_log: Option<PathBuf>,
        notes: Option<String>,
    },
    ScenarioRun {
        name: String,
        role: String,
        dry_run: bool,
        out_dir: Option<PathBuf>,
        mangohud_log_override: Option<PathBuf>,
    },
    ScenarioCompare {
        name: String,
        baseline: Option<PathBuf>,
        current: Option<PathBuf>,
        top: usize,
        json_summary: bool,
        validate: bool,
    },
    ScenarioPath {
        name: String,
    },
    ScenarioList,
    Advisor {
        run: Option<PathBuf>,
        profiles: Option<PathBuf>,
        json: bool,
        watch_runs: bool,
        runs_dir: Option<PathBuf>,
        poll_seconds: u64,
        once: bool,
    },
    Doctor {
        input: crate::doctor::DoctorInput,
    },
    ProfileTemplate {
        topology: bool,
    },
    InspectIrqs {
        json: bool,
        filter: Vec<String>,
        top: usize,
    },
    AutotuneGenerateProfiles {
        watch_process: Option<String>,
        out: PathBuf,
        allow_cpus: Option<String>,
        deny_cpus: Option<String>,
        min_render_cpus: usize,
        min_game_cpus: usize,
        min_compositor_cpus: usize,
        min_background_cpus: usize,
    },
    AutotuneStatus {
        json: bool,
    },
    Agent {
        bind: std::net::SocketAddr,
        runs_dir: Option<std::path::PathBuf>,
        allow_unsafe_bind: bool,
        bearer_token_env: String,
        bearer_token_file: Option<std::path::PathBuf>,
        max_duration_seconds: u64,
        max_targets: usize,
        max_concurrent_recordings: usize,
    },
    Completions {
        shell: clap_complete::Shell,
    },
    AutotuneReplayHistory {
        history: PathBuf,
    },
    AutotuneRestore {
        journal: Option<PathBuf>,
        audit: Option<PathBuf>,
        history: Option<PathBuf>,
        dry_run: bool,
    },
    Man {
        output: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FocusSource {
    Heuristic,
    Foreground,
    Hybrid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForegroundSourceArg {
    Auto,
    Sway,
    Hyprland,
    X11,
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

impl ForegroundSourceArg {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CsvStreamTarget {
    File(PathBuf),
    Stdout,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutoFocusConfig {
    pub enabled: bool,
    pub source: FocusSource,
    pub poll_ms: u64,
    pub min_confidence: f32,
    pub switch_cooldown_ms: u64,
    pub switch_margin: f32,
    pub required_polls: u32,
    pub max_roots: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub struct ForegroundWindowConfig {
    pub enabled: bool,
    pub source: ForegroundSourceArg,
    pub poll_ms: u64,
    pub max_stale_ms: u64,
    pub include_title: bool,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub target_pids: Vec<u32>,
    pub tree_pids: Vec<u32>,
    pub summary_period_ms: u64,
    pub epoch_period_ms: Option<u64>,
    pub spike_threshold_ns: u64,
    pub alert_threshold_ns: Option<u64>,
    pub alert_webhook_url: Option<String>,
    pub verbose: bool,
    pub task_filters: TaskFilters,
    pub keep_missing_pid: bool,
    pub watch_process: Option<String>,
    pub persistent: bool,
    pub watch_poll_ms: u64,
    pub watch_timeout: Option<Duration>,
    pub max_tasks: usize,
    pub csv_stream: Option<CsvStreamTarget>,
    pub irq_latency: bool,
    pub irqs: Vec<u32>,
    pub hwmon: bool,
    pub hwmon_root: Option<PathBuf>,
    pub hwmon_drm_card: Option<String>,
    pub hwmon_render_node: Option<PathBuf>,
    pub mangohud_log: Option<PathBuf>,
    pub mangohud_log_live: bool,
    pub otlp_endpoint: Option<String>,
    pub otel_service_name: String,
    pub auto_focus: bool,
    pub focus_source: FocusSource,
    #[allow(dead_code)]
    pub foreground_window: bool,
    #[allow(dead_code)]
    pub foreground_source: ForegroundSourceArg,
    #[allow(dead_code)]
    pub foreground_poll_ms: u64,
    #[allow(dead_code)]
    pub foreground_max_stale_ms: u64,
    #[allow(dead_code)]
    pub foreground_include_title: bool,
    #[allow(dead_code)]
    pub auto_focus_poll_ms: u64,
    pub auto_focus_min_confidence: f32,
    pub auto_focus_switch_cooldown_ms: u64,
    pub auto_focus_switch_margin: f32,
    #[allow(dead_code)]
    pub auto_focus_required_polls: u32,
    #[allow(dead_code)]
    pub auto_focus_max_roots: usize,
    pub remote: Option<String>,

    pub tui: bool,
    pub retain_intervals: Option<usize>,
    pub recording: Option<RecordingConfig>,
    pub max_duration: Option<Duration>,
    pub cpu_freq: bool,
    pub cgroupv2: Option<PathBuf>,
    // Experimental: native cgroup filtering applies to current-task probes only.
    // Scheduler wakee filtering still uses TARGET_PIDS.
    pub native_cgroup_filter: bool,
    pub follow_exec: bool,
    pub exclude_tree_pids: Vec<u32>,
    pub faults: bool,
    pub cpu_perf: bool,
    pub cpu_perf_kernel: bool,
    pub cpu_perf_max_tasks: usize,
    pub cpu_perf_cache_refs: bool,
    pub block_io: bool,
    pub stat_wait: bool,
    pub json_stream: bool,
    pub metrics_port: Option<u16>,
    pub ringbuf_size_kb: Option<u32>,
    pub wakeup_map_factor: Option<u32>,
}

impl Config {
    pub fn csv_streams_to_stdout(&self) -> bool {
        matches!(self.csv_stream, Some(CsvStreamTarget::Stdout))
    }

    pub fn has_explicit_target(&self) -> bool {
        !self.target_pids.is_empty()
            || !self.tree_pids.is_empty()
            || self.watch_process.is_some()
            || self.cgroupv2.is_some()
    }

    #[allow(dead_code)]
    pub fn auto_focus_enabled(&self) -> bool {
        self.auto_focus && !self.has_explicit_target()
    }

    #[allow(dead_code)]
    pub fn auto_focus_config(&self) -> AutoFocusConfig {
        AutoFocusConfig {
            enabled: self.auto_focus_enabled(),
            source: self.focus_source,
            poll_ms: self.auto_focus_poll_ms,
            min_confidence: self.auto_focus_min_confidence,
            switch_cooldown_ms: self.auto_focus_switch_cooldown_ms,
            switch_margin: self.auto_focus_switch_margin,
            required_polls: self.auto_focus_required_polls,
            max_roots: self.auto_focus_max_roots,
        }
    }

    #[allow(dead_code)]
    pub fn foreground_window_config(&self) -> ForegroundWindowConfig {
        ForegroundWindowConfig {
            enabled: self.foreground_window
                || (self.auto_focus
                    && matches!(
                        self.focus_source,
                        FocusSource::Foreground | FocusSource::Hybrid
                    )),
            source: self.foreground_source,
            poll_ms: self.foreground_poll_ms,
            max_stale_ms: self.foreground_max_stale_ms,
            include_title: self.foreground_include_title,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RecordingConfig {
    pub run_name: Option<String>,
    pub out_dir: Option<PathBuf>,
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
) -> anyhow::Result<Arc<Config>> {
    let monitor = MonitorArgs {
        watch_process: input.watch_process.clone(),
        tree_pids: input.tree_pid.map_or(Vec::new(), |pid| vec![pid]),
        summary_period_ms: Some(input.summary_ms),
        preset: Some(input.preset.clone()),
        hwmon: input.hwmon,
        no_hwmon: !input.hwmon,
        mangohud_log: input.mangohud_log.clone(),
        no_record: input.decision_log.is_none(),
        run_name: Some("autotune-observe".to_owned()),
        ..Default::default()
    };

    Ok(Arc::new(config_from_monitor_args(monitor, false, None)?))
}

pub fn parse_app_command() -> anyhow::Result<AppCommand> {
    parse_app_command_from(std::env::args_os())
}

pub fn parse_app_command_from<I, T>(args: I) -> anyhow::Result<AppCommand>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = Cli::try_parse_from(args)?;

    match cli.command {
        Some(Command::Monitor(args)) => Ok(AppCommand::Monitor(Arc::new(
            config_from_monitor_args(args, false, None)?,
        ))),
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
            Ok(AppCommand::Monitor(Arc::new(config_from_monitor_args(
                args.monitor,
                true,
                max_duration,
            )?)))
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
            Ok(AppCommand::Bench {
                config: Arc::new(config_from_monitor_args(
                    args.monitor,
                    true,
                    Some(Duration::from_secs(args.duration)),
                )?),
                role: args.role,
                run_name,
            })
        }
        Some(Command::InspectTree(args)) => {
            if args.tree_pid == 0 {
                anyhow::bail!("--tree-pid must be greater than zero");
            }
            Ok(AppCommand::InspectTree {
                tree_pid: args.tree_pid,
            })
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
            let filter_class = if let Some(class_str) = &args.filter_class {
                Some(
                    TaskClass::from_str_opt(class_str)
                        .ok_or_else(|| anyhow::anyhow!("unknown task class: {class_str}"))?,
                )
            } else {
                None
            };
            Ok(AppCommand::Report {
                path: args.path,
                json: args.json,
                analysis_json: args.analysis_json,
                json_summary: args.json_summary,
                html: args.html,
                top: args.top,
                cluster_window_ms: args.cluster_window_ms,
                batch: args.batch,
                diff: args.diff,
                filter_class,
                flamegraph: args.flamegraph,
            })
        }
        Some(Command::Summary(args)) => {
            if args.top == 0 {
                anyhow::bail!("--top must be greater than zero");
            }
            let filter_class = if let Some(class_str) = &args.filter_class {
                Some(
                    TaskClass::from_str_opt(class_str)
                        .ok_or_else(|| anyhow::anyhow!("unknown task class: {class_str}"))?,
                )
            } else {
                None
            };
            Ok(AppCommand::Summary {
                path: args.path,
                json: args.json,
                top: args.top,
                filter_class,
            })
        }
        Some(Command::Validate(args)) => Ok(AppCommand::Validate {
            path: args.path,
            json: args.json,
            strict: args.strict,
        }),
        Some(Command::Restore(args)) => Ok(AppCommand::Restore {
            dry_run: args.dry_run,
        }),
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
            Ok(AppCommand::ApplyProfile {
                tree_pid: args.tree_pid,
                profile: args.profile,
                force: args.force,
                dry_run: args.dry_run,
                allow_medium_risk: args.allow_medium_risk,
                watch: args.watch,
                keep_applied: args.keep_applied,
                refresh_ms: args.refresh_ms,
                enforce: args.enforce,
            })
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
            Ok(AppCommand::Tune {
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
            })
        }
        Some(Command::Recommend(args)) => Ok(AppCommand::Recommend {
            baseline: args.baseline,
            tune: args.tune,
            json: args.json,
            markdown: args.markdown,
        }),
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
            let filter_class = if let Some(class_str) = &args.filter_class {
                Some(
                    TaskClass::from_str_opt(class_str)
                        .ok_or_else(|| anyhow::anyhow!("unknown task class: {class_str}"))?,
                )
            } else {
                None
            };
            Ok(AppCommand::Check {
                baseline: args.baseline,
                current: args.current,
                max_regression_p99_ms: args.max_regression_p99_ms,
                max_max_regression_ms: args.max_max_regression_ms,
                json: args.json,
                top: args.top,
                filter_class,
            })
        }
        Some(Command::Autotune(args)) => {
            if let Some(cmd) = args.command {
                match cmd {
                    AutotuneCommand::GenerateProfiles(args) => {
                        Ok(AppCommand::AutotuneGenerateProfiles {
                            watch_process: args.watch_process,
                            out: args.out,
                            allow_cpus: args.allow_cpus,
                            deny_cpus: args.deny_cpus,
                            min_render_cpus: args.min_render_cpus,
                            min_game_cpus: args.min_game_cpus,
                            min_compositor_cpus: args.min_compositor_cpus,
                            min_background_cpus: args.min_background_cpus,
                        })
                    }
                    AutotuneCommand::Replay(replay) => Ok(AppCommand::AutotuneReplay {
                        run: replay.run,
                        config: replay.config,
                    }),
                    AutotuneCommand::ReplayHistory(replay_args) => {
                        Ok(AppCommand::AutotuneReplayHistory {
                            history: replay_args.history,
                        })
                    }
                    AutotuneCommand::Restore(args) => Ok(AppCommand::AutotuneRestore {
                        journal: args.journal,
                        audit: args.audit,
                        history: args.history,
                        dry_run: args.dry_run,
                    }),
                }
            } else {
                validate_autotune_mode(&args.mode)?;
                Ok(AppCommand::Autotune {
                    input: crate::autotune::AutotuneCommandInput {
                        config: args.config,
                        watch_process: args.watch_process,
                        tree_pid: args.tree_pid,
                        profiles: args.profiles,
                        mode: args.mode,
                        decision_log: args.decision_log,
                        duration_seconds: args.duration_seconds,
                        summary_ms: args.summary_ms,
                        preset: args.preset,
                        hwmon: args.hwmon,
                        mangohud_log: args.mangohud_log,
                    },
                })
            }
        }
        Some(Command::AutotuneStatus(args)) => Ok(AppCommand::AutotuneStatus { json: args.json }),
        Some(Command::Audit(args)) => Ok(AppCommand::Audit {
            path: args.path,
            tail: args.tail,
            json: args.json,
        }),
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
            Ok(AppCommand::Advisor {
                run: args.run,
                profiles: args.profiles,
                json: args.json,
                watch_runs: args.watch_runs,
                runs_dir: args.runs_dir,
                poll_seconds: args.poll_seconds,
                once: args.once,
            })
        }
        Some(Command::Doctor(args)) => Ok(AppCommand::Doctor {
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
        }),
        Some(Command::ProfileTemplate(args)) => Ok(AppCommand::ProfileTemplate {
            topology: args.topology,
        }),
        Some(Command::InspectIrqs(args)) => {
            if args.top == 0 {
                anyhow::bail!("--top must be greater than zero");
            }
            Ok(AppCommand::InspectIrqs {
                json: args.json,
                filter: args.filter.clone(),
                top: args.top,
            })
        }
        None => Ok(AppCommand::Monitor(Arc::new(config_from_monitor_args(
            cli.legacy_monitor,
            false,
            None,
        )?))),
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
            Ok(AppCommand::Agent {
                bind,
                runs_dir: args.runs_dir,
                allow_unsafe_bind: args.allow_unsafe_bind,
                bearer_token_env: args.bearer_token_env,
                bearer_token_file: args.bearer_token_file,
                max_duration_seconds: args.max_duration_seconds,
                max_targets: args.max_targets,
                max_concurrent_recordings: args.max_concurrent_recordings,
            })
        }
        Some(Command::Completions(args)) => Ok(AppCommand::Completions { shell: args.shell }),
        Some(Command::Man(args)) => Ok(AppCommand::Man {
            output: args.output,
        }),
        Some(Command::Probes(args)) => Ok(AppCommand::Probes { json: args.json }),
        Some(Command::Rules(args)) => Ok(AppCommand::Rules {
            command: args.command,
        }),
        Some(Command::Scenario(args)) => match args.command {
            ScenarioCommand::Create(args) => {
                if args.name.trim().is_empty() {
                    anyhow::bail!("scenario name must not be empty");
                }
                if args.duration == 0 {
                    anyhow::bail!("scenario duration must be greater than zero");
                }
                Ok(AppCommand::ScenarioCreate {
                    name: args.name,
                    force: args.force,
                    watch_process: args.watch_process,
                    duration: args.duration,
                    preset: args.preset,
                    mangohud_log: args.mangohud_log,
                    notes: args.notes,
                })
            }
            ScenarioCommand::Run(args) => {
                if args.name.trim().is_empty() {
                    anyhow::bail!("scenario name must not be empty");
                }
                if !matches!(args.role.as_str(), "baseline" | "current") {
                    anyhow::bail!("--role must be baseline or current");
                }
                Ok(AppCommand::ScenarioRun {
                    name: args.name,
                    role: args.role,
                    dry_run: args.dry_run,
                    out_dir: args.out_dir,
                    mangohud_log_override: args.mangohud_log_override,
                })
            }
            ScenarioCommand::Compare(args) => {
                if args.name.trim().is_empty() {
                    anyhow::bail!("scenario name must not be empty");
                }
                if args.top == 0 {
                    anyhow::bail!("--top must be greater than zero");
                }
                Ok(AppCommand::ScenarioCompare {
                    name: args.name,
                    baseline: args.baseline,
                    current: args.current,
                    top: args.top,
                    json_summary: args.json_summary,
                    validate: args.validate,
                })
            }
            ScenarioCommand::Path(args) => {
                if args.name.trim().is_empty() {
                    anyhow::bail!("scenario name must not be empty");
                }
                Ok(AppCommand::ScenarioPath { name: args.name })
            }
            ScenarioCommand::List => Ok(AppCommand::ScenarioList),
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

pub fn config_from_monitor_args(
    args: MonitorArgs,
    force_recording: bool,
    max_duration: Option<Duration>,
) -> anyhow::Result<Config> {
    let file_config = crate::config_file::load_user_config()?.unwrap_or_default();
    config_from_monitor_args_with_file(args, file_config, force_recording, max_duration)
}

fn config_from_monitor_args_with_file(
    mut args: MonitorArgs,
    file_config: crate::config_file::UserConfigFile,
    force_recording: bool,
    max_duration: Option<Duration>,
) -> anyhow::Result<Config> {
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
    } else if let Some(config_include) = file_config.include_comm {
        args.include_comm = config_include;
    }

    if !args.exclude_comm.is_empty() {
        // use CLI
    } else if let Some(config_exclude) = file_config.exclude_comm {
        args.exclude_comm = config_exclude;
    }

    let hwmon = merge_bool(
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

    // Re-evaluating irq_latency based on prompt: "Do not auto-enable all IRQ monitoring... Presets do not modify IRQ latency. Document that users should pass --irq-latency --irq N explicitly."
    // But "lightweight" should disable it.
    // Let's use merge_bool but with builtin = false.
    let irq_latency = merge_bool(
        false,
        None,
        preset_defaults.irq_latency,
        args.irq_latency,
        false, // No no-irq-latency flag yet
    );

    let retain_intervals = args.retain_intervals.or(file_config.retain_intervals);

    if let Some(foreground_window) = file_config.foreground_window {
        args.foreground_window = foreground_window;
    }

    if let Some(focus_source) = file_config.focus_source.as_deref() {
        args.focus_source = FocusSource::parse_config_value(focus_source)?;
    }

    if let Some(foreground_source) = file_config.foreground_source.as_deref() {
        args.foreground_source = ForegroundSourceArg::parse_config_value(foreground_source)?;
    }

    if let Some(foreground_poll_ms) = file_config.foreground_poll_ms {
        args.foreground_poll_ms = foreground_poll_ms;
    }

    if let Some(foreground_max_stale_ms) = file_config.foreground_max_stale_ms {
        args.foreground_max_stale_ms = foreground_max_stale_ms;
    }

    if let Some(foreground_include_title) = file_config.foreground_include_title {
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

    let include_comm = validate_comm_patterns("--include-comm", &args.include_comm)?;
    let exclude_comm = validate_comm_patterns("--exclude-comm", &args.exclude_comm)?;
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
    let alert_webhook_url = if alert_threshold_ns.is_some() {
        args.alert_webhook_url.or_else(|| {
            std::env::var("STUTTER_ALERT_WEBHOOK_URL")
                .ok()
                .filter(|url| !url.is_empty())
        })
    } else {
        args.alert_webhook_url
    };

    let recording = if args.no_record {
        None
    } else if force_recording || args.run_name.is_some() || args.out_dir.is_some() {
        Some(RecordingConfig {
            run_name: args
                .run_name
                .or_else(|| force_recording.then(|| "record".to_owned())),
            out_dir: args.out_dir,
        })
    } else {
        None
    };

    let cpu_freq = (cpu_freq_config || recording.is_some()) && !args.no_cpu_freq;
    if matches!(args.metrics_port, Some(0)) {
        anyhow::bail!("--metrics-port must be greater than zero");
    }

    Ok(Config {
        target_pids: args.target_pids,
        tree_pids: args.tree_pids,
        summary_period_ms,
        epoch_period_ms: args.epoch_period_ms,
        spike_threshold_ns,
        alert_threshold_ns,
        alert_webhook_url,
        verbose: args.verbose,
        task_filters: TaskFilters {
            include_comm,
            exclude_comm,
        },
        keep_missing_pid: args.keep_missing_pid,
        watch_process: args.watch_process,
        persistent: args.persistent,
        watch_poll_ms: args.watch_poll_ms,
        watch_timeout: args.watch_timeout_seconds.map(Duration::from_secs),
        max_tasks,
        csv_stream: {
            let csv_stream = match (&args.csv_path, &args.stream_csv) {
                (Some(path), None) => Some(CsvStreamTarget::File(path.clone())),
                (None, Some(value)) if value == "-" => Some(CsvStreamTarget::Stdout),
                (None, Some(value)) if value.trim().is_empty() => {
                    anyhow::bail!("--stream-csv path must not be empty");
                }
                (None, Some(value)) => Some(CsvStreamTarget::File(PathBuf::from(value))),
                (None, None) => None,
                (Some(_), Some(_)) => {
                    anyhow::bail!("--stream-csv conflicts with --csv");
                }
            };
            if matches!(csv_stream, Some(CsvStreamTarget::Stdout)) && args.json_stream {
                anyhow::bail!(
                    "--stream-csv - cannot be used with --json-stream because both write to stdout"
                );
            }
            csv_stream
        },
        irq_latency,
        irqs: args.irqs,
        hwmon,
        hwmon_root: args.hwmon_root,
        hwmon_drm_card: args.hwmon_drm_card,
        hwmon_render_node: args.hwmon_render_node,
        mangohud_log: args.mangohud_log,
        mangohud_log_live: args.mangohud_log_live,

        tui: args.tui,
        retain_intervals,
        recording,
        max_duration,
        cpu_freq,
        cgroupv2: args.cgroupv2,
        native_cgroup_filter: args.native_cgroup_filter,
        follow_exec: args.follow_exec && !args.no_follow_exec,
        exclude_tree_pids: args.exclude_tree_pids,
        faults,
        cpu_perf: args.cpu_perf,
        cpu_perf_kernel: args.cpu_perf_kernel,
        cpu_perf_max_tasks: args.cpu_perf_max_tasks,
        cpu_perf_cache_refs: args.cpu_perf_cache_refs,
        block_io,
        stat_wait,
        json_stream: args.json_stream,
        metrics_port: args.metrics_port,
        ringbuf_size_kb: args.ringbuf_size_kb,
        wakeup_map_factor: args.wakeup_map_factor,
        otlp_endpoint: args.otlp_endpoint,
        otel_service_name: args.otel_service_name,
        auto_focus: args.auto_focus,
        focus_source: args.focus_source,
        foreground_window: args.foreground_window,
        foreground_source: args.foreground_source,
        foreground_poll_ms: args.foreground_poll_ms,
        foreground_max_stale_ms: args.foreground_max_stale_ms,
        foreground_include_title: args.foreground_include_title,
        auto_focus_poll_ms: args.auto_focus_poll_ms,
        auto_focus_min_confidence: args.auto_focus_min_confidence,
        auto_focus_switch_cooldown_ms: args.auto_focus_switch_cooldown_ms,
        auto_focus_switch_margin: args.auto_focus_switch_margin,
        auto_focus_required_polls: args.auto_focus_required_polls,
        auto_focus_max_roots: args.auto_focus_max_roots,
        remote: args.remote,
    })
}

fn validate_pids(flag: &str, pids: &[u32]) -> anyhow::Result<()> {
    if pids.contains(&0) {
        anyhow::bail!("{flag} must be greater than zero");
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

fn validate_comm_patterns(flag: &str, patterns: &[String]) -> anyhow::Result<Vec<CompiledPattern>> {
    let mut compiled = Vec::new();
    for pattern in patterns {
        if pattern.is_empty() {
            anyhow::bail!("{flag} patterns must not be empty");
        }
        compiled.push(CompiledPattern::new(pattern.clone())?);
    }
    Ok(compiled)
}

#[cfg(test)]
fn parse_monitor_config_for_phase15<const N: usize>(
    args: [&str; N],
) -> anyhow::Result<Arc<Config>> {
    let _lock = crate::test_support::TEST_MUTEX.lock().unwrap();
    match parse_app_command_from(args.iter().map(OsString::from))? {
        AppCommand::Monitor(config) => Ok(config),
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

    assert!(config.auto_focus);
    assert_eq!(config.focus_source, FocusSource::Foreground);
    assert!(config.foreground_window);
    assert_eq!(config.foreground_source, ForegroundSourceArg::Sway);
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
    assert!(foreground_window.foreground_window);
    assert!(foreground_window.foreground_include_title);

    let foreground_focus = parse_monitor_config_for_phase15([
        "stutter",
        "monitor",
        "--auto-focus",
        "--focus-source",
        "foreground",
        "--foreground-include-title",
    ])
    .unwrap();
    assert!(foreground_focus.auto_focus);
    assert_eq!(foreground_focus.focus_source, FocusSource::Foreground);
    assert!(foreground_focus.foreground_window);
    assert!(foreground_focus.foreground_include_title);
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
    ) -> anyhow::Result<Arc<Config>> {
        match parse_app_command_from_inner(args.iter().map(OsString::from))? {
            AppCommand::Monitor(config) => Ok(config),
            other => anyhow::bail!("expected AppCommand::Monitor, got {other:?}"),
        }
    }

    fn parse_monitor_config_from<const N: usize>(args: [&str; N]) -> anyhow::Result<Arc<Config>> {
        match parse_app_command_from(args.iter().map(OsString::from))? {
            AppCommand::Monitor(config) => Ok(config),
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

        let AppCommand::Report {
            top,
            html,
            cluster_window_ms,
            ..
        } = command
        else {
            panic!("expected report command");
        };

        assert_eq!(top, 25);
        assert_eq!(html, Some(PathBuf::from("/tmp/report.html")));
        assert_eq!(cluster_window_ms, 5);
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

        let AppCommand::Summary {
            path,
            json,
            top,
            filter_class,
        } = command
        else {
            panic!("expected summary command");
        };

        assert_eq!(path, PathBuf::from("/tmp/run"));
        assert!(json);
        assert_eq!(top, 3);
        assert_eq!(filter_class, Some(TaskClass::Game));
    }

    #[test]
    fn validate_requires_path() {
        assert!(parse_app_command_from(["stutter", "validate"]).is_err());
    }

    #[test]
    fn validate_accepts_path() {
        let command = parse_app_command_from(["stutter", "validate", "/tmp/run"]).unwrap();

        let AppCommand::Validate { path, json, strict } = command else {
            panic!("expected validate command");
        };

        assert_eq!(path, PathBuf::from("/tmp/run"));
        assert!(!json);
        assert!(!strict);
    }

    #[test]
    fn validate_accepts_json() {
        let command =
            parse_app_command_from(["stutter", "validate", "--json", "/tmp/run"]).unwrap();

        let AppCommand::Validate { path, json, strict } = command else {
            panic!("expected validate command");
        };

        assert_eq!(path, PathBuf::from("/tmp/run"));
        assert!(json);
        assert!(!strict);
    }

    #[test]
    fn validate_accepts_strict() {
        let command =
            parse_app_command_from(["stutter", "validate", "--strict", "/tmp/run"]).unwrap();

        let AppCommand::Validate { path, json, strict } = command else {
            panic!("expected validate command");
        };

        assert_eq!(path, PathBuf::from("/tmp/run"));
        assert!(!json);
        assert!(strict);
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

        let AppCommand::Report {
            batch,
            json_summary,
            top,
            path,
            ..
        } = command
        else {
            panic!("expected report command");
        };

        assert_eq!(batch, Some(PathBuf::from("/tmp/runs")));
        assert!(json_summary);
        assert_eq!(top, 4);
        assert_eq!(path, None);
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

        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };

        assert_eq!(
            config.task_filters.include_comm,
            vec![CompiledPattern::new("RenderThread".to_owned()).unwrap()]
        );
        assert_eq!(
            config.task_filters.exclude_comm,
            vec![CompiledPattern::new("steamwebhelper".to_owned()).unwrap()]
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

        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };

        assert_eq!(config.tree_pids, vec![42]);
        assert_eq!(config.exclude_tree_pids, vec![7, 100]);
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

        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };

        assert_eq!(config.alert_threshold_ns, Some(250_000_000));
        assert_eq!(
            config.alert_webhook_url.as_deref(),
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

        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };

        assert_eq!(config.epoch_period_ms, Some(5_000));
        assert_eq!(config.summary_period_ms, 5_000);
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

        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };

        assert_eq!(config.watch_process.as_deref(), Some("KingdomCome"));
        assert_eq!(
            config.csv_stream,
            Some(CsvStreamTarget::File(PathBuf::from("/tmp/stutter.csv")))
        );
    }

    #[test]
    fn follow_exec_defaults_on_and_can_be_disabled() {
        let default_command =
            parse_app_command_from(["stutter", "monitor", "--pid", "42"]).unwrap();
        let AppCommand::Monitor(default_config) = default_command else {
            panic!("expected monitor command");
        };
        assert!(default_config.follow_exec);

        let disabled_command =
            parse_app_command_from(["stutter", "monitor", "--pid", "42", "--no-follow-exec"])
                .unwrap();
        let AppCommand::Monitor(disabled_config) = disabled_command else {
            panic!("expected monitor command");
        };
        assert!(!disabled_config.follow_exec);
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

        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };

        assert_eq!(
            config.cgroupv2.as_deref(),
            Some(std::path::Path::new("/sys/fs/cgroup/test.slice"))
        );
        assert!(config.native_cgroup_filter);
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

        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };

        assert!(!config.native_cgroup_filter);
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

        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };

        assert!(config.cpu_perf);
        assert!(config.cpu_perf_kernel);
        assert_eq!(config.cpu_perf_max_tasks, 16);
        assert!(!config.cpu_perf_cache_refs);
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

        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };

        assert!(config.cpu_perf);
        assert!(config.cpu_perf_cache_refs);
        assert!(config.recording.is_some());
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

        let AppCommand::Check {
            baseline,
            current,
            max_regression_p99_ms,
            max_max_regression_ms,
            json,
            top,
            filter_class,
        } = command
        else {
            panic!("expected check command");
        };

        assert_eq!(baseline, PathBuf::from("/tmp/base"));
        assert_eq!(current, PathBuf::from("/tmp/current"));
        assert_eq!(max_regression_p99_ms, Some(0.5));
        assert_eq!(max_max_regression_ms, Some(2.0));
        assert!(json);
        assert_eq!(top, 5);
        assert_eq!(filter_class, Some(TaskClass::Game));
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
        assert!(matches!(restore, AppCommand::Restore { dry_run: false }));

        let apply = parse_app_command_from([
            "stutter",
            "apply-profile",
            "--tree-pid",
            "42",
            "--profile",
            "/tmp/profile.toml",
        ])
        .unwrap();

        let AppCommand::ApplyProfile {
            tree_pid,
            profile,
            force,
            dry_run,
            allow_medium_risk,
            watch,
            keep_applied,
            refresh_ms,
            enforce,
        } = apply
        else {
            panic!("expected apply profile command");
        };

        assert_eq!(tree_pid, 42);
        assert_eq!(profile, PathBuf::from("/tmp/profile.toml"));
        assert!(!force);
        assert!(!dry_run);
        assert!(!allow_medium_risk);
        assert!(!watch);
        assert!(!keep_applied);
        assert_eq!(refresh_ms, 1_000);
        assert!(!enforce);
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

        let AppCommand::ApplyProfile {
            force,
            allow_medium_risk,
            dry_run: _,
            watch,
            keep_applied,
            refresh_ms,
            ..
        } = command
        else {
            panic!("expected apply profile command");
        };

        assert!(force);
        assert!(allow_medium_risk);
        assert!(watch);
        assert!(keep_applied);
        assert_eq!(refresh_ms, 250);
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

        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };

        assert!(config.keep_missing_pid);
        assert_eq!(config.watch_poll_ms, 500);
        assert_eq!(config.watch_timeout, Some(Duration::from_secs(3)));
    }

    #[test]
    fn parses_restore_dry_run() {
        let command = parse_app_command_from(["stutter", "restore", "--dry-run"]).unwrap();
        assert!(matches!(command, AppCommand::Restore { dry_run: true }));
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

        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };

        assert!(config.irq_latency);
        assert_eq!(config.irqs, vec![137]);
        assert!(config.hwmon);
        assert_eq!(config.hwmon_drm_card.as_deref(), Some("card1"));
        assert_eq!(
            config.hwmon_render_node,
            Some(PathBuf::from("/dev/dri/renderD129"))
        );
        assert_eq!(config.mangohud_log, Some(PathBuf::from("/tmp/mango.csv")));
        assert!(config.tui);
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

        let AppCommand::Tune {
            tree_pid,
            profiles,
            epoch_seconds,
            warmup_seconds,
            runs,
            keep_best,
            baseline_profile,
            out_dir,
            mangohud_log,
            enforce,
            hwmon,
        } = command
        else {
            panic!("expected tune command");
        };

        assert_eq!(tree_pid, 42);
        assert_eq!(profiles, PathBuf::from("/tmp/profiles.toml"));
        assert_eq!(epoch_seconds, 60);
        assert_eq!(warmup_seconds, 10);
        assert_eq!(runs, 3);
        assert!(keep_best);
        assert_eq!(baseline_profile, None);
        assert_eq!(out_dir, None);
        assert_eq!(mangohud_log, Some(PathBuf::from("/tmp/tune-mango.csv")));
        assert!(!enforce);
        assert!(!hwmon);
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

        let AppCommand::Bench {
            config,
            role,
            run_name,
        } = command
        else {
            panic!("expected bench command");
        };

        assert_eq!(role, "baseline");
        assert_eq!(run_name, "bench-baseline-route-a");
        assert_eq!(config.max_duration, Some(Duration::from_secs(180)));
        assert_eq!(
            config.recording.as_ref().unwrap().run_name.as_deref(),
            Some("bench-baseline-route-a")
        );
        assert_eq!(config.watch_process.as_deref(), Some("Game.exe"));
        assert!(config.persistent);
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

        let AppCommand::Bench { config, .. } = command else {
            panic!("expected bench command");
        };

        assert_eq!(config.watch_process.as_deref(), Some("Game.exe"));
        assert!(config.hwmon);
        assert_eq!(config.mangohud_log, Some(PathBuf::from("/tmp/mango.csv")));
    }

    #[test]
    fn parses_doctor_command() {
        let command = parse_app_command_from(["stutter", "doctor"]).unwrap();

        let AppCommand::Doctor { input } = command else {
            panic!("expected doctor command");
        };

        assert!(!input.json);
        assert!(!input.hwmon);
        assert!(!input.irq_latency);
    }

    #[test]
    fn probes_command_parses() {
        let command = parse_app_command_from(["stutter", "probes"]).unwrap();

        let AppCommand::Probes { json } = command else {
            panic!("expected probes command");
        };

        assert!(!json);
    }

    #[test]
    fn probes_json_parses() {
        let command = parse_app_command_from(["stutter", "probes", "--json"]).unwrap();

        let AppCommand::Probes { json } = command else {
            panic!("expected probes command");
        };

        assert!(json);
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

        let AppCommand::Audit { path, tail, json } = command else {
            panic!("expected audit command");
        };

        assert_eq!(path, Some(PathBuf::from("/tmp/actions.jsonl")));
        assert_eq!(tail, 50);
        assert!(json);
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

        let AppCommand::Advisor {
            run,
            profiles,
            json,
            watch_runs,
            ..
        } = command
        else {
            panic!("expected advisor command");
        };

        assert_eq!(run, Some(PathBuf::from("/tmp/run")));
        assert_eq!(profiles, Some(PathBuf::from("profiles.toml")));
        assert!(json);
        assert!(!watch_runs);
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

        let AppCommand::Advisor {
            run,
            watch_runs,
            runs_dir,
            poll_seconds,
            once,
            ..
        } = command
        else {
            panic!("expected advisor command");
        };

        assert_eq!(run, None);
        assert!(watch_runs);
        assert_eq!(runs_dir, Some(PathBuf::from("/tmp/runs")));
        assert_eq!(poll_seconds, 1);
        assert!(once);
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

        let AppCommand::Doctor { input } = command else {
            panic!("expected doctor command");
        };

        assert!(input.json);
        assert!(input.hwmon);
        assert_eq!(input.hwmon_root, Some(PathBuf::from("/tmp/fake")));
    }

    #[test]
    fn parses_doctor_irq_latency_without_irq() {
        let command = parse_app_command_from(["stutter", "doctor", "--irq-latency"]).unwrap();

        let AppCommand::Doctor { input } = command else {
            panic!("expected doctor command");
        };

        assert!(input.irq_latency);
        assert!(input.irqs.is_empty());
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

        let AppCommand::Check {
            baseline,
            current,
            max_regression_p99_ms,
            max_max_regression_ms,
            json,
            top,
            filter_class,
        } = command
        else {
            panic!("expected check command");
        };

        assert_eq!(baseline, PathBuf::from("run1/"));
        assert_eq!(current, PathBuf::from("run2/"));
        assert_eq!(max_regression_p99_ms, Some(2.5));
        assert_eq!(max_max_regression_ms, None);
        assert!(!json);
        assert_eq!(top, 10);
        assert_eq!(filter_class, None);
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

        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };

        assert!(config.faults);
        assert!(config.block_io);
        assert!(config.stat_wait);
    }

    #[test]
    fn record_enables_cpu_freq_by_default_but_can_be_disabled() {
        let command = parse_app_command_from(["stutter", "record", "--pid", "42"]).unwrap();
        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };
        assert!(config.cpu_freq);

        let command =
            parse_app_command_from(["stutter", "record", "--pid", "42", "--no-cpu-freq"]).unwrap();
        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };
        assert!(!config.cpu_freq);
    }

    #[test]
    fn monitor_disables_cpu_freq_by_default_but_can_be_enabled() {
        let command = parse_app_command_from(["stutter", "monitor", "--pid", "42"]).unwrap();
        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };
        assert!(!config.cpu_freq);

        let command =
            parse_app_command_from(["stutter", "monitor", "--pid", "42", "--cpu-freq"]).unwrap();
        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };
        assert!(config.cpu_freq);
    }

    #[test]
    fn monitor_json_stream_flag_sets_config() {
        let command =
            parse_app_command_from(["stutter", "monitor", "--pid", "42", "--json-stream"]).unwrap();
        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };
        assert!(config.json_stream);

        let command = parse_app_command_from(["stutter", "monitor", "--pid", "42"]).unwrap();
        let AppCommand::Monitor(config) = command else {
            panic!("expected monitor command");
        };
        assert!(!config.json_stream);
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
        let config = config_from_monitor_args_with_file(args, file_config, false, None).unwrap();
        assert_eq!(config.summary_period_ms, 500);
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
        let config = config_from_monitor_args_with_file(args, file_config, false, None).unwrap();
        assert_eq!(config.summary_period_ms, 200);
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
        let config = config_from_monitor_args_with_file(args, file_config, false, None).unwrap();
        assert_eq!(config.task_filters.include_comm.len(), 2);
        // They get sorted in config_from_monitor_args
        assert_eq!(config.task_filters.include_comm[0].raw, "Game");
        assert_eq!(config.task_filters.include_comm[1].raw, "Render");
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
        let config = config_from_monitor_args_with_file(args, file_config, false, None).unwrap();
        assert_eq!(config.task_filters.include_comm.len(), 1);
        assert_eq!(config.task_filters.include_comm[0].raw, "RenderThread");
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
            config_from_monitor_args_with_file(args, Default::default(), false, None).unwrap();

        assert!(config.hwmon);
        assert!(config.cpu_freq);
        assert!(config.faults);
        assert!(config.stat_wait);
        assert!(config.block_io);
        assert!(!config.irq_latency);
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
            config_from_monitor_args_with_file(args, Default::default(), false, None).unwrap();
        assert!(!config.cpu_freq);
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
            config_from_monitor_args_with_file(args, Default::default(), false, None).unwrap();

        assert!(!config.hwmon);
        assert!(!config.cpu_freq);
        assert!(!config.faults);
        assert!(!config.stat_wait);
        assert!(!config.block_io);
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
            config_from_monitor_args_with_file(args, Default::default(), false, None).unwrap();
        assert!(config.faults);
    }

    #[test]
    fn stream_csv_path_sets_file_target() {
        let args = MonitorArgs {
            target_pids: vec![1234],
            stream_csv: Some("out.csv".to_owned()),
            ..Default::default()
        };
        let config = config_from_monitor_args(args, false, None).unwrap();

        assert!(matches!(
            config.csv_stream,
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
        let config = config_from_monitor_args(args, false, None).unwrap();

        assert!(matches!(config.csv_stream, Some(CsvStreamTarget::Stdout)));
    }

    #[test]
    fn stream_csv_stdout_conflicts_with_json_stream() {
        let args = MonitorArgs {
            target_pids: vec![1234],
            stream_csv: Some("-".to_owned()),
            json_stream: true,
            ..Default::default()
        };
        let err = config_from_monitor_args(args, false, None).unwrap_err();
        assert!(err.to_string().contains("stdout"));
    }

    #[test]
    fn agent_accepts_allow_unsafe_bind() {
        let command = parse_app_command_from(["stutter", "agent", "--allow-unsafe-bind"]).unwrap();
        let AppCommand::Agent {
            allow_unsafe_bind, ..
        } = command
        else {
            panic!("expected agent command");
        };
        assert!(allow_unsafe_bind);
    }

    #[test]
    fn agent_accepts_bearer_token_file() {
        let command =
            parse_app_command_from(["stutter", "agent", "--bearer-token-file", "/tmp/token"])
                .unwrap();
        let AppCommand::Agent {
            bearer_token_file, ..
        } = command
        else {
            panic!("expected agent command");
        };
        assert_eq!(bearer_token_file, Some(PathBuf::from("/tmp/token")));
    }

    #[test]
    fn agent_accepts_bearer_token_env() {
        let command =
            parse_app_command_from(["stutter", "agent", "--bearer-token-env", "MY_TOKEN"]).unwrap();
        let AppCommand::Agent {
            bearer_token_env, ..
        } = command
        else {
            panic!("expected agent command");
        };
        assert_eq!(bearer_token_env, "MY_TOKEN");
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

        let AppCommand::ScenarioCreate {
            name,
            duration,
            watch_process,
            ..
        } = command
        else {
            panic!("expected ScenarioCreate command");
        };

        assert_eq!(name, "kcd-route");
        assert_eq!(duration, 60);
        assert_eq!(watch_process, Some("KingdomCome.exe".to_owned()));
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
        let AppCommand::ScenarioRun { name, role, .. } = command else {
            panic!("expected ScenarioRun command");
        };
        assert_eq!(name, "kcd-route");
        assert_eq!(role, "baseline");
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
        let AppCommand::ScenarioRun { role, .. } = command else {
            panic!("expected ScenarioRun command");
        };
        assert_eq!(role, "current");
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
        let AppCommand::ScenarioCompare { name, top, .. } = command else {
            panic!("expected ScenarioCompare command");
        };
        assert_eq!(name, "kcd-route");
        assert_eq!(top, 5);
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
        let AppCommand::ScenarioPath { name } = command else {
            panic!("expected ScenarioPath command");
        };
        assert_eq!(name, "kcd-route");
    }

    #[test]
    fn monitor_defaults_keep_heuristic_focus_and_no_foreground_window() {
        let config = parse_monitor_config_from(["stutter", "monitor"]).unwrap();

        assert!(!config.auto_focus);
        assert_eq!(config.focus_source, FocusSource::Heuristic);
        assert!(!config.foreground_window);
        assert!(!config.foreground_window_config().enabled);
        assert_eq!(config.foreground_source, ForegroundSourceArg::Auto);
        assert_eq!(config.foreground_poll_ms, 1000);
        assert_eq!(config.foreground_max_stale_ms, 2500);
        assert!(!config.foreground_include_title);

        let foreground = config.foreground_window_config();
        assert!(!foreground.enabled);
        assert_eq!(foreground.source, ForegroundSourceArg::Auto);
        assert_eq!(foreground.poll_ms, 1000);
        assert_eq!(foreground.max_stale_ms, 2500);
        assert!(!foreground.include_title);
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

        assert_eq!(config.target_pids, vec![1234]);
        assert_eq!(config.tree_pids, vec![5678]);
        assert!(!config.auto_focus);
        assert_eq!(config.focus_source, FocusSource::Heuristic);
        assert!(config.foreground_window);
        assert_eq!(config.foreground_source, ForegroundSourceArg::Sway);
        assert_eq!(config.foreground_poll_ms, 750);
        assert_eq!(config.foreground_max_stale_ms, 3000);

        let foreground = config.foreground_window_config();
        assert!(foreground.enabled);
        assert_eq!(foreground.source, ForegroundSourceArg::Sway);
        assert_eq!(foreground.poll_ms, 750);
        assert_eq!(foreground.max_stale_ms, 3000);
        assert!(!foreground.include_title);
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

        assert!(config.auto_focus);
        assert_eq!(config.focus_source, FocusSource::Foreground);
        assert!(config.foreground_window);
        assert_eq!(config.foreground_source, ForegroundSourceArg::Sway);
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
        assert!(foreground_window.foreground_window);
        assert!(foreground_window.foreground_include_title);

        let foreground_focus = parse_monitor_config_from([
            "stutter",
            "monitor",
            "--auto-focus",
            "--focus-source",
            "foreground",
            "--foreground-include-title",
        ])
        .unwrap();
        assert!(foreground_focus.auto_focus);
        assert_eq!(foreground_focus.focus_source, FocusSource::Foreground);
        assert!(foreground_focus.foreground_window);
        assert!(foreground_focus.foreground_include_title);
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

        assert!(config.auto_focus);
        assert_eq!(config.focus_source, FocusSource::Foreground);
        assert!(
            config.foreground_window,
            "non-heuristic focus_source must normalize foreground_window to true"
        );
        assert_eq!(config.foreground_source, ForegroundSourceArg::X11);
        assert!(config.foreground_include_title);

        let auto_focus = config.auto_focus_config();
        assert!(auto_focus.enabled);
        assert_eq!(auto_focus.source, FocusSource::Foreground);

        let foreground = config.foreground_window_config();
        assert!(foreground.enabled);
        assert_eq!(foreground.source, ForegroundSourceArg::X11);
        assert!(foreground.include_title);
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

        assert!(config.auto_focus);
        assert_eq!(config.focus_source, FocusSource::Hybrid);
        assert!(
            config.foreground_window,
            "hybrid focus_source must normalize foreground_window to true"
        );

        let auto_focus = config.auto_focus_config();
        assert!(auto_focus.enabled);
        assert_eq!(auto_focus.source, FocusSource::Hybrid);

        let foreground = config.foreground_window_config();
        assert!(foreground.enabled);
        assert_eq!(foreground.source, ForegroundSourceArg::Auto);
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

        assert_eq!(config.focus_source, FocusSource::Foreground);
        assert!(config.foreground_window);
        assert_eq!(config.foreground_source, ForegroundSourceArg::Sway);
        assert_eq!(config.foreground_poll_ms, 750);
        assert_eq!(config.foreground_max_stale_ms, 3000);
        assert!(config.foreground_include_title);

        let foreground = config.foreground_window_config();
        assert!(foreground.enabled);
        assert_eq!(foreground.source, ForegroundSourceArg::Sway);
        assert_eq!(foreground.poll_ms, 750);
        assert_eq!(foreground.max_stale_ms, 3000);
        assert!(foreground.include_title);

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

        assert!(config.foreground_window);
        assert_eq!(config.foreground_poll_ms, 1000);
        assert_eq!(config.foreground_max_stale_ms, 500);
    }
}
