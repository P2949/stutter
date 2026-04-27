use std::{ffi::OsString, path::PathBuf, time::Duration};

use clap::{Args, Parser, Subcommand};

use crate::TARGET_PIDS_MAX;

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

    path: PathBuf,
}

pub enum AppCommand {
    Monitor(Config),
    InspectTree {
        tree_pid: u32,
    },
    Report {
        path: PathBuf,
        json: bool,
        top: usize,
    },
}

#[derive(Debug, Clone)]
pub struct Config {
    pub target_pids: Vec<u32>,
    pub tree_pids: Vec<u32>,
    pub summary_period_ms: u64,
    pub spike_threshold_ns: u64,
    pub verbose: bool,
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
            Ok(AppCommand::Report {
                path: args.path,
                json: args.json,
                top: args.top,
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

    if args.target_pids.is_empty() && args.tree_pids.is_empty() {
        anyhow::bail!("at least one --pid <PID> or --tree-pid <PID> is required");
    }

    args.target_pids.sort_unstable();
    args.target_pids.dedup();
    args.tree_pids.sort_unstable();
    args.tree_pids.dedup();

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
