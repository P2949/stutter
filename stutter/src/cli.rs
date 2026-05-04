use std::{ffi::OsString, path::PathBuf, sync::Arc, time::Duration};

use clap::{ArgAction, Args, Parser, Subcommand};

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
    InspectTree(InspectTreeArgs),
    Report(ReportArgs),
    Restore(RestoreArgs),
    ApplyProfile(ApplyProfileArgs),
    Tune(TuneArgs),
    Check(CheckArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct MonitorArgs {
    #[arg(long = "pid", short = 'p', value_name = "PID")]
    target_pids: Vec<u32>,

    #[arg(long = "tree-pid", value_name = "PID")]
    tree_pids: Vec<u32>,

    #[arg(long = "exclude-tree-pid", value_name = "PID")]
    exclude_tree_pids: Vec<u32>,

    #[arg(long = "summary-ms", default_value_t = 1_000)]
    summary_period_ms: u64,

    #[arg(long = "epoch", value_name = "MS")]
    epoch_period_ms: Option<u64>,

    #[arg(long = "spike-us", default_value_t = 1_000)]
    spike_threshold_us: u64,

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

    #[arg(long, default_value_t = 1024)]
    max_tasks: usize,

    #[arg(long = "csv", value_name = "PATH")]
    csv_path: Option<PathBuf>,

    #[arg(long = "irq-latency")]
    irq_latency: bool,

    #[arg(long = "irq", value_name = "IRQ")]
    irqs: Vec<u32>,

    #[arg(long = "hwmon")]
    hwmon: bool,

    #[arg(long = "hwmon-root", value_name = "PATH")]
    hwmon_root: Option<PathBuf>,

    #[arg(long = "hwmon-drm-card", value_name = "CARD")]
    hwmon_drm_card: Option<String>,

    #[arg(long = "hwmon-render-node", value_name = "NODE")]
    hwmon_render_node: Option<PathBuf>,

    #[arg(long = "mangohud-log", value_name = "PATH")]
    mangohud_log: Option<PathBuf>,

    #[arg(long = "tui")]
    tui: bool,

    #[arg(long = "retain-intervals", value_name = "N")]
    retain_intervals: Option<usize>,

    #[arg(long = "no-record")]
    no_record: bool,

    #[arg(long = "cpu-freq")]
    cpu_freq: bool,

    #[arg(long = "cgroupv2", value_name = "PATH")]
    cgroupv2: Option<PathBuf>,

    #[arg(
        long = "follow-exec",
        default_value_t = true,
        action = ArgAction::SetTrue,
        conflicts_with = "no_follow_exec"
    )]
    follow_exec: bool,

    #[arg(long = "no-follow-exec", action = ArgAction::SetTrue)]
    no_follow_exec: bool,

    #[arg(long = "faults")]
    faults: bool,

    #[arg(long = "block-io")]
    block_io: bool,

    #[arg(long = "stat-wait")]
    stat_wait: bool,
}

#[derive(Args, Debug, Clone)]
struct RecordArgs {
    #[command(flatten)]
    monitor: MonitorArgs,

    #[arg(long, value_name = "SECONDS")]
    duration: Option<u64>,
}

#[derive(Args, Debug, Clone)]
struct InspectTreeArgs {
    #[arg(long = "tree-pid", value_name = "PID")]
    tree_pid: u32,
}

#[derive(Args, Debug, Clone)]
struct ReportArgs {
    #[arg(long)]
    json: bool,

    #[arg(long = "html", value_name = "PATH")]
    html: Option<PathBuf>,

    #[arg(long, default_value_t = 10, value_name = "N")]
    top: usize,

    #[arg(long = "cluster-ms", default_value_t = 5, value_name = "MS")]
    cluster_window_ms: u64,

    #[arg(long = "diff", value_name = "PATH")]
    diff: Option<PathBuf>,

    #[arg(long = "filter-class", value_name = "CLASS")]
    filter_class: Option<String>,

    path: PathBuf,
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

    #[arg(long = "mangohud-log", value_name = "PATH")]
    pub mangohud_log: Option<PathBuf>,

    #[arg(long = "runs", short = 'n', default_value_t = 3, value_name = "N")]
    pub runs: u32,

    #[arg(long)]
    pub enforce: bool,
}

#[derive(Args, Debug, Clone)]
pub struct CheckArgs {
    #[arg(long = "baseline", value_name = "PATH")]
    pub baseline: PathBuf,

    #[arg(long = "current", value_name = "PATH")]
    pub current: PathBuf,

    #[arg(long = "max-regression-p99-ms", value_name = "MS")]
    pub max_regression_p99_ms: f64,
}

#[derive(Debug)]
pub enum AppCommand {
    Monitor(Arc<Config>),
    Restore {
        dry_run: bool,
    },
    ApplyProfile {
        tree_pid: u32,
        profile: PathBuf,
        force: bool,
        dry_run: bool,
        watch: bool,
        keep_applied: bool,
        refresh_ms: u64,
        enforce: bool,
    },
    InspectTree {
        tree_pid: u32,
    },
    Report {
        path: PathBuf,
        json: bool,
        html: Option<PathBuf>,
        top: usize,
        cluster_window_ms: u64,
        diff: Option<PathBuf>,
        filter_class: Option<TaskClass>,
    },
    Tune {
        tree_pid: u32,
        profiles: PathBuf,
        epoch_seconds: u64,
        warmup_seconds: u64,
        runs: u32,
        keep_best: bool,
        mangohud_log: Option<PathBuf>,
        enforce: bool,
    },
    Check {
        baseline: PathBuf,
        current: PathBuf,
        max_regression_p99_ms: f64,
    },
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
    pub csv_path: Option<PathBuf>,
    pub irq_latency: bool,
    pub irqs: Vec<u32>,
    pub hwmon: bool,
    pub hwmon_root: Option<PathBuf>,
    pub hwmon_drm_card: Option<String>,
    pub hwmon_render_node: Option<PathBuf>,
    pub mangohud_log: Option<PathBuf>,
    pub mangohud_ignore_offset: u64,
    pub tui: bool,
    pub retain_intervals: Option<usize>,
    pub recording: Option<RecordingConfig>,
    pub max_duration: Option<Duration>,
    pub cpu_freq: bool,
    pub cgroupv2: Option<PathBuf>,
    pub follow_exec: bool,
    pub exclude_tree_pids: Vec<u32>,
    pub faults: bool,
    pub block_io: bool,
    pub stat_wait: bool,
}

