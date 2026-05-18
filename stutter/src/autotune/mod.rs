//! Autotune planning, measurement, and controller orchestration.
//!
//! Owns:
//! - candidate generation, objective scoring, live experiment state, controller runtime setup,
//!   history/replay models, workload policy, protection, and autotune command dispatch.
//!
//! Does not own:
//! - raw sysfs/procfs mutation, remote API authorization, CLI argument parsing, recorder file
//!   formats, or daemon privilege transport.
//!
//! Allowed dependencies:
//! - actions for audited mutations, daemon policy types for safety decisions, config models,
//!   focus/process-tree inputs, recorder/report data models, and system observation helpers.
//!
//! Main entry points:
//! - `AutotuneCommandInput`, `autotune_command`, `runtime::run_autotune_controller_session`,
//!   `controller::AutotuneController`, planner/candidate modules, and emergency restore flows.
//!
//! Safety, mutation, and persistence invariants:
//! - live tuning must route mutation through action providers and daemon policy checks;
//! - experiments must keep enough journal/history state to recover or explain decisions;
//! - startup recovery and emergency restore paths must treat prior applied actions as durable
//!   state until they are verified restored;
//! - unsupported live modes must fail before constructing a mutating runtime configuration.

pub(crate) mod active_config;
pub(crate) mod apply;
pub(crate) mod apply_low_risk;
pub(crate) mod baseline;
pub(crate) mod candidate;
pub(crate) mod comparison;
pub(crate) mod conflicts;
pub(crate) mod controller_journal;
pub(crate) mod emergency_restore;
pub(crate) mod experiment;
pub(crate) mod generate_profiles;
pub(crate) mod gpu_focus;
pub(crate) mod history;
pub(crate) mod history_replay;
pub(crate) mod human_output;
pub(crate) mod kept;
pub(crate) mod live_experiment;
pub(crate) mod measurement;
pub(crate) mod profiles;
pub(crate) mod protection;
pub(crate) mod providers;
pub(crate) mod replay;
pub(crate) mod resolution;
pub(crate) mod shutdown;
pub(crate) mod situation;
pub(crate) mod status;
pub(crate) mod system_context;
pub(crate) mod target_selection;
pub(crate) mod washout;
pub(crate) mod workload_policy;

pub const DEFAULT_MIN_FOCUS_CONFIDENCE: f32 = 0.70;

pub(crate) mod candidate_memory;
pub(crate) mod context_segment;
pub(crate) mod controller;
pub(crate) mod decision;
pub(crate) mod decision_log;
pub(crate) mod objective;
pub(crate) mod observation;
pub(crate) mod observation_builder;
pub(crate) mod planner;
pub(crate) mod prometheus_metrics;
pub(crate) mod quality;
pub(crate) mod report_overlay;
pub(crate) mod rolling_window;
pub(crate) mod runtime;
pub(crate) mod simulation;
pub(crate) mod startup_recovery;
pub(crate) mod state;
pub(crate) mod tui_panel;

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
    pub allow_medium_risk: bool,
}

fn unsupported_live_autotune_mode_error(mode: impl std::fmt::Display) -> anyhow::Error {
    anyhow::anyhow!(
        "mode '{}' is not supported; use --mode observe, --mode suggest, --mode apply-low-risk, or --mode apply-medium-risk with --allow-medium-risk",
        mode
    )
}

fn ensure_supported_live_autotune_mode(
    mode: DaemonMode,
    allow_medium_risk: bool,
) -> anyhow::Result<()> {
    match mode {
        DaemonMode::Observe | DaemonMode::Suggest | DaemonMode::ApplyLowRisk => Ok(()),
        DaemonMode::ApplyMediumRisk if allow_medium_risk => Ok(()),
        DaemonMode::ApplyMediumRisk => {
            anyhow::bail!(
                "apply-medium-risk requires --allow-medium-risk and only applies reversible process-local candidates"
            )
        }
        DaemonMode::ApplyHighRisk => anyhow::bail!("high-risk apply is not implemented"),
    }
}

