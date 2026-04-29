use std::{ffi::OsString, path::PathBuf, time::Duration};

use clap::{Args, Parser, Subcommand};

use crate::{TARGET_PIDS_MAX, process_tree::TaskFilters};

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
    Restore,
    ApplyProfile(ApplyProfileArgs),
}

#[derive(Args, Debug, Clone, Default)]
pub struct MonitorArgs {
    #[arg(long = "pid", short = 'p', value_name = "PID")]
    target_pids: Vec<u32>,

    #[arg(long = "tree-pid", value_name = "PID")]
    tree_pids: Vec<u32>,

    #[arg(long = "summary-ms", default_value_t = 1_000)]
    summary_period_ms: u64,

    #[arg(long = "spike-us", default_value_t = 1_000)]
    spike_threshold_us: u64,

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

    #[arg(long = "csv", value_name = "PATH")]
    csv_path: Option<PathBuf>,
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

    #[arg(long, default_value_t = 10, value_name = "N")]
    top: usize,

    #[arg(long = "cluster-ms", default_value_t = 5, value_name = "MS")]
    cluster_window_ms: u64,

    path: PathBuf,
}

#[derive(Args, Debug, Clone)]
struct ApplyProfileArgs {
    #[arg(long = "tree-pid", value_name = "PID")]
    tree_pid: u32,

    #[arg(long = "profile", value_name = "FILE")]
    profile: PathBuf,

    #[arg(long)]
    force: bool,

    #[arg(long)]
    watch: bool,

    #[arg(long = "keep-applied")]
    keep_applied: bool,

    #[arg(long = "refresh-ms", default_value_t = 1_000)]
    refresh_ms: u64,
}

#[derive(Debug)]
pub enum AppCommand {
    Monitor(Config),
    Restore,
    ApplyProfile {
        tree_pid: u32,
        profile: PathBuf,
        force: bool,
        watch: bool,
        keep_applied: bool,
        refresh_ms: u64,
    },
    InspectTree {
        tree_pid: u32,
    },
    Report {
        path: PathBuf,
        json: bool,
        top: usize,
        cluster_window_ms: u64,
    },
}

#[derive(Debug, Clone)]
pub struct Config {
    pub target_pids: Vec<u32>,
    pub tree_pids: Vec<u32>,
    pub summary_period_ms: u64,
    pub spike_threshold_ns: u64,
    pub verbose: bool,
    pub task_filters: TaskFilters,
    pub watch_process: Option<String>,
    pub persistent: bool,
    pub csv_path: Option<PathBuf>,
    pub recording: Option<RecordingConfig>,
    pub max_duration: Option<Duration>,
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
        Some(Command::Monitor(args)) => Ok(AppCommand::Monitor(config_from_monitor_args(
            args, false, None,
        )?)),
        Some(Command::Record(args)) => {
            if matches!(args.duration, Some(0)) {
                anyhow::bail!("--duration must be greater than zero");
            }

            let max_duration = args.duration.map(Duration::from_secs);
            Ok(AppCommand::Monitor(config_from_monitor_args(
                args.monitor,
                true,
                max_duration,
            )?))
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
            Ok(AppCommand::Report {
                path: args.path,
                json: args.json,
                top: args.top,
                cluster_window_ms: args.cluster_window_ms,
            })
        }
        Some(Command::Restore) => Ok(AppCommand::Restore),
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
                watch: args.watch,
                keep_applied: args.keep_applied,
                refresh_ms: args.refresh_ms,
            })
        }
        None => Ok(AppCommand::Monitor(config_from_monitor_args(
            cli.legacy_monitor,
            false,
            None,
        )?)),
    }
}

