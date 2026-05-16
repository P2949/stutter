use std::path::PathBuf;

use clap::{Args, Subcommand};
use serde::Serialize;

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonArgs {
    #[command(subcommand)]
    pub command: Option<DaemonCommand>,
}

#[derive(Subcommand, Clone, Debug, Serialize)]
pub enum DaemonCommand {
    Config(DaemonConfigArgs),
    Policy(DaemonPolicyArgs),
    Profiles(DaemonProfilesArgs),
    Explain(DaemonExplainArgs),
    WhyNotOptimize(DaemonWhyNotOptimizeArgs),
    WhatChanged(DaemonWhatChangedArgs),
    Status(DaemonStatusArgs),
    Watch(DaemonWatchArgs),
    Doctor(DaemonDoctorArgs),
    ResetState(DaemonResetStateArgs),
    BenchOverhead(DaemonBenchOverheadArgs),
    Soak(DaemonSoakArgs),
    Acceptance(DaemonAcceptanceArgs),
    Pause(DaemonPauseArgs),
    Resume(DaemonResumeArgs),
    EmergencyRestore(DaemonRestoreArgs),
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonConfigArgs {
    #[command(subcommand)]
    pub command: DaemonConfigCommand,
}

#[derive(Subcommand, Clone, Debug, Serialize)]
pub enum DaemonConfigCommand {
    Explain(DaemonConfigExplainArgs),
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonConfigExplainArgs {
    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub preset: Option<String>,
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonPolicyArgs {
    #[command(subcommand)]
    pub command: DaemonPolicyCommand,
}

#[derive(Subcommand, Clone, Debug, Serialize)]
pub enum DaemonPolicyCommand {
    Explain(DaemonPolicyExplainArgs),
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonPolicyExplainArgs {
    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub preset: Option<String>,
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonProfilesArgs {
    #[command(subcommand)]
    pub command: DaemonProfilesCommand,
}

#[derive(Subcommand, Clone, Debug, Serialize)]
pub enum DaemonProfilesCommand {
    List(DaemonProfilesListArgs),
    Forget(DaemonProfilesForgetArgs),
    Explain(DaemonProfilesExplainArgs),
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonProfilesListArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonProfilesForgetArgs {
    #[arg(long)]
    pub json: bool,

    #[arg(long = "workload-identity-hash")]
    pub workload_identity_hash: Option<String>,

    #[arg(long)]
    pub candidate: Option<String>,

    #[arg(long)]
    pub all: bool,

    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonProfilesExplainArgs {
    #[arg(long)]
    pub json: bool,

    #[arg(long = "workload-identity-hash")]
    pub workload_identity_hash: Option<String>,
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonExplainArgs {
    #[arg(long)]
    pub json: bool,

    #[arg(long, default_value = "10")]
    pub explain_last: usize,
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonWhyNotOptimizeArgs {
    #[arg(long)]
    pub json: bool,

    #[arg(long, default_value = "10")]
    pub explain_last: usize,
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonWhatChangedArgs {
    #[arg(long)]
    pub json: bool,

    #[arg(long, default_value = "10")]
    pub explain_last: usize,
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonStatusArgs {
    #[arg(long)]
    pub json: bool,

    #[arg(long, default_value = "10")]
    pub explain_last: usize,
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonWatchArgs {
    #[arg(long)]
    pub verbose: bool,

    #[arg(long, default_value = "1000")]
    pub interval_ms: u64,

    #[arg(long)]
    pub iterations: Option<u64>,

    #[arg(long, default_value = "10")]
    pub explain_last: usize,
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonDoctorArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonResetStateArgs {
    #[arg(long)]
    pub json: bool,

    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonBenchOverheadArgs {
    #[arg(long)]
    pub json: bool,

    #[arg(long, default_value = "5000")]
    pub duration_ms: u64,
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonSoakArgs {
    #[arg(long)]
    pub json: bool,

    #[arg(long, default_value = "300000")]
    pub duration_ms: u64,

    #[arg(long)]
    pub preset: Option<String>,
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonAcceptanceArgs {
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonPauseArgs {}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonResumeArgs {}

#[derive(Args, Clone, Debug, Serialize)]
pub struct DaemonRestoreArgs {
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long)]
    pub emergency: bool,
}

pub fn daemon_config_from_soak_args(
    args: DaemonSoakArgs,
) -> anyhow::Result<crate::daemon::DaemonSoakConfig> {
    let profile = args
        .preset
        .as_deref()
        .unwrap_or("observe")
        .parse::<crate::daemon::DaemonSoakProfile>()?;
    Ok(crate::daemon::DaemonSoakConfig {
        profile,
        duration_seconds: args.duration_ms / 1000,
        tick_millis: 1000,
        budget: crate::daemon::DaemonSoakBudget::default(),
    })
}
