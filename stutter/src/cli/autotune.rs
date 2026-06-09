use std::fmt;

use clap::ValueEnum;

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub(super) enum LiveAutotuneMode {
    Observe,
    Suggest,
    ApplyLowRisk,
    ApplyMediumRisk,
}

impl LiveAutotuneMode {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Suggest => "suggest",
            Self::ApplyLowRisk => "apply-low-risk",
            Self::ApplyMediumRisk => "apply-medium-risk",
        }
    }

    pub(super) const fn as_daemon_mode(self) -> crate::daemon::policy::DaemonMode {
        match self {
            Self::Observe => crate::daemon::policy::DaemonMode::Observe,
            Self::Suggest => crate::daemon::policy::DaemonMode::Suggest,
            Self::ApplyLowRisk => crate::daemon::policy::DaemonMode::ApplyLowRisk,
            Self::ApplyMediumRisk => crate::daemon::policy::DaemonMode::ApplyMediumRisk,
        }
    }
}

impl fmt::Display for LiveAutotuneMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Args, Debug, Clone)]
pub(super) struct AutotuneArgs {
    #[command(subcommand)]
    pub(super) command: Option<AutotuneCommand>,

    #[arg(long = "config", value_name = "PATH")]
    pub(super) config: Option<PathBuf>,

    #[arg(long = "watch-process", value_name = "COMM")]
    pub(super) watch_process: Option<String>,

    #[arg(long = "tree-pid", value_name = "PID")]
    pub(super) tree_pid: Option<u32>,

    #[arg(long = "profiles", value_name = "FILE")]
    pub(super) profiles: Option<PathBuf>,

    #[arg(
        long = "mode",
        value_enum,
        default_value_t = LiveAutotuneMode::Observe,
        help = "Live autotune mode: observe, suggest, apply-low-risk, or apply-medium-risk. apply-medium-risk requires --allow-medium-risk and is limited to reversible process-local/cgroup candidates. High-risk apply is reserved internally and is not exposed by live autotune."
    )]
    pub(super) mode: LiveAutotuneMode,

    #[arg(long = "decision-log", value_name = "PATH")]
    pub(super) decision_log: Option<PathBuf>,

    #[arg(long = "duration-seconds")]
    pub(super) duration_seconds: Option<u64>,

    #[arg(
        long = "washout-seconds",
        default_value_t = crate::autotune::washout::DEFAULT_WASHOUT_SECONDS
    )]
    pub(super) washout_seconds: u64,

    #[arg(
        long = "washout-verify-interval-ms",
        default_value_t = crate::autotune::washout::DEFAULT_WASHOUT_VERIFY_INTERVAL_MS
    )]
    pub(super) washout_verify_interval_ms: u64,

    #[arg(long = "summary-ms", default_value_t = 1000)]
    pub(super) summary_ms: u64,

    #[arg(long = "preset", default_value = "diagnosis")]
    pub(super) preset: String,

    #[arg(long = "hwmon")]
    pub(super) hwmon: bool,

    #[arg(long = "mangohud-log")]
    pub(super) mangohud_log: Option<PathBuf>,

    #[arg(
        long = "auto-focus",
        help = "Allow autotune observe/suggest to classify the whole system and follow the selected focus group"
    )]
    pub(super) auto_focus: bool,

    #[arg(
        long = "min-focus-confidence",
        default_value_t = crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE,
        help = "Minimum focus confidence required before live autotune can suggest or apply candidates"
    )]
    pub(super) min_focus_confidence: f32,

    #[arg(
        long = "focus-source",
        value_enum,
        default_value_t = FocusSource::Hybrid,
        help = "Autotune focus source: heuristic, foreground, or hybrid"
    )]
    pub(super) focus_source: FocusSource,

    #[arg(
        long = "foreground-window",
        help = "Collect foreground-window context for autotune focus classification"
    )]
    pub(super) foreground_window: bool,

    #[arg(
        long = "foreground-source",
        value_enum,
        default_value_t = ForegroundSource::Auto,
        help = "Foreground-window provider for autotune focus: auto, sway, hyprland, gnome, kde, x11"
    )]
    pub(super) foreground_source: ForegroundSource,

    #[arg(long = "foreground-poll-ms", default_value_t = 1000)]
    pub(super) foreground_poll_ms: u64,

    #[arg(long = "foreground-max-stale-ms", default_value_t = 2500)]
    pub(super) foreground_max_stale_ms: u64,

    #[arg(
        long = "allow-system-wide-suggestions",
        help = "Allow autotune to suggest system-wide candidates when in suggest mode"
    )]
    pub(super) allow_system_wide_suggestions: bool,

    #[arg(
        long = "allow-medium-risk",
        help = "Explicitly unlock live apply-medium-risk for reversible process-local candidates"
    )]
    pub(super) allow_medium_risk: bool,

    #[arg(
        long = "high-risk-dry-run",
        help = "In suggest mode, run dry-run diagnostics for manual-only high-risk candidates without enabling apply"
    )]
    pub(super) high_risk_dry_run: bool,

    #[arg(
        long = "dry-run-all-safe",
        help = "In suggest mode, run safe candidate dry-runs, write candidate plan files, and never apply changes"
    )]
    pub(super) dry_run_all_safe: bool,
}

