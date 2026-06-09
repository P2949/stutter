use super::*;

#[derive(Args, Debug, Clone)]
#[command(about = "Print a guided workflow for proving an advisor fix plan")]
pub(super) struct ProveFixArgs {
    #[arg(long = "plan", value_name = "PATH")]
    pub(super) plan: PathBuf,

    #[arg(long = "profiles", value_name = "FILE")]
    pub(super) profiles: PathBuf,

    #[arg(long = "tree-pid", value_name = "PID")]
    pub(super) tree_pid: u32,

    #[arg(long = "scenario", value_name = "NAME")]
    pub(super) scenario_name: Option<String>,

    #[arg(long = "workload-label", value_name = "LABEL")]
    pub(super) workload_label: Option<String>,

    #[arg(long = "route-label", value_name = "LABEL")]
    pub(super) route_label: Option<String>,

    #[arg(long = "duration", default_value_t = 180, value_name = "SECONDS")]
    pub(super) duration_seconds: u64,

    #[arg(long = "baseline-runs", value_name = "N")]
    pub(super) baseline_runs: Option<usize>,

    #[arg(long = "test-runs", value_name = "N")]
    pub(super) test_runs: Option<usize>,

    #[arg(
        long = "baseline-profile",
        default_value = "baseline-online",
        value_name = "NAME"
    )]
    pub(super) baseline_profile: String,

    #[arg(
        long = "html",
        default_value = "fix-validation.html",
        value_name = "PATH"
    )]
    pub(super) html: PathBuf,
}
