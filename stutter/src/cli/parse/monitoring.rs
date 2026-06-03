use std::{sync::Arc, time::Duration};

use clap::ArgMatches;

use super::super::monitor::{
    BenchArgs, MonitorArgs, RecordArgs, RecordingMode, monitor_arg_presence_from_matches,
    monitor_config_from_monitor_args_with_presence,
};
use crate::commands::input::{AppCommand, BenchCommandInput, MonitorCommandInput};

pub(super) fn parse_monitor_command(
    args: MonitorArgs,
    matches: &ArgMatches,
) -> anyhow::Result<AppCommand> {
    Ok(AppCommand::Monitor(MonitorCommandInput {
        config: Arc::new(monitor_config_from_monitor_args_with_presence(
            args,
            RecordingMode::Monitor,
            monitor_arg_presence_from_matches(matches, Some("monitor")),
        )?),
    }))
}

pub(super) fn parse_record_command(
    args: RecordArgs,
    matches: &ArgMatches,
) -> anyhow::Result<AppCommand> {
    if matches!(args.duration, Some(0)) {
        anyhow::bail!("--duration must be greater than zero");
    }
    if args.monitor.no_record {
        anyhow::bail!("record --no-record is contradictory; use 'monitor' for non-recording runs");
    }
    let max_duration = args.duration.map(Duration::from_secs);
    Ok(AppCommand::Monitor(MonitorCommandInput {
        config: Arc::new(monitor_config_from_monitor_args_with_presence(
            args.monitor,
            RecordingMode::ForceRecording { max_duration },
            monitor_arg_presence_from_matches(matches, Some("record")),
        )?),
    }))
}

pub(super) fn parse_bench_command(
    mut args: BenchArgs,
    matches: &ArgMatches,
) -> anyhow::Result<AppCommand> {
    if args.duration == 0 {
        anyhow::bail!("--duration must be greater than zero");
    }
    let Some(scenario_name) = args.monitor.scenario_name.clone() else {
        anyhow::bail!("--scenario is required for bench");
    };
    if scenario_name.trim().is_empty() {
        anyhow::bail!("--scenario must not be empty");
    }
    if !matches!(args.role.as_str(), "baseline" | "current") {
        anyhow::bail!("--role must be baseline or current");
    }
    if args.monitor.no_record {
        anyhow::bail!("bench --no-record is contradictory");
    }
    let run_name = format!("bench-{}-{}", args.role, scenario_name);
    args.monitor.run_name = Some(run_name.clone());
    args.monitor.route_label = args.monitor.route_label.or(Some(scenario_name.clone()));
    let config = Arc::new(monitor_config_from_monitor_args_with_presence(
        args.monitor,
        RecordingMode::ForceRecording {
            max_duration: Some(Duration::from_secs(args.duration)),
        },
        monitor_arg_presence_from_matches(matches, Some("bench")),
    )?);
    Ok(AppCommand::Bench(BenchCommandInput {
        config,
        role: args.role,
        run_name,
    }))
}

pub(super) fn parse_legacy_monitor_command(
    args: MonitorArgs,
    matches: &ArgMatches,
) -> anyhow::Result<AppCommand> {
    Ok(AppCommand::Monitor(MonitorCommandInput {
        config: Arc::new(monitor_config_from_monitor_args_with_presence(
            args,
            RecordingMode::Monitor,
            monitor_arg_presence_from_matches(matches, None),
        )?),
    }))
}