fn config_from_monitor_args(
    mut args: MonitorArgs,
    force_recording: bool,
    max_duration: Option<Duration>,
) -> anyhow::Result<Config> {
    validate_pids("--pid", &args.target_pids)?;
    validate_pids("--tree-pid", &args.tree_pids)?;

    if args.summary_period_ms == 0 {
        anyhow::bail!("--summary-ms must be greater than zero");
    }

    if args.spike_threshold_us == 0 {
        anyhow::bail!("--spike-us must be greater than zero");
    }

    if args.target_pids.is_empty() && args.tree_pids.is_empty() && args.watch_process.is_none() {
        anyhow::bail!(
            "at least one --pid <PID>, --tree-pid <PID>, or --watch-process <COMM> is required"
        );
    }

    args.target_pids.sort_unstable();
    args.target_pids.dedup();
    args.tree_pids.sort_unstable();
    args.tree_pids.dedup();
    args.include_comm.sort();
    args.include_comm.dedup();
    args.exclude_comm.sort();
    args.exclude_comm.dedup();

    validate_comm_patterns("--include-comm", &args.include_comm)?;
    validate_comm_patterns("--exclude-comm", &args.exclude_comm)?;
    if matches!(args.watch_process.as_deref(), Some("")) {
        anyhow::bail!("--watch-process must not be empty");
    }
    if args.persistent && args.watch_process.is_none() {
        anyhow::bail!("--persistent requires --watch-process");
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

    let recording = if force_recording || args.run_name.is_some() || args.out_dir.is_some() {
        Some(RecordingConfig {
            run_name: args
                .run_name
                .or_else(|| force_recording.then(|| "record".to_owned())),
            out_dir: args.out_dir,
        })
    } else {
        None
    };

    Ok(Config {
        target_pids: args.target_pids,
        tree_pids: args.tree_pids,
        summary_period_ms: args.summary_period_ms,
        spike_threshold_ns,
        verbose: args.verbose,
        task_filters: TaskFilters {
            include_comm: args.include_comm,
            exclude_comm: args.exclude_comm,
        },
        watch_process: args.watch_process,
        persistent: args.persistent,
        csv_path: args.csv_path,
        recording,
        max_duration,
    })
}

fn validate_pids(flag: &str, pids: &[u32]) -> anyhow::Result<()> {
    if pids.contains(&0) {
        anyhow::bail!("{flag} must be greater than zero");
    }
    Ok(())
}

fn validate_comm_patterns(flag: &str, patterns: &[String]) -> anyhow::Result<()> {
    if patterns.iter().any(|pattern| pattern.is_empty()) {
        anyhow::bail!("{flag} patterns must not be empty");
    }
    Ok(())
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
            "--cluster-ms",
            "5",
            "--top",
            "25",
            "/tmp/run",
        ])
        .unwrap();

        let AppCommand::Report {
            top,
            cluster_window_ms,
            ..
        } = command
        else {
            panic!("expected report command");
        };

        assert_eq!(top, 25);
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

        assert_eq!(config.task_filters.include_comm, vec!["RenderThread"]);
        assert_eq!(config.task_filters.exclude_comm, vec!["steamwebhelper"]);
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
    fn rejects_zero_report_cluster_window() {
        let err = parse_app_command_from(["stutter", "report", "--cluster-ms", "0", "/tmp/run"])
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("--cluster-ms must be greater than zero")
        );
    }

    #[test]
    fn parses_restore_and_apply_profile_commands() {
        let restore = parse_app_command_from(["stutter", "restore"]).unwrap();
        assert!(matches!(restore, AppCommand::Restore));

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
            watch,
            keep_applied,
            refresh_ms,
        } = apply
        else {
            panic!("expected apply profile command");
        };

        assert_eq!(tree_pid, 42);
        assert_eq!(profile, PathBuf::from("/tmp/profile.toml"));
        assert!(!force);
        assert!(!watch);
        assert!(!keep_applied);
        assert_eq!(refresh_ms, 1_000);
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
    fn rejects_persistent_without_watch_process() {
        let err = parse_app_command_from(["stutter", "monitor", "--pid", "42", "--persistent"])
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("--persistent requires --watch-process")
        );
    }
}
