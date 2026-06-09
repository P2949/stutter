use super::*;

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonArgs {
    #[command(subcommand)]
    pub(super) command: DaemonCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub(super) enum DaemonCommand {
    Config(DaemonConfigArgs),
    Policy(DaemonPolicyArgs),
    #[command(name = "policy-lint")]
    PolicyLint(DaemonPolicyLintArgs),
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
    #[command(name = "resync-state")]
    ResyncState(DaemonResyncStateArgs),
    Restore(DaemonRestoreArgs),
    #[command(name = "emergency-restore")]
    EmergencyRestore(DaemonRestoreArgs),
    #[command(name = "rollback-drill")]
    RollbackDrill(DaemonRollbackDrillArgs),
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonPolicyArgs {
    #[command(subcommand)]
    pub(super) command: DaemonPolicyCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub(super) enum DaemonPolicyCommand {
    Explain(DaemonPolicyExplainArgs),
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonPolicyExplainArgs {
    #[arg(long = "json")]
    pub(super) json: bool,

    #[arg(long = "preset", value_name = "NAME")]
    pub(super) preset: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonPolicyLintArgs {
    #[arg(long = "json")]
    pub(super) json: bool,

    #[arg(long = "preset", value_name = "NAME")]
    pub(super) preset: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonProfilesArgs {
    #[command(subcommand)]
    pub(super) command: DaemonProfilesCommand,
}

#[derive(Subcommand, Debug, Clone)]
pub(super) enum DaemonProfilesCommand {
    List(DaemonProfilesListArgs),
    Forget(DaemonProfilesForgetArgs),
    Explain(DaemonProfilesExplainArgs),
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonProfilesListArgs {
    #[arg(long = "json")]
    pub(super) json: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonProfilesForgetArgs {
    #[arg(long = "workload-hash", value_name = "HASH")]
    pub(super) workload_identity_hash: Option<String>,

    #[arg(long = "candidate", value_name = "NAME_OR_ACTION_ID")]
    pub(super) candidate: Option<String>,

    #[arg(long = "all")]
    pub(super) all: bool,

    #[arg(long = "dry-run")]
    pub(super) dry_run: bool,

    #[arg(long = "json")]
    pub(super) json: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonProfilesExplainArgs {
    #[arg(long = "workload-hash", value_name = "HASH")]
    pub(super) workload_identity_hash: Option<String>,

    #[arg(long = "json")]
    pub(super) json: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonResyncStateArgs {
    #[arg(long = "dry-run")]
    pub(super) dry_run: bool,

    #[arg(long = "json")]
    pub(super) json: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonExplainArgs {
    #[arg(long = "json")]
    pub(super) json: bool,

    #[arg(long = "explain-last", default_value_t = 10)]
    pub(super) explain_last: usize,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonWhyNotOptimizeArgs {
    #[arg(long = "json")]
    pub(super) json: bool,

    #[arg(long = "explain-last", default_value_t = 10)]
    pub(super) explain_last: usize,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonWhatChangedArgs {
    #[arg(long = "json")]
    pub(super) json: bool,

    #[arg(long = "explain-last", default_value_t = 10)]
    pub(super) explain_last: usize,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonConfigArgs {
    #[arg(long = "explain")]
    pub(super) explain: bool,

    #[arg(long = "json", requires = "explain")]
    pub(super) json: bool,

    #[arg(long = "preset", value_name = "NAME")]
    pub(super) preset: Option<String>,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonStatusArgs {
    #[arg(long = "json")]
    pub(super) json: bool,

    #[arg(
        long,
        value_name = "N",
        num_args = 0..=1,
        default_missing_value = "10",
        value_parser = clap::value_parser!(usize)
    )]
    pub(super) explain_last: Option<usize>,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonWatchArgs {
    #[arg(long = "interval-ms", default_value_t = 1_000)]
    pub(super) interval_ms: u64,

    #[arg(long = "iterations", value_name = "N")]
    pub(super) iterations: Option<u64>,

    #[arg(long = "verbose")]
    pub(super) verbose: bool,

    #[arg(long = "explain-last", default_value_t = 10)]
    pub(super) explain_last: usize,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonDoctorArgs {
    #[arg(long = "json")]
    pub(super) json: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonResetStateArgs {
    #[arg(long = "dry-run")]
    pub(super) dry_run: bool,

    #[arg(long = "json")]
    pub(super) json: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonBenchOverheadArgs {
    #[arg(long = "json")]
    pub(super) json: bool,

    #[arg(long = "duration-ms", default_value_t = 1_000)]
    pub(super) duration_ms: u64,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonSoakArgs {
    #[arg(long = "json")]
    pub(super) json: bool,

    #[arg(long = "duration-seconds", default_value_t = 60)]
    pub(super) duration_seconds: u64,

    #[arg(long = "tick-ms", default_value_t = 1_000)]
    pub(super) tick_ms: u64,

    #[arg(long = "profile", default_value = "observe-only")]
    pub(super) profile: String,

    #[arg(long = "max-disk-growth-bytes")]
    pub(super) max_disk_growth_bytes: Option<u64>,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonAcceptanceArgs {
    #[arg(long = "json")]
    pub(super) json: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonPauseArgs {}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonResumeArgs {}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonRestoreArgs {
    #[arg(long = "dry-run")]
    pub(super) dry_run: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct DaemonRollbackDrillArgs {
    #[arg(long = "dry-run")]
    pub(super) dry_run: bool,

    #[arg(long = "json")]
    pub(super) json: bool,
}

#[cfg(test)]
#[path = "tests/daemon.rs"]
mod tests;
