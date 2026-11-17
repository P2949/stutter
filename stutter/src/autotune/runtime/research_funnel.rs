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
        funnel_driver::{FunnelDriver, RuleRegistry, production_inputs, selection_blocker},
        profile_applier::StudySettingsFile,
        profile_hook::{FileProfileHook, cyberpunk_running, production_spec},
        session_host::arm_prefix_paths,
        study_executor::ProfileHook,
        telemetry_observer::SharedTelemetry,
    },
    daemon::privilege::UnixSocketPrivilegedActionService,
};

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// The remembered list mappings plus what the arms' own settings files show now, saved back as an
/// immutable observation. Untrusted remembered mappings (unreadable or contradictory) are not used,
/// and an arm whose observation contradicts what is known is ignored, never allowed to overwrite it.
fn observed_mappings(
    study_dir: &std::path::Path,
    uid: u32,
    files: &[StudySettingsFile],
) -> crate::autotune::research::profile_mappings::Mappings {
    use crate::autotune::research::profile_mappings::Mappings;
    let Ok(dir) = File::open(study_dir) else {
        return Mappings::default();
    };
    let mut known = Mappings::load(&dir, uid).unwrap_or_else(|error| {
        log::warn!("remembered profile mappings are not usable: {error:#}");
        Mappings::default()
    });
    let mut fresh = Mappings::default();
    for file in files {
        if let Some(seen) = file.read().ok().and_then(|bytes| Mappings::observe(&bytes)) {
            match known.clone().merge(&seen) {
                Ok(()) => {
                    let _ = known.merge(&seen);
                    let _ = fresh.merge(&seen);
                }
                Err(reason) => log::warn!("a settings observation is ignored: {reason}"),
            }
        }
    }
    if let Err(error) = fresh.save(&dir) {
        log::warn!("profile mappings not remembered: {error:#}");
    }
    known
}

/// The graphics-profile hook over every arm's own `UserSettings.json`. If any of them cannot be
/// opened safely (a cold prefix has none yet), the hook says so instead of editing a subset.
fn profiles(study_dir: &std::path::Path, intent: &ResearchIntentV1) -> Box<dyn ProfileHook + Send> {
    // Each arm is opened on its own: the ones that can be opened are recovered and restored whatever
    // the others are, and the ones that cannot (no file in a cold prefix, an unsafe one) only refuse
    // a profile, never hide a snapshot beside them.
    let (mut files, mut unavailable) = (Vec::new(), Vec::new());
    for tool in &intent.tools {
        let settings = arm_prefix_paths(study_dir, tool.source).2;
        match StudySettingsFile::new(study_dir, settings.clone(), intent.uid) {
            Ok(file) => files.push(file),
            Err(_) => unavailable.push(settings),
        }
    }
    // The list mappings observed so far (remembered beside the study) and any the arms' current
    // settings now show. A profile is resolved against them; a value never observed keeps it blocked.
    let mappings = observed_mappings(study_dir, intent.uid, &files);
    Box::new(
        FileProfileHook::new(files, production_spec, cyberpunk_running)
            .with_unavailable(unavailable)
            .with_mappings(mappings),
    )
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
    let telemetry: SharedTelemetry = Default::default();
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
    .with_action_gate(gate.clone())
    .with_telemetry(telemetry.clone());
    // A treatment study no measured build can run is recorded as blocked before it starts.
    let screened = selection_blocker(campaign.try_clone()?, intent.clone(), rules.clone());
    // No CP-LONG work unit is installed in this build: a study on the long profile is recorded as
    // blocked, never run on the native benchmark and reported as a long session.
    let long_window = intent.long_attempt_seconds;
    let selection: crate::autotune::research::funnel_driver::SelectionBlocker =
        Box::new(move |study| {
            crate::autotune::research::long_session::long_unit_blocker(study, None, long_window)
                .or_else(|| screened(study))
        });
    Ok(FunnelDriver::new(
        campaign,
        intent,
        profiles(study_dir, intent),
        attempts,
        |_| None,
        rules,
    )?
    .with_selection_blocker(selection)
    .with_telemetry(telemetry)
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
