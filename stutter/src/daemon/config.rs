use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    actions::SafetyClass,
    daemon::policy::{ActionSource, DaemonMode},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DaemonConfig {
    pub mode: DaemonMode,
    pub source: ActionSource,
    pub target: DaemonTargetConfig,
    pub safety: DaemonSafetyConfig,
    pub retention: DaemonRetentionConfig,
    pub remote: DaemonRemoteConfig,
    pub autotune: DaemonAutotuneConfig,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            mode: DaemonMode::Observe,
            source: ActionSource::Cli,
            target: DaemonTargetConfig::default(),
            safety: DaemonSafetyConfig::default(),
            retention: DaemonRetentionConfig::default(),
            remote: DaemonRemoteConfig::default(),
            autotune: DaemonAutotuneConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonTargetConfig {
    pub target_pids: Vec<u32>,
    pub tree_pids: Vec<u32>,
    pub watch_process: Option<String>,
    pub require_explicit_target: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DaemonSafetyConfig {
    pub max_safety_class: SafetyClass,
    pub allowed_action_classes: BTreeSet<SafetyClass>,
    pub denied_action_families: BTreeSet<String>,
    pub allow_system_wide_actions: bool,
    pub allow_high_risk: bool,
    pub allow_persistent_effects: bool,
    pub min_confidence: f32,
}

impl Default for DaemonSafetyConfig {
    fn default() -> Self {
        let mut allowed_action_classes = BTreeSet::new();
        allowed_action_classes.insert(SafetyClass::ObserveOnly);

        Self {
            max_safety_class: SafetyClass::ObserveOnly,
            allowed_action_classes,
            denied_action_families: BTreeSet::new(),
            allow_system_wide_actions: false,
            allow_high_risk: false,
            allow_persistent_effects: false,
            min_confidence: 0.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonRetentionConfig {
    pub max_history_events: usize,
    pub max_state_snapshots: usize,
    pub retain_crash_diagnostics: bool,
}

impl Default for DaemonRetentionConfig {
    fn default() -> Self {
        Self {
            max_history_events: 10_000,
            max_state_snapshots: 16,
            retain_crash_diagnostics: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonRemoteConfig {
    pub allow_remote_apply: bool,
    pub require_auth_for_apply: bool,
    pub allow_non_loopback_apply: bool,
}

impl Default for DaemonRemoteConfig {
    fn default() -> Self {
        Self {
            allow_remote_apply: false,
            require_auth_for_apply: true,
            allow_non_loopback_apply: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonAutotuneConfig {
    pub candidate_window_seconds: u64,
    pub washout_seconds: u64,
    pub rollback_on_crash_recovery: bool,
}

impl Default for DaemonAutotuneConfig {
    fn default() -> Self {
        Self {
            candidate_window_seconds: 30,
            washout_seconds: 10,
            rollback_on_crash_recovery: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_config_default_serializes() {
        let config = DaemonConfig::default();

        let json = serde_json::to_string(&config).unwrap();

        assert!(json.contains("\"mode\":\"observe\""));
        assert!(json.contains("\"source\":\"cli\""));
        assert!(json.contains("\"retain_crash_diagnostics\":true"));
    }

    #[test]
    fn daemon_config_owns_user_intent_fields() {
        let mut config = DaemonConfig {
            mode: DaemonMode::ApplyLowRisk,
            source: ActionSource::RemoteAgent,
            ..DaemonConfig::default()
        };
        config.target.tree_pids.push(1234);
        config.target.require_explicit_target = true;
        config.safety.max_safety_class = SafetyClass::ReversibleLowRisk;
        config.safety.allow_system_wide_actions = false;
        config.retention.max_state_snapshots = 4;
        config.remote.allow_remote_apply = true;
        config.autotune.candidate_window_seconds = 60;

        assert_eq!(config.mode, DaemonMode::ApplyLowRisk);
        assert_eq!(config.source, ActionSource::RemoteAgent);
        assert_eq!(config.target.tree_pids, vec![1234]);
        assert!(config.target.require_explicit_target);
        assert_eq!(
            config.safety.max_safety_class,
            SafetyClass::ReversibleLowRisk
        );
        assert_eq!(config.retention.max_state_snapshots, 4);
        assert!(config.remote.allow_remote_apply);
        assert_eq!(config.autotune.candidate_window_seconds, 60);
    }
}
