//! The production child-study funnel, selected only when the administrator enables it. Both the
//! funnel switch and live launch must be on and the study directory configured; anything else
//! installs no builder, so every campaign keeps running the flat schedule exactly as before.

use std::fs::File;

use super::FunnelBuilder;
use crate::{
    autotune::research::{
        ProtonSource, ResearchIntentV1,
        action_gate::{ActionGate, SharedActionGate},
        driver::AttemptHost,
        driver_attempts::DriverStudyAttempts,
        funnel_driver::{FunnelDriver, RuleRegistry, production_inputs},
        profile_applier::StudySettingsFile,
        profile_hook::{FileProfileHook, UnavailableProfiles, cyberpunk_running, production_spec},
        session_host::arm_prefix_paths,
        study_executor::ProfileHook,
    },
    daemon::privilege::UnixSocketPrivilegedActionService,
};

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// The graphics-profile hook over every arm's own `UserSettings.json`. If any of them cannot be
/// opened safely (a cold prefix has none yet), the hook says so instead of editing a subset.
fn profiles(study_dir: &std::path::Path, intent: &ResearchIntentV1) -> Box<dyn ProfileHook + Send> {
    let files: Result<Vec<_>, _> = intent
        .tools
        .iter()
        .map(|tool| {
            let settings = arm_prefix_paths(study_dir, tool.source).2;
            StudySettingsFile::new(study_dir, settings, intent.uid)
        })
        .collect();
    match files {
        Ok(files) => Box::new(FileProfileHook::new(
            files,
            production_spec,
            cyberpunk_running,
        )),
        Err(_) => Box::new(UnavailableProfiles("SettingsFileUnavailable")),
    }
}

fn build(
    study_dir: &std::path::Path,
    worker_socket: Option<&std::path::Path>,
    intent: &ResearchIntentV1,
    campaign: &File,
    host: Box<dyn AttemptHost + Send>,
) -> anyhow::Result<FunnelDriver> {
    // The action child's gate: unarmed until an Action study arms it on the sighted game process.
    // Its restoration proof reads the worker's outstanding-token count; unknown stays unproven.
    let socket = worker_socket.map(std::path::Path::to_path_buf);
    let gate: SharedActionGate = std::sync::Arc::new(std::sync::Mutex::new(
        ActionGate::unarmed(Box::new(move || {
            let socket = socket.clone()?;
            UnixSocketPrivilegedActionService::new(socket)
                .outstanding_tokens()
                .ok()
                .and_then(|count| u32::try_from(count).ok())
        }))
        // A restoration is also compared with the target's own kernel state, read from /proc.
        .with_state_probe(Box::new(
            crate::autotune::research::action_gate::procfs_state,
        )),
    ));
    let rules = RuleRegistry::default();
    let results = study_dir.to_path_buf();
    // The driver rejects a clock that moves backward, so the clock is clamped to never do so.
    let mut last = 0u64;
    let attempts = DriverStudyAttempts::new(
        campaign.try_clone()?,
        intent.clone(),
        host,
        Box::new(move || {
            last = last.max(unix_now());
            last
        }),
        // The controller passes its own lease fence on every tick.
        Box::new(|| Ok(())),
        Box::new(move |arm: ProtonSource| arm_prefix_paths(&results, arm).1),
        production_inputs(campaign.try_clone()?, intent.clone(), rules.clone()),
    )
    .with_action_gate(gate.clone());
    Ok(FunnelDriver::new(
        campaign,
        intent,
        profiles(study_dir, intent),
        attempts,
        crate::autotune::research::child_study::treatment_design_blocker,
        rules,
    )?
    .with_action_gate(gate))
}

/// The funnel builder the resident session installs: `None` (the flat schedule) unless the
/// child-study funnel and live launch are both explicitly enabled and the study directory is set.
pub(in crate::autotune::runtime) fn select_funnel_builder(
    config: &crate::daemon::config::autotune::DaemonAutotuneConfig,
) -> Option<FunnelBuilder> {
    if !(config.research_child_funnel && config.research_live_launch) {
        return None;
    }
    let study_dir = config.research_study_dir.clone()?;
    let socket = config
        .privileged_worker_socket
        .clone()
        .or_else(|| crate::daemon::privilege::default_privileged_worker_socket_path().ok());
    Some(Box::new(move |intent, campaign, host| {
        build(&study_dir, socket.as_deref(), intent, campaign, host)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> crate::daemon::config::autotune::DaemonAutotuneConfig {
        crate::daemon::config::autotune::DaemonAutotuneConfig::default()
    }

    #[test]
    fn the_funnel_is_off_unless_both_switches_and_the_study_directory_are_set() {
        let mut config = config();
        assert!(select_funnel_builder(&config).is_none(), "off by default");
        config.research_study_dir = Some("/home/u/studies".into());
        config.research_child_funnel = true;
        assert!(
            select_funnel_builder(&config).is_none(),
            "needs live launch too"
        );
        config.research_child_funnel = false;
        config.research_live_launch = true;
        assert!(
            select_funnel_builder(&config).is_none(),
            "needs the funnel switch"
        );
        config.research_child_funnel = true;
        config.research_study_dir = None;
        assert!(
            select_funnel_builder(&config).is_none(),
            "needs the study directory"
        );
        config.research_study_dir = Some("/home/u/studies".into());
        assert!(select_funnel_builder(&config).is_some());
    }
}