#[derive(Subcommand, Debug, Clone)]
pub(super) enum AutotuneCommand {
    #[command(name = "generate-profiles")]
    GenerateProfiles(AutotuneGenerateProfilesArgs),
    #[command(name = "apply-candidate")]
    ApplyCandidate(AutotuneApplyCandidateArgs),
    Replay(AutotuneReplayArgs),

    ReplayHistory(AutotuneReplayHistoryArgs),

    Restore(AutotuneRestoreArgs),
}

#[derive(Args, Debug, Clone)]
pub(super) struct AutotuneApplyCandidateArgs {
    #[arg(long = "candidate-json", value_name = "FILE")]
    pub(super) candidate_json: PathBuf,

    #[arg(long = "dry-run")]
    pub(super) dry_run: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct AutotuneGenerateProfilesArgs {
    #[arg(long = "watch-process", value_name = "COMM")]
    pub(super) watch_process: Option<String>,

    #[arg(long = "out", value_name = "PATH_OR_-")]
    pub(super) out: PathBuf,

    #[arg(long = "allow-cpus", value_name = "CPU_LIST")]
    pub(super) allow_cpus: Option<String>,

    #[arg(long = "deny-cpus", value_name = "CPU_LIST")]
    pub(super) deny_cpus: Option<String>,

    #[arg(long = "min-render-cpus", default_value_t = 1)]
    pub(super) min_render_cpus: usize,

    #[arg(long = "min-game-cpus", default_value_t = 1)]
    pub(super) min_game_cpus: usize,

    #[arg(long = "min-compositor-cpus", default_value_t = 1)]
    pub(super) min_compositor_cpus: usize,

    #[arg(long = "min-background-cpus", default_value_t = 2)]
    pub(super) min_background_cpus: usize,
}

#[derive(Args, Debug, Clone)]
pub(super) struct AutotuneRestoreArgs {
    #[arg(
        long = "journal",
        value_name = "PATH",
        help = "Path to autotune controller_journal.json; defaults to ~/.local/state/stutter/autotune/controller_journal.json"
    )]
    pub(super) journal: Option<PathBuf>,

    #[arg(
        long = "audit",
        value_name = "PATH",
        help = "Path to audit JSONL output; defaults to the normal stutter audit log"
    )]
    pub(super) audit: Option<PathBuf>,

    #[arg(
        long = "history",
        value_name = "PATH",
        help = "Path to autotune history JSONL output; defaults to the normal autotune history log"
    )]
    pub(super) history: Option<PathBuf>,

    #[arg(long = "dry-run")]
    pub(super) dry_run: bool,
}

#[derive(Args, Debug, Clone)]
pub(super) struct AutotuneReplayHistoryArgs {
    #[arg(value_name = "HISTORY_JSONL")]
    pub(super) history: PathBuf,
}

#[derive(Args, Debug, Clone)]
pub(super) struct AutotuneReplayArgs {
    #[arg(long = "run", value_name = "RUN_DIR")]
    pub(super) run: PathBuf,

    #[arg(long = "config", value_name = "AUTOTUNE_TOML")]
    pub(super) config: Option<PathBuf>,
}

#[derive(Args, Debug, Clone)]
pub(super) struct AutotuneStatusArgs {
    #[arg(long = "json")]
    pub(super) json: bool,
}

pub(super) fn validate_autotune_mode(
    mode: LiveAutotuneMode,
    allow_medium_risk: bool,
    dry_run_all_safe: bool,
) -> anyhow::Result<()> {
    if dry_run_all_safe && mode != LiveAutotuneMode::Suggest {
        anyhow::bail!("--dry-run-all-safe requires --mode suggest");
    }

    match mode {
        LiveAutotuneMode::Observe | LiveAutotuneMode::Suggest | LiveAutotuneMode::ApplyLowRisk => {
            Ok(())
        }
        LiveAutotuneMode::ApplyMediumRisk if allow_medium_risk => Ok(()),
        LiveAutotuneMode::ApplyMediumRisk => anyhow::bail!(
            "apply-medium-risk requires --allow-medium-risk and only applies reversible process-local candidates"
        ),
    }
}

#[cfg(test)]
#[path = "tests/autotune.rs"]
mod tests;
