pub mod active_config;
pub mod apply;
pub mod apply_low_risk;
pub mod baseline;
pub mod candidate;
pub mod comparison;
pub mod conflicts;
pub mod controller_journal;
pub mod emergency_restore;
pub mod experiment;
pub mod generate_profiles;
pub mod history;
pub mod history_replay;
pub mod human_output;
pub mod kept;
pub mod measurement;
pub mod profiles;
pub mod protection;
pub mod providers;
pub mod replay;
pub mod resolution;
pub mod shutdown;
pub mod situation;
pub mod status;
pub mod system_context;
pub mod washout;
pub mod workload_policy;

pub const DEFAULT_MIN_FOCUS_CONFIDENCE: f32 = 0.70;

pub mod candidate_memory;
pub mod context_segment;
pub mod controller;
pub mod decision;
pub mod decision_log;
pub mod objective;
pub mod observation;
pub mod planner;
pub mod prometheus_metrics;
pub mod quality;
pub mod report_overlay;
pub mod rolling_window;
pub mod runtime;
pub mod simulation;
pub mod startup_recovery;
pub mod state;
pub mod tui_panel;

use std::{path::PathBuf, time::Duration};

use crate::daemon::{ActionSource, DaemonMode};

#[derive(Debug, Clone)]
pub struct AutotuneCommandInput {
    pub config: Option<PathBuf>,
    pub watch_process: Option<String>,
    pub tree_pid: Option<u32>,
    pub profiles: Option<PathBuf>,
    pub mode: String,
    pub decision_log: Option<PathBuf>,
    pub duration_seconds: Option<u64>,
    pub washout_seconds: u64,
    pub washout_verify_interval_ms: u64,
    pub summary_ms: u64,
    pub preset: String,
    pub hwmon: bool,
    pub mangohud_log: Option<PathBuf>,
    pub auto_focus: bool,
    pub min_focus_confidence: f32,
    pub focus_source: crate::config::FocusSource,
    pub foreground_window: bool,
    pub foreground_source: crate::config::ForegroundSource,
    pub foreground_poll_ms: u64,
    pub foreground_max_stale_ms: u64,
    pub allow_system_wide_suggestions: bool,
}

fn unsupported_live_autotune_mode_error(mode: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!(
        "mode '{}' is not supported; use --mode observe, --mode suggest, or --mode apply-low-risk. apply-low-risk currently applies CPU-affinity candidates only",
        mode
    )
}

fn ensure_supported_live_autotune_mode(mode: DaemonMode) -> anyhow::Result<()> {
    match mode {
        DaemonMode::Observe | DaemonMode::Suggest | DaemonMode::ApplyLowRisk => Ok(()),
        DaemonMode::ApplyMediumRisk | DaemonMode::ApplyHighRisk => {
            Err(unsupported_live_autotune_mode_error(mode))
        }
    }
}

fn parse_supported_live_autotune_mode(raw_mode: &str) -> anyhow::Result<DaemonMode> {
    let mode = raw_mode
        .parse::<DaemonMode>()
        .map_err(|_| unsupported_live_autotune_mode_error(raw_mode))?;
    ensure_supported_live_autotune_mode(mode)?;
    Ok(mode)
}

fn runtime_config_for_command(
    input: &AutotuneCommandInput,
    mode: DaemonMode,
    profiles: Vec<crate::profiles::Profile>,
) -> anyhow::Result<runtime::AutotuneRuntimeConfig> {
    let mut daemon_config = runtime::daemon_config_for_runtime_mode(
        mode,
        ActionSource::AutotuneRuntime,
        input.tree_pid,
        input.watch_process.clone(),
    );

    daemon_config.safety.min_confidence = input.min_focus_confidence;
    daemon_config.autotune.washout_seconds = input.washout_seconds;
    daemon_config.safety.allow_system_wide_suggestions = input.allow_system_wide_suggestions;

    if mode == DaemonMode::ApplyLowRisk {
        daemon_config.autotune.candidate_window_seconds = input.duration_seconds.unwrap_or(30);
    }

    let config = runtime::AutotuneRuntimeConfig::from_daemon_config(
        daemon_config,
        input.decision_log.clone(),
    )
    .with_profiles(profiles)
    .with_washout(input.washout_seconds, input.washout_verify_interval_ms);

    Ok(config)
}