#[derive(Debug, Clone)]
pub struct RecordingConfig {
    pub run_name: Option<String>,
    pub out_dir: Option<PathBuf>,
}

pub fn parse_app_command() -> anyhow::Result<AppCommand> {
    parse_app_command_from(std::env::args_os())
}

fn parse_app_command_from<I, T>(args: I) -> anyhow::Result<AppCommand>
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
                html: args.html,
                top: args.top,
                cluster_window_ms: args.cluster_window_ms,
                diff: args.diff,
                filter_class,
            })
        }
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
                mangohud_log: args.mangohud_log,
                enforce: args.enforce,
            })
        }
        Some(Command::Check(args)) => {
            if !args.max_regression_p99_ms.is_finite() || args.max_regression_p99_ms < 0.0 {
                anyhow::bail!("--max-regression-p99-ms must be a finite non-negative value");
            }
            Ok(AppCommand::Check {
                baseline: args.baseline,
                current: args.current,
                max_regression_p99_ms: args.max_regression_p99_ms,
            })
        }
        None => Ok(AppCommand::Monitor(Arc::new(config_from_monitor_args(
            cli.legacy_monitor,
            false,
            None,
        )?))),
    }
}

fn config_from_monitor_args(
    mut args: MonitorArgs,
    force_recording: bool,
    max_duration: Option<Duration>,
) -> anyhow::Result<Config> {
    validate_pids("--pid", &args.target_pids)?;
    validate_pids("--tree-pid", &args.tree_pids)?;
    validate_pids("--exclude-tree-pid", &args.exclude_tree_pids)?;

    if args.summary_period_ms == 0 {
        anyhow::bail!("--summary-ms must be greater than zero");
    }
    if matches!(args.epoch_period_ms, Some(0)) {
        anyhow::bail!("--epoch must be greater than zero");
    }

    if args.spike_threshold_us == 0 {
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
    if args.max_tasks == 0 {
        anyhow::bail!("--max-tasks must be greater than zero");
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
    if (args.hwmon_root.is_some()
        || args.hwmon_drm_card.is_some()
        || args.hwmon_render_node.is_some())
        && !args.hwmon
    {
        anyhow::bail!("--hwmon-root, --hwmon-drm-card, and --hwmon-render-node require --hwmon");
    }

    if args.target_pids.len() > TARGET_PIDS_MAX {
        anyhow::bail!(
            "too many unique target PIDs: got {}, but TARGET_PIDS supports at most {}",
            args.target_pids.len(),
            TARGET_PIDS_MAX
        );
    }

    let spike_threshold_ns = args
        .spike_threshold_us
        .checked_mul(1_000)
        .ok_or_else(|| anyhow::anyhow!("--spike-us value is too large"))?;
    let summary_period_ms = args.epoch_period_ms.unwrap_or(args.summary_period_ms);
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

    let cpu_freq = args.cpu_freq || recording.is_some();
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
        max_tasks: args.max_tasks,
        csv_path: args.csv_path,
        irq_latency: args.irq_latency,
        irqs: args.irqs,
        hwmon: args.hwmon,
        hwmon_root: args.hwmon_root,
        hwmon_drm_card: args.hwmon_drm_card,
        hwmon_render_node: args.hwmon_render_node,
        mangohud_log: args.mangohud_log,
        tui: args.tui,
        retain_intervals: args.retain_intervals,
        recording,
        max_duration,
        cpu_freq,
        cgroupv2: args.cgroupv2,
        follow_exec: args.follow_exec && !args.no_follow_exec,
        exclude_tree_pids: args.exclude_tree_pids,
        mangohud_ignore_offset: 0,
        faults: args.faults,
        block_io: args.block_io,
        stat_wait: args.stat_wait,
    })
}

fn validate_pids(flag: &str, pids: &[u32]) -> anyhow::Result<()> {
    if pids.contains(&0) {
        anyhow::bail!("{flag} must be greater than zero");
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
mod tests {
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
        assert_eq!(config.csv_path, Some(PathBuf::from("/tmp/stutter.csv")));
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
            "--watch",
            "--keep-applied",
            "--refresh-ms",
            "250",
        ])
        .unwrap();

        let AppCommand::ApplyProfile {
            force,
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
            mangohud_log,
            enforce,
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
        assert_eq!(mangohud_log, Some(PathBuf::from("/tmp/tune-mango.csv")));
        assert!(!enforce);
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
        } = command
        else {
            panic!("expected check command");
        };

        assert_eq!(baseline, PathBuf::from("run1/"));
        assert_eq!(current, PathBuf::from("run2/"));
        assert_eq!(max_regression_p99_ms, 2.5);
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
}
