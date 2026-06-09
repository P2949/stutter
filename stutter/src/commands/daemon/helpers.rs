use std::path::Path;

use super::status::DaemonStatusOutput;
use crate::{
    config_file::{self, UserConfigFile},
    daemon::{
        config::{DaemonConfig, DaemonPreset},
        policy::{ActionSource, DaemonPolicy, DaemonPolicyBuildInput, build_daemon_policy},
        state::{DaemonState, DaemonStateSnapshotWriter, load_daemon_state},
        store::DaemonStateStore,
    },
};

pub fn build_policy_from_daemon_status(status: &DaemonStatusOutput) -> DaemonPolicy {
    build_policy_from_daemon_state_with_user_config_result(
        &status.state,
        status.state_loaded,
        config_file::load_user_config(),
    )
}

pub fn build_policy_from_daemon_state_with_user_config_result(
    state: &DaemonState,
    state_loaded: bool,
    user_config: anyhow::Result<Option<UserConfigFile>>,
) -> DaemonPolicy {
    let config = match user_config {
        Ok(user_config) => {
            build_daemon_config_from_state(state, state_loaded, user_config.as_ref())
                .unwrap_or_else(|err| {
                    log::warn!("daemon_policy_config_build_failed err={err:#}; using observe-only");
                    DaemonConfig::from_preset(DaemonPreset::ObserveOnly, ActionSource::Cli)
                })
        }
        Err(err) => {
            log::warn!("daemon_policy_config_load_failed err={err:#}; using observe-only");
            DaemonConfig::from_preset(DaemonPreset::ObserveOnly, ActionSource::Cli)
        }
    };

    build_daemon_policy(DaemonPolicyBuildInput {
        config: &config,
        remote_context: None,
    })
}

pub fn build_daemon_config_from_state(
    state: &DaemonState,
    state_loaded: bool,
    user_config: Option<&UserConfigFile>,
) -> anyhow::Result<DaemonConfig> {
    let mut config =
        config_file::daemon_config_from_user_config(user_config, None, ActionSource::Cli)?;

    if state_loaded {
        config.mode = state.mode;
    }

    apply_daemon_state_target_to_config(&mut config, state);
    Ok(config)
}

pub fn apply_daemon_state_target_to_config(config: &mut DaemonConfig, state: &DaemonState) {
    config.target.require_explicit_target = config.mode.supports_apply();
    if let Some(target) = state.active_target.as_ref() {
        if let Some(root_pid) = target.root_pid
            && !config.target.tree_pids.contains(&root_pid)
        {
            config.target.tree_pids.push(root_pid);
        }
        config.target.watch_process = target.comm.clone();
    }
}

pub fn load_recent_daemon_decisions(limit: usize) -> Vec<super::status::DaemonRecentDecision> {
    if limit == 0 {
        return Vec::new();
    }

    let path = crate::autotune::history::default_autotune_history_path();
    let Ok(events) = crate::autotune::history::read_autotune_history_events(&path) else {
        return Vec::new();
    };

    events
        .into_iter()
        .rev()
        .take(limit)
        .map(|event| super::status::DaemonRecentDecision {
            unix_nanos: event.unix_nanos,
            phase: format!("{:?}", event.phase),
            mode: format!("{:?}", event.mode),
            decision: event.decision.decision,
            candidate_name: event.decision.candidate_name,
            action_id: event.action_id,
            rollback_performed: event.rollback_performed,
            reason: event.reason.clone(),
        })
        .collect()
}

pub fn daemon_state_store_for_path(path: &Path) -> anyhow::Result<DaemonStateStore> {
    let state = if path.exists() {
        load_daemon_state(path)?
    } else {
        DaemonState::default()
    };

    Ok(DaemonStateStore::new(
        state,
        Some(DaemonStateSnapshotWriter::new(path)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::{policy::DaemonMode, state::DaemonState};

    #[test]
    fn daemon_explain_policy_uses_configured_safety_with_live_state() {
        let state = DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            active_target: Some(crate::daemon::state::DaemonTargetState {
                root_pid: Some(1234),
                active_targets: 1,
                comm: Some("game".to_owned()),
            }),
            ..Default::default()
        };

        let policy = build_policy_from_daemon_state_with_user_config_result(&state, true, Ok(None));

        assert_eq!(policy.mode, DaemonMode::ApplyLowRisk);
    }

    #[test]
    fn daemon_explain_policy_uses_configured_mode_when_state_is_missing() {
        let state = DaemonState::default();

        let policy =
            build_policy_from_daemon_state_with_user_config_result(&state, false, Ok(None));

        assert_eq!(policy.mode, DaemonMode::Observe);
        assert!(
            !policy
                .enabled_action_families
                .contains("cpu_affinity_profile")
        );
    }

    #[test]
    fn daemon_explain_policy_falls_back_to_observe_only_when_config_is_unreadable() {
        let state = DaemonState {
            mode: DaemonMode::ApplyLowRisk,
            ..Default::default()
        };

        let policy = build_policy_from_daemon_state_with_user_config_result(
            &state,
            true,
            Err(anyhow::anyhow!("config-unreadable")),
        );

        assert_eq!(policy.mode, DaemonMode::Observe);
        assert!(
            !policy
                .enabled_action_families
                .contains("cpu_affinity_profile")
        );
    }
}