pub async fn autotune_command(input: AutotuneCommandInput) -> anyhow::Result<()> {
    let mode = parse_supported_live_autotune_mode(&input.mode)?;

    if mode == DaemonMode::ApplyLowRisk {
        if input.auto_focus {
            anyhow::bail!(
                "apply-low-risk does not support --auto-focus yet; pass --tree-pid or --watch-process"
            );
        }

        if input.tree_pid.is_none() && input.watch_process.is_none() {
            anyhow::bail!("apply-low-risk requires --tree-pid or --watch-process");
        }
    }

    if mode == DaemonMode::Suggest
        && let (Some(profiles_path), Some(tree_pid)) = (input.profiles.as_deref(), input.tree_pid)
    {
        let loaded_profiles = profiles::load_autotune_profiles(profiles_path)?;
        let candidates =
            candidate::generate_profile_candidates(&loaded_profiles.profiles, tree_pid, None);
        let dry_run_records = candidate::dry_run_candidates(&candidates);
        let suggestions = candidate::suggestions_from_dry_run_records(
            &dry_run_records,
            tree_pid,
            Some(profiles_path),
            crate::actions::SafetyClass::ReversibleMediumRisk,
            "scheduler pressure detected on Game/WineServer classes",
        );
        candidate::print_candidate_suggestions(&suggestions);
    }

    let loaded_profiles = match input.profiles.as_deref() {
        Some(path) => Some(profiles::load_autotune_profiles(path)?),
        None => None,
    };

    if let Some(loaded_profiles) = &loaded_profiles {
        println!(
            "autotune profiles={} count={} candidates={}",
            loaded_profiles.path.display(),
            loaded_profiles.len(),
            loaded_profiles.profile_names().join(",")
        );
    }

    let monitor_config = crate::cli::autotune_monitor_config(&input)?;

    let profile_list = loaded_profiles
        .as_ref()
        .map(|loaded| loaded.profiles.clone())
        .unwrap_or_default();

    let runtime_config = runtime_config_for_command(&input, mode, profile_list)?;

    let duration = if mode == DaemonMode::ApplyLowRisk {
        None
    } else {
        input.duration_seconds.map(Duration::from_secs)
    };
    let exit =
        runtime::run_autotune_controller_session(monitor_config, runtime_config, None, duration)
            .await?;

    if let Some(last_decision) = exit.last_decision {
        println!(
            "autotune runtime finished reason=\"{}\" last_decision={} score_total={}",
            exit.reason, last_decision.decision, last_decision.score_total
        );
    } else {
        println!("autotune runtime finished reason=\"{}\"", exit.reason);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_autotune_input(mode: &str) -> AutotuneCommandInput {
        AutotuneCommandInput {
            config: None,
            watch_process: None,
            tree_pid: Some(1234),
            profiles: None,
            mode: mode.to_owned(),
            decision_log: None,
            duration_seconds: Some(1),
            washout_seconds: washout::DEFAULT_WASHOUT_SECONDS,
            washout_verify_interval_ms: washout::DEFAULT_WASHOUT_VERIFY_INTERVAL_MS,
            summary_ms: 1000,
            preset: "diagnosis".to_owned(),
            hwmon: false,
            mangohud_log: None,
            auto_focus: false,
            min_focus_confidence: crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE,
            focus_source: crate::config::FocusSource::Hybrid,
            foreground_window: false,
            foreground_source: crate::config::ForegroundSource::Auto,
            foreground_poll_ms: 1000,
            foreground_max_stale_ms: 2500,
            allow_system_wide_suggestions: false,
        }
    }

    #[test]
    fn runtime_config_builder_accepts_all_controller_modes() {
        for raw_mode in ["observe", "suggest", "apply-low-risk"] {
            let input = base_autotune_input(raw_mode);
            let mode = parse_supported_live_autotune_mode(raw_mode).unwrap();
            runtime_config_for_command(&input, mode, Vec::new()).unwrap();
        }
    }

    #[test]
    fn verify_autotune_construction_applies_mode_and_target_to_daemon_config() {
        let mut input = base_autotune_input("apply-low-risk");
        input.tree_pid = Some(4444);
        input.watch_process = Some("Game.exe".to_owned());
        input.min_focus_confidence = 0.88;

        let config =
            runtime_config_for_command(&input, DaemonMode::ApplyLowRisk, Vec::new()).unwrap();

        assert_eq!(config.daemon_config.mode, DaemonMode::ApplyLowRisk);
        assert_eq!(config.daemon_config.target.tree_pids, vec![4444]);
        assert_eq!(
            config.daemon_config.target.watch_process.as_deref(),
            Some("Game.exe")
        );
        assert_eq!(config.daemon_config.safety.min_confidence, 0.88);
        assert_eq!(config.daemon_policy.mode, DaemonMode::ApplyLowRisk);
        assert_eq!(config.daemon_policy.min_confidence, 0.88);
    }

    #[test]
    fn unsupported_medium_and_high_live_modes_are_rejected_by_central_gate() {
        for raw_mode in ["apply-medium-risk", "apply-high-risk"] {
            let err = parse_supported_live_autotune_mode(raw_mode)
                .unwrap_err()
                .to_string();

            assert!(err.contains(&format!("mode '{raw_mode}' is not supported")));
            assert!(err.contains("--mode observe"));
            assert!(err.contains("--mode suggest"));
            assert!(err.contains("--mode apply-low-risk"));
        }
    }

    #[tokio::test]
    async fn apply_low_risk_requires_target() {
        let input = AutotuneCommandInput {
            config: None,
            watch_process: None,
            tree_pid: None,
            profiles: None,
            mode: "apply-low-risk".to_owned(),
            decision_log: None,
            duration_seconds: Some(1),
            washout_seconds: washout::DEFAULT_WASHOUT_SECONDS,
            washout_verify_interval_ms: washout::DEFAULT_WASHOUT_VERIFY_INTERVAL_MS,
            summary_ms: 1000,
            preset: "diagnosis".to_owned(),
            hwmon: false,
            mangohud_log: None,
            auto_focus: false,
            min_focus_confidence: crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE,
            focus_source: crate::config::FocusSource::Hybrid,
            foreground_window: false,
            foreground_source: crate::config::ForegroundSource::Auto,
            foreground_poll_ms: 1000,
            foreground_max_stale_ms: 2500,
            allow_system_wide_suggestions: false,
        };

        let err = autotune_command(input).await.unwrap_err().to_string();
        assert_eq!(err, "apply-low-risk requires --tree-pid or --watch-process");
    }

    #[tokio::test]
    async fn apply_low_risk_rejects_auto_focus_selector() {
        let input = AutotuneCommandInput {
            config: None,
            watch_process: None,
            tree_pid: None,
            profiles: None,
            mode: "apply-low-risk".to_owned(),
            decision_log: None,
            duration_seconds: Some(1),
            washout_seconds: washout::DEFAULT_WASHOUT_SECONDS,
            washout_verify_interval_ms: washout::DEFAULT_WASHOUT_VERIFY_INTERVAL_MS,
            summary_ms: 1000,
            preset: "diagnosis".to_owned(),
            hwmon: false,
            mangohud_log: None,
            auto_focus: true,
            min_focus_confidence: crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE,
            focus_source: crate::config::FocusSource::Hybrid,
            foreground_window: false,
            foreground_source: crate::config::ForegroundSource::Auto,
            foreground_poll_ms: 1000,
            foreground_max_stale_ms: 2500,
            allow_system_wide_suggestions: false,
        };

        let err = autotune_command(input).await.unwrap_err().to_string();
        assert_eq!(
            err,
            "apply-low-risk does not support --auto-focus yet; pass --tree-pid or --watch-process"
        );
    }

    #[tokio::test]
    async fn unknown_mode_is_rejected() {
        let input = AutotuneCommandInput {
            config: None,
            watch_process: None,
            tree_pid: None,
            profiles: None,
            mode: "unknown-mode".to_owned(),
            decision_log: None,
            duration_seconds: Some(1),
            washout_seconds: washout::DEFAULT_WASHOUT_SECONDS,
            washout_verify_interval_ms: washout::DEFAULT_WASHOUT_VERIFY_INTERVAL_MS,
            summary_ms: 1000,
            preset: "diagnosis".to_owned(),
            hwmon: false,
            mangohud_log: None,
            auto_focus: false,
            min_focus_confidence: crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE,
            focus_source: crate::config::FocusSource::Hybrid,
            foreground_window: false,
            foreground_source: crate::config::ForegroundSource::Auto,
            foreground_poll_ms: 1000,
            foreground_max_stale_ms: 2500,
            allow_system_wide_suggestions: false,
        };

        let err = autotune_command(input).await.unwrap_err().to_string();
        assert!(err.contains("mode 'unknown-mode' is not supported"));
    }
}
