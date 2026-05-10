pub mod apply_low_risk;
pub mod baseline;
pub mod candidate;
pub mod comparison;
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
pub mod replay;
pub mod resolution;
pub mod shutdown;
pub mod status;
pub mod washout;

#[cfg(feature = "autotune-controller")]
pub mod candidate_memory;
#[cfg(feature = "autotune-controller")]
pub mod context_segment;
#[cfg(feature = "autotune-controller")]
pub mod controller;
#[cfg(feature = "autotune-controller")]
pub mod decision;
pub mod decision_log;
#[cfg(feature = "autotune-controller")]
pub mod observation;
#[cfg(feature = "autotune-controller")]
pub mod prometheus_metrics;
#[cfg(feature = "autotune-controller")]
pub mod quality;
#[cfg(feature = "autotune-controller")]
pub mod report_overlay;
#[cfg(feature = "autotune-controller")]
pub mod rolling_window;
#[cfg(feature = "autotune-controller")]
pub mod runtime;
#[cfg(feature = "autotune-controller")]
pub mod startup_recovery;
pub mod state;
pub mod tui_panel;

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use serde::Serialize;
#[cfg(not(feature = "autotune-controller"))]
use tokio::sync::{mpsc, oneshot};

use crate::{
    autotune::human_output::{
        HumanAutotuneMode, HumanControllerPhase, HumanDecisionKind, HumanDecisionWindow,
        HumanSituationKind, print_human_decision_window,
    },
    cli::Config,
    session_events::MonitorEvent,
};

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct AutotuneCommandInput {
    pub config: Option<PathBuf>,
    pub watch_process: Option<String>,
    pub tree_pid: Option<u32>,
    pub profiles: Option<PathBuf>,
    pub mode: String,
    pub decision_log: Option<PathBuf>,
    pub duration_seconds: Option<u64>,
    pub summary_ms: u64,
    pub preset: String,
    pub hwmon: bool,
    pub mangohud_log: Option<PathBuf>,
    pub auto_focus: bool,
    pub focus_source: crate::cli::FocusSource,
    pub foreground_window: bool,
    pub foreground_source: crate::cli::ForegroundSourceArg,
    pub foreground_poll_ms: u64,
    pub foreground_max_stale_ms: u64,
    pub allow_system_wide_actions: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct AutotuneDecisionLogEntry {
    pub unix_nanos: u128,
    pub mode: String,
    pub event_kind: String,
    pub decision: String,
    pub reason: String,
    pub interval_records: usize,
}

#[allow(dead_code)]
#[derive(Debug, Default)]
struct ObservePolicyStub {
    mode: String,
    watch_process: Option<String>,
    decision_log: Option<PathBuf>,
    interval_events: usize,
    interval_records: usize,
}

#[allow(dead_code)]
impl ObservePolicyStub {
    fn new(mode: String, watch_process: Option<String>, decision_log: Option<PathBuf>) -> Self {
        Self {
            mode,
            watch_process,
            decision_log,
            interval_events: 0,
            interval_records: 0,
        }
    }

    fn on_event(&mut self, event: MonitorEvent) -> anyhow::Result<()> {
        match event {
            MonitorEvent::Interval { records, .. } => {
                self.interval_events += 1;
                self.interval_records += records.len();
                self.write_decision(
                    "interval",
                    "noop",
                    "observe/suggest mode does not apply actions",
                    records.len(),
                )?;
            }
            MonitorEvent::DataQualityWarning { message } => {
                self.write_decision("data_quality_warning", "noop", &message, 0)?;
            }
            MonitorEvent::Finished { reason } => {
                self.write_decision("finished", "noop", &reason, 0)?;
            }
            other => {
                self.write_decision(other.kind(), "noop", "event observed; no action applied", 0)?;
            }
        }
        Ok(())
    }

    fn write_decision(
        &self,
        event_kind: &str,
        decision_str: &str,
        reason: &str,
        interval_records: usize,
    ) -> anyhow::Result<()> {
        let entry = AutotuneDecisionLogEntry {
            unix_nanos: crate::audit::unix_nanos_now(),
            mode: self.mode.clone(),
            event_kind: event_kind.to_owned(),
            decision: decision_str.to_owned(),
            reason: reason.to_owned(),
            interval_records,
        };

        if let Some(path) = &self.decision_log {
            if let Some(parent) = path.parent()
                && !parent.as_os_str().is_empty()
            {
                fs::create_dir_all(parent)?;
            }

            let mut file = OpenOptions::new().create(true).append(true).open(path)?;
            serde_json::to_writer(&mut file, &entry)?;
            file.write_all(b"\n")?;
        }

        let window = HumanDecisionWindow {
            phase: HumanControllerPhase::Observing,
            mode: match self.mode.as_str() {
                "observe" => HumanAutotuneMode::Observe,
                "suggest" => HumanAutotuneMode::Suggest,
                _ => HumanAutotuneMode::Observe,
            },
            target: self.watch_process.clone().unwrap_or_else(|| "-".to_owned()),
            score_total: 0,
            situation: HumanSituationKind::Unknown,
            decision: match decision_str {
                "noop" => HumanDecisionKind::Noop,
                _ => HumanDecisionKind::Noop,
            },
            reason: reason.to_owned(),
        };
        print_human_decision_window(&window);

        Ok(())
    }
}

pub async fn autotune_command(input: AutotuneCommandInput) -> anyhow::Result<()> {
    if input.mode == "suggest"
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
            crate::actions::SafetyClass::ReversibleLowRisk,
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

    match input.mode.as_str() {
        "observe" | "suggest" => {}
        "apply-low-risk" => {
            let outcome = apply_low_risk::apply_low_risk_command(&input).await?;
            println!(
                "apply-low-risk complete: candidate={} affected_tasks={} safety={:?}",
                outcome.candidate_name, outcome.affected_tasks, outcome.safety_class
            );
            return Ok(());
        }
        _ => {
            anyhow::bail!(
                "mode '{}' is not supported; use --mode observe, --mode suggest, or --mode apply-low-risk",
                input.mode
            )
        }
    }

    let monitor_config = crate::cli::autotune_monitor_config(&input)?;

    #[cfg(feature = "autotune-controller")]
    {
        let runtime_config = match input.mode.as_str() {
            "observe" => runtime::AutotuneRuntimeConfig::observe(
                input.decision_log.clone(),
                input.tree_pid,
                input.watch_process.clone(),
            ),
            "suggest" => runtime::AutotuneRuntimeConfig::suggest(
                input.decision_log.clone(),
                input.tree_pid,
                input.watch_process.clone(),
            ),
            other => {
                anyhow::bail!(
                    "mode '{}' is not supported by the live autotune runtime",
                    other
                )
            }
        };

        let duration = input.duration_seconds.map(Duration::from_secs);
        let exit = runtime::run_autotune_controller_session(
            monitor_config,
            runtime_config,
            None,
            duration,
        )
        .await?;

        if let Some(last_decision) = exit.last_decision {
            println!(
                "autotune runtime finished reason=\"{}\" last_decision={} score_total={}",
                exit.reason, last_decision.decision, last_decision.score_total
            );
        } else {
            println!("autotune runtime finished reason=\"{}\"", exit.reason);
        }

        return Ok(());
    }

    #[cfg(not(feature = "autotune-controller"))]
    {
        let (event_tx, mut event_rx) = mpsc::channel::<MonitorEvent>(1024);
        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        let duration = input.duration_seconds.map(Duration::from_secs);
        let mut policy = ObservePolicyStub::new(
            input.mode.clone(),
            input.watch_process.clone(),
            input.decision_log.clone(),
        );

        let monitor_task = tokio::spawn(async move {
            crate::session::run_monitor(monitor_config, None, Some(event_tx), Some(stop_rx)).await
        });

        let timeout_task = duration.map(|duration| {
            tokio::spawn(async move {
                tokio::time::sleep(duration).await;
                let _ = stop_tx.send(());
            })
        });

        while let Some(event) = event_rx.recv().await {
            policy.on_event(event)?;
        }

        if let Some(timeout_task) = timeout_task {
            let _ = timeout_task.await;
        }

        let monitor_result = monitor_task.await?;
        monitor_result?;

        Ok(())
    }
}

#[allow(dead_code)]
pub fn make_monitor_config_for_tests(config: Arc<Config>) -> Arc<Config> {
    config
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_policy_writes_decision_jsonl() {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "stutter-autotune-decision-test-{}-{}.jsonl",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));

        let mut policy = ObservePolicyStub::new("observe".to_owned(), None, Some(path.clone()));
        policy
            .on_event(MonitorEvent::DataQualityWarning {
                message: "test warning".to_owned(),
            })
            .unwrap();

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"mode\":\"observe\""));
        assert!(text.contains("\"event_kind\":\"data_quality_warning\""));
        assert!(text.contains("\"decision\":\"noop\""));
        assert!(text.contains("test warning"));

        fs::remove_file(path).ok();
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
            summary_ms: 1000,
            preset: "diagnosis".to_owned(),
            hwmon: false,
            mangohud_log: None,
            auto_focus: false,
            focus_source: crate::cli::FocusSource::Hybrid,
            foreground_window: false,
            foreground_source: crate::cli::ForegroundSourceArg::Auto,
            foreground_poll_ms: 1000,
            foreground_max_stale_ms: 2500,
            allow_system_wide_actions: false,
        };

        let err = autotune_command(input).await.unwrap_err().to_string();
        assert_eq!(
            err,
            "apply-low-risk requires exactly one target selector; pass --tree-pid or --watch-process"
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
            summary_ms: 1000,
            preset: "diagnosis".to_owned(),
            hwmon: false,
            mangohud_log: None,
            auto_focus: false,
            focus_source: crate::cli::FocusSource::Hybrid,
            foreground_window: false,
            foreground_source: crate::cli::ForegroundSourceArg::Auto,
            foreground_poll_ms: 1000,
            foreground_max_stale_ms: 2500,
            allow_system_wide_actions: false,
        };

        let err = autotune_command(input).await.unwrap_err().to_string();
        assert!(err.contains("mode 'unknown-mode' is not supported"));
    }
}