fn parse_supported_live_autotune_mode(
    raw_mode: &str,
    allow_medium_risk: bool,
) -> anyhow::Result<DaemonMode> {
    let mode = raw_mode
        .parse::<DaemonMode>()
        .map_err(|_| unsupported_live_autotune_mode_error(raw_mode))?;
    ensure_supported_live_autotune_mode(mode, allow_medium_risk)?;
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
    daemon_config.autotune.allow_medium_risk_apply = input.allow_medium_risk;

    if matches!(mode, DaemonMode::ApplyLowRisk | DaemonMode::ApplyMediumRisk) {
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
    let mode = parse_supported_live_autotune_mode(&input.mode, input.allow_medium_risk)?;

    if matches!(mode, DaemonMode::ApplyLowRisk | DaemonMode::ApplyMediumRisk) {
        if input.auto_focus {
            anyhow::bail!(
                "{mode} does not support --auto-focus yet; pass --tree-pid or --watch-process"
            );
        }

        if input.tree_pid.is_none() && input.watch_process.is_none() {
            anyhow::bail!("{mode} requires --tree-pid or --watch-process");
        }
    }

    if mode == DaemonMode::Suggest
        && let (Some(profiles_path), Some(tree_pid)) = (input.profiles.as_deref(), input.tree_pid)
    {
        let loaded_profiles = profiles::load_autotune_profiles(profiles_path)?;
        let candidates =
            candidate::generate_profile_candidates(&loaded_profiles.profiles, tree_pid, None);
        let dry_run_records = candidate::dry_run_candidates(&candidates);
        let plan_dir = candidate::default_candidate_plan_dir();
        let suggestions = candidate::suggestions_from_candidates_and_dry_run_records(
            &candidates,
            &dry_run_records,
            &plan_dir,
            Some(profiles_path),
            crate::actions::SafetyClass::ReversibleMediumRisk,
            "scheduler pressure detected on Game/WineServer classes",
        )?;
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

    let duration = if matches!(mode, DaemonMode::ApplyLowRisk | DaemonMode::ApplyMediumRisk) {
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

    #[test]
    fn autotune_child_modules_are_not_public_submodules() {
        let source = include_str!("mod.rs");

        let public_child_modules: Vec<&str> = source
            .lines()
            .map(str::trim_start)
            .filter(|line| line.starts_with("pub mod "))
            .collect();

        assert!(
            public_child_modules.is_empty(),
            "autotune child modules must stay crate-private and be exposed intentionally through api::autotune: {public_child_modules:?}"
        );
    }

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
            allow_medium_risk: false,
        }
    }

    #[test]
    fn runtime_config_builder_accepts_all_controller_modes() {
        for raw_mode in ["observe", "suggest", "apply-low-risk"] {
            let input = base_autotune_input(raw_mode);
            let mode =
                parse_supported_live_autotune_mode(raw_mode, input.allow_medium_risk).unwrap();
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
    fn medium_live_mode_requires_unlock_and_high_live_mode_is_rejected() {
        let err = parse_supported_live_autotune_mode("apply-medium-risk", false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("requires --allow-medium-risk"));

        let mode = parse_supported_live_autotune_mode("apply-medium-risk", true).unwrap();
        assert_eq!(mode, DaemonMode::ApplyMediumRisk);

        let err = parse_supported_live_autotune_mode("apply-high-risk", true)
            .unwrap_err()
            .to_string();
        assert!(err.contains("high-risk apply is not implemented"));
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
            allow_medium_risk: false,
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
            allow_medium_risk: false,
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
            allow_medium_risk: false,
        };

        let err = autotune_command(input).await.unwrap_err().to_string();
        assert!(err.contains("mode 'unknown-mode' is not supported"));
    }
}
