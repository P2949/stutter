use super::*;

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

    #[arg(
        long = "mode",
        default_value = "observe",
        help = "Autotune mode: observe, suggest, apply-low-risk, apply-medium-risk, or apply-high-risk. Live autotune supports observe, suggest, apply-low-risk, and apply-medium-risk when --allow-medium-risk is set; apply-medium-risk is limited to reversible process-local/cgroup candidates, and apply-high-risk is not implemented."
    )]
    pub mode: String,

    #[arg(long = "decision-log", value_name = "PATH")]
    pub decision_log: Option<PathBuf>,

    #[arg(long = "duration-seconds")]
    pub duration_seconds: Option<u64>,

    #[arg(
        long = "washout-seconds",
        default_value_t = crate::autotune::washout::DEFAULT_WASHOUT_SECONDS
    )]
    pub washout_seconds: u64,

    #[arg(
        long = "washout-verify-interval-ms",
        default_value_t = crate::autotune::washout::DEFAULT_WASHOUT_VERIFY_INTERVAL_MS
    )]
    pub washout_verify_interval_ms: u64,

    #[arg(long = "summary-ms", default_value_t = 1000)]
    pub summary_ms: u64,

    #[arg(long = "preset", default_value = "diagnosis")]
    pub preset: String,

    #[arg(long = "hwmon")]
    pub hwmon: bool,

    #[arg(long = "mangohud-log")]
    pub mangohud_log: Option<PathBuf>,

    #[arg(
        long = "auto-focus",
        help = "Allow autotune observe/suggest to classify the whole system and follow the selected focus group"
    )]
    pub auto_focus: bool,

    #[arg(
        long = "min-focus-confidence",
        default_value_t = crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE,
        help = "Minimum focus confidence required before live autotune can suggest or apply candidates"
    )]
    pub min_focus_confidence: f32,

    #[arg(
        long = "focus-source",
        value_enum,
        default_value_t = FocusSource::Hybrid,
        help = "Autotune focus source: heuristic, foreground, or hybrid"
    )]
    pub focus_source: FocusSource,

    #[arg(
        long = "foreground-window",
        help = "Collect foreground-window context for autotune focus classification"
    )]
    pub foreground_window: bool,

    #[arg(
        long = "foreground-source",
        value_enum,
        default_value_t = ForegroundSource::Auto,
        help = "Foreground-window provider for autotune focus: auto, sway, hyprland, x11"
    )]
    pub foreground_source: ForegroundSource,

    #[arg(long = "foreground-poll-ms", default_value_t = 1000)]
    pub foreground_poll_ms: u64,

    #[arg(long = "foreground-max-stale-ms", default_value_t = 2500)]
    pub foreground_max_stale_ms: u64,

    #[arg(
        long = "allow-system-wide-suggestions",
        help = "Allow autotune to suggest system-wide candidates when in suggest mode"
    )]
    pub allow_system_wide_suggestions: bool,

    #[arg(
        long = "allow-medium-risk",
        help = "Explicitly unlock live apply-medium-risk for reversible process-local candidates"
    )]
    pub allow_medium_risk: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub enum AutotuneCommand {
    #[command(name = "generate-profiles")]
    GenerateProfiles(AutotuneGenerateProfilesArgs),
    #[command(name = "apply-candidate")]
    ApplyCandidate(AutotuneApplyCandidateArgs),
    Replay(AutotuneReplayArgs),

    ReplayHistory(AutotuneReplayHistoryArgs),

    Restore(AutotuneRestoreArgs),
}

#[derive(Args, Debug, Clone)]
pub struct AutotuneApplyCandidateArgs {
    #[arg(long = "candidate-json", value_name = "FILE")]
    pub candidate_json: PathBuf,

    #[arg(long = "dry-run")]
    pub dry_run: bool,
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
pub struct AutotuneStatusArgs {
    #[arg(long = "json")]
    pub json: bool,
}

pub fn validate_autotune_mode(mode: &str, allow_medium_risk: bool) -> anyhow::Result<()> {
    match mode {
        "observe" | "suggest" | "apply-low-risk" => Ok(()),
        "apply-medium-risk" if allow_medium_risk => Ok(()),
        "apply-medium-risk" => anyhow::bail!(
            "apply-medium-risk requires --allow-medium-risk and only applies reversible process-local candidates"
        ),
        "apply-high-risk" => anyhow::bail!("high-risk apply is not implemented"),
        _ => anyhow::bail!(
            "unsupported autotune mode; use observe, suggest, apply-low-risk, or apply-medium-risk with --allow-medium-risk"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autotune_cli_parses_washout_flags() {
        let cli = Cli::try_parse_from([
            "stutter",
            "autotune",
            "--washout-seconds",
            "30",
            "--washout-verify-interval-ms",
            "2000",
        ])
        .unwrap();

        let Some(Command::Autotune(args)) = cli.command else {
            panic!("expected autotune command");
        };

        assert_eq!(args.washout_seconds, 30);
        assert_eq!(args.washout_verify_interval_ms, 2_000);
        assert_eq!(
            args.min_focus_confidence,
            crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE
        );
    }

    #[test]
    fn autotune_cli_parses_min_focus_confidence() {
        let cli =
            Cli::try_parse_from(["stutter", "autotune", "--min-focus-confidence", "0.42"]).unwrap();

        let Some(Command::Autotune(args)) = cli.command else {
            panic!("expected autotune command");
        };

        assert_eq!(args.min_focus_confidence, 0.42);
    }
}
