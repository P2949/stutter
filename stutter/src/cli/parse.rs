use std::ffi::OsString;

use clap::{CommandFactory, Parser};

use super::{
    app::{Cli, Command},
    version::requested_version_features,
};
use crate::commands::input::{AppCommand, VersionCommandInput};

#[path = "parse/agent.rs"]
mod agent;
#[path = "parse/autotune.rs"]
mod autotune;
#[path = "parse/daemon.rs"]
mod daemon;
#[path = "parse/monitoring.rs"]
mod monitoring;
#[path = "parse/reports.rs"]
mod reports;
#[path = "parse/rules_scenario.rs"]
mod rules_scenario;
#[path = "parse/service.rs"]
mod service;

pub(crate) fn parse_app_command() -> anyhow::Result<AppCommand> {
    parse_app_command_from(std::env::args_os())
}

pub(crate) fn parse_app_command_from<I, T>(args: I) -> anyhow::Result<AppCommand>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let argv: Vec<OsString> = args.into_iter().map(Into::into).collect();
    if requested_version_features(&argv) {
        return Ok(AppCommand::Version(VersionCommandInput { features: true }));
    }

    let matches = Cli::command().try_get_matches_from(argv.clone())?;
    let cli = Cli::try_parse_from(argv)?;

    match cli.command {
        Some(Command::Monitor(args)) => monitoring::parse_monitor_command(args, &matches),
        Some(Command::Record(args)) => monitoring::parse_record_command(args, &matches),
        Some(Command::Bench(args)) => monitoring::parse_bench_command(args, &matches),
        Some(Command::InspectTree(args)) => reports::parse_inspect_tree_command(args),
        Some(Command::Report(args)) => reports::parse_report_command(args),
        Some(Command::Summary(args)) => reports::parse_summary_command(args),
        Some(Command::Validate(args)) => reports::parse_validate_command(args),
        Some(Command::Restore(args)) => reports::parse_restore_command(args),
        Some(Command::ApplyProfile(args)) => reports::parse_apply_profile_command(args),
        Some(Command::ProfilePlan(args)) => reports::parse_profile_plan_command(args),
        Some(Command::Tune(args)) => reports::parse_tune_command(args),
        Some(Command::Recommend(args)) => reports::parse_recommend_command(args),
        Some(Command::ProveFix(args)) => reports::parse_prove_fix_command(args),
        Some(Command::Release(args)) => reports::parse_release_command(args),
        Some(Command::Check(args)) => reports::parse_check_command(args),
        Some(Command::Compare(args)) => reports::parse_compare_command(args),
        Some(Command::Config(args)) => reports::parse_config_command(args),
        Some(Command::Audit(args)) => reports::parse_audit_command(args),
        Some(Command::Advisor(args)) => reports::parse_advisor_command(args),
        Some(Command::Doctor(args)) => reports::parse_doctor_command(args),
        Some(Command::ProfileTemplate(args)) => reports::parse_profile_template_command(args),
        Some(Command::InspectIrqs(args)) => reports::parse_inspect_irqs_command(args),
        Some(Command::InspectDrmTracepoints(args)) => {
            reports::parse_inspect_drm_tracepoints_command(args)
        }
        Some(Command::WaylandProbe(args)) => reports::parse_wayland_probe_command(args),
        Some(Command::Autotune(args)) => autotune::parse_autotune_command(args),
        Some(Command::AutotuneStatus(args)) => autotune::parse_autotune_status_command(args),
        None => monitoring::parse_legacy_monitor_command(cli.legacy_monitor, &matches),
        Some(Command::Agent(args)) => agent::parse_agent_command(args),
        Some(Command::PrivilegedWorker(args)) => agent::parse_privileged_worker_command(args),
        Some(Command::Daemon(args)) => daemon::parse_daemon_command(args),
        Some(Command::Service(args)) => service::parse_service_command(args),
        Some(Command::Completions(args)) => reports::parse_completions_command(args),
        Some(Command::Man(args)) => reports::parse_man_command(args),
        Some(Command::Probes(args)) => reports::parse_probes_command(args),
        Some(Command::Rules(args)) => rules_scenario::parse_rules_command(args),
        Some(Command::Scenario(args)) => rules_scenario::parse_scenario_command(args),
    }
}

pub fn command() -> clap::Command {
    Cli::command()
}
