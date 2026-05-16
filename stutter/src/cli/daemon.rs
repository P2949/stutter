use super::*;

#[derive(Args, Debug, Clone)]
pub struct DaemonArgs {
    #[command(subcommand)]
    pub command: DaemonCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DaemonCommand {
    Config(DaemonConfigArgs),
    Policy(DaemonPolicyArgs),
    Profiles(DaemonProfilesArgs),
    Explain(DaemonExplainArgs),
    #[command(name = "why-not-optimize")]
    WhyNotOptimize(DaemonWhyNotOptimizeArgs),
    #[command(name = "what-changed")]
    WhatChanged(DaemonWhatChangedArgs),
    Status(DaemonStatusArgs),
    Watch(DaemonWatchArgs),
    Doctor(DaemonDoctorArgs),
    #[command(name = "reset-state")]
    ResetState(DaemonResetStateArgs),
    #[command(name = "bench-overhead")]
    BenchOverhead(DaemonBenchOverheadArgs),
    Soak(DaemonSoakArgs),
    Acceptance(DaemonAcceptanceArgs),
    Pause(DaemonPauseArgs),
    Resume(DaemonResumeArgs),
    Restore(DaemonRestoreArgs),
    #[command(name = "emergency-restore")]
    EmergencyRestore(DaemonRestoreArgs),
}

#[derive(Args, Debug, Clone)]
pub struct DaemonPolicyArgs {
    #[command(subcommand)]
    pub command: DaemonPolicyCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DaemonPolicyCommand {
    Explain(DaemonPolicyExplainArgs),
}

#[derive(Args, Debug, Clone)]
pub struct DaemonPolicyExplainArgs {
    #[arg(long = "json")]
    pub json: bool,

    #[arg(long = "preset", value_name = "NAME")]
    pub preset: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct DaemonProfilesArgs {
    #[command(subcommand)]
    pub command: DaemonProfilesCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub enum DaemonProfilesCommand {
    List(DaemonProfilesListArgs),
    Forget(DaemonProfilesForgetArgs),
    Explain(DaemonProfilesExplainArgs),
}

#[derive(Args, Debug, Clone)]
pub struct DaemonProfilesListArgs {
    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DaemonProfilesForgetArgs {
    #[arg(long = "workload-hash", value_name = "HASH")]
    pub workload_identity_hash: Option<String>,

    #[arg(long = "candidate", value_name = "NAME_OR_ACTION_ID")]
    pub candidate: Option<String>,

    #[arg(long = "all")]
    pub all: bool,

    #[arg(long = "dry-run")]
    pub dry_run: bool,

    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DaemonProfilesExplainArgs {
    #[arg(long = "workload-hash", value_name = "HASH")]
    pub workload_identity_hash: Option<String>,

    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DaemonExplainArgs {
    #[arg(long = "json")]
    pub json: bool,

    #[arg(long = "explain-last", default_value_t = 10)]
    pub explain_last: usize,
}

#[derive(Args, Debug, Clone)]
pub struct DaemonWhyNotOptimizeArgs {
    #[arg(long = "json")]
    pub json: bool,

    #[arg(long = "explain-last", default_value_t = 10)]
    pub explain_last: usize,
}

#[derive(Args, Debug, Clone)]
pub struct DaemonWhatChangedArgs {
    #[arg(long = "json")]
    pub json: bool,

    #[arg(long = "explain-last", default_value_t = 10)]
    pub explain_last: usize,
}

#[derive(Args, Debug, Clone)]
pub struct DaemonConfigArgs {
    #[arg(long = "explain")]
    pub explain: bool,

    #[arg(long = "json", requires = "explain")]
    pub json: bool,

    #[arg(long = "preset", value_name = "NAME")]
    pub preset: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub struct DaemonStatusArgs {
    #[arg(long = "json")]
    pub json: bool,

    #[arg(long = "explain-last", default_value_t = 10)]
    pub explain_last: usize,
}

#[derive(Args, Debug, Clone)]
pub struct DaemonWatchArgs {
    #[arg(long = "interval-ms", default_value_t = 1_000)]
    pub interval_ms: u64,

    #[arg(long = "iterations", value_name = "N")]
    pub iterations: Option<u64>,

    #[arg(long = "verbose")]
    pub verbose: bool,

    #[arg(long = "explain-last", default_value_t = 10)]
    pub explain_last: usize,
}

#[derive(Args, Debug, Clone)]
pub struct DaemonDoctorArgs {
    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DaemonResetStateArgs {
    #[arg(long = "dry-run")]
    pub dry_run: bool,

    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DaemonBenchOverheadArgs {
    #[arg(long = "json")]
    pub json: bool,

    #[arg(long = "duration-ms", default_value_t = 1_000)]
    pub duration_ms: u64,
}

#[derive(Args, Debug, Clone)]
pub struct DaemonSoakArgs {
    #[arg(long = "json")]
    pub json: bool,

    #[arg(long = "duration-seconds", default_value_t = 60)]
    pub duration_seconds: u64,

    #[arg(long = "tick-ms", default_value_t = 1_000)]
    pub tick_ms: u64,

    #[arg(long = "profile", default_value = "observe-only")]
    pub profile: String,

    #[arg(long = "max-disk-growth-bytes")]
    pub max_disk_growth_bytes: Option<u64>,
}

#[derive(Args, Debug, Clone)]
pub struct DaemonAcceptanceArgs {
    #[arg(long = "json")]
    pub json: bool,
}

#[derive(Args, Debug, Clone)]
pub struct DaemonPauseArgs {}

#[derive(Args, Debug, Clone)]
pub struct DaemonResumeArgs {}

#[derive(Args, Debug, Clone)]
pub struct DaemonRestoreArgs {
    #[arg(long = "dry-run")]
    pub dry_run: bool,
}
