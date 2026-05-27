use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use super::*;
use crate::{
    actions::SafetyClass,
    daemon::policy::{ActionSource, DaemonMode},
};

pub const DEFAULT_PRIVILEGED_WORKER_SOCKET_READY_TIMEOUT_MS: u64 = 2_000;
pub const DEFAULT_PRIVILEGED_WORKER_SOCKET_READY_RETRY_MS: u64 = 50;
pub const DEFAULT_PRIVILEGED_WORKER_SHUTDOWN_POLL_MS: u64 = 25;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DaemonPreset {
    #[default]
    ObserveOnly,
    GamingLowRisk,
    GamingLaptopSafe,
    WorkstationLowRisk,
    DebugAggressive,
}

impl DaemonPreset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ObserveOnly => "observe-only",
            Self::GamingLowRisk => "gaming-low-risk",
            Self::GamingLaptopSafe => "gaming-laptop-safe",
            Self::WorkstationLowRisk => "workstation-low-risk",
            Self::DebugAggressive => "debug-aggressive",
        }
    }
}

impl fmt::Display for DaemonPreset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DaemonPreset {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "observe-only" => Ok(Self::ObserveOnly),
            "gaming-low-risk" => Ok(Self::GamingLowRisk),
            "gaming-laptop-safe" => Ok(Self::GamingLaptopSafe),
            "workstation-low-risk" => Ok(Self::WorkstationLowRisk),
            "debug-aggressive" => Ok(Self::DebugAggressive),
            other => anyhow::bail!(
                "invalid daemon preset {other:?}; valid values are observe-only, gaming-low-risk, gaming-laptop-safe, workstation-low-risk, debug-aggressive"
            ),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DaemonConfig {
    pub preset: DaemonPreset,
    pub mode: DaemonMode,
    pub source: ActionSource,
    pub target: DaemonTargetConfig,
    pub safety: DaemonSafetyConfig,
    pub health: DaemonHealthConfig,
    pub retention: DaemonRetentionConfig,
    pub remote: DaemonRemoteConfig,
    pub autotune: DaemonAutotuneConfig,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self::from_preset(DaemonPreset::ObserveOnly, ActionSource::Cli)
    }
}

impl DaemonConfig {
    pub fn from_preset(preset: DaemonPreset, source: ActionSource) -> Self {
        let mut config = Self {
            preset,
            mode: DaemonMode::Observe,
            source,
            target: DaemonTargetConfig::default(),
            safety: DaemonSafetyConfig::default(),
            health: DaemonHealthConfig::default(),
            retention: DaemonRetentionConfig::default(),
            remote: DaemonRemoteConfig::default(),
            autotune: DaemonAutotuneConfig::default(),
        };

        config.apply_preset(preset);
        config
    }

    pub fn apply_preset(&mut self, preset: DaemonPreset) {
        self.preset = preset;
        self.safety = DaemonSafetyConfig::default();
        self.health = DaemonHealthConfig::default();
        self.retention = DaemonRetentionConfig::default();
        self.remote = DaemonRemoteConfig::default();
        self.autotune = DaemonAutotuneConfig::default();

        match preset {
            DaemonPreset::ObserveOnly => {
                self.mode = DaemonMode::Observe;
                self.safety.max_safety_class = SafetyClass::ObserveOnly;
                self.safety.allowed_action_classes = safety_classes_up_to(SafetyClass::ObserveOnly);
                self.safety.min_confidence = 0.0;
            }
            DaemonPreset::GamingLowRisk => {
                self.mode = DaemonMode::ApplyLowRisk;
                self.safety.max_safety_class = SafetyClass::ReversibleLowRisk;
                self.safety.allowed_action_classes =
                    safety_classes_up_to(SafetyClass::ReversibleLowRisk);
                self.safety
                    .enabled_action_families
                    .insert("cpu_affinity_profile".to_owned());
                self.safety.min_confidence = 0.85;
                self.autotune.candidate_window_seconds = 30;
                self.autotune.washout_seconds = 10;
                self.retention.max_history_events = 20_000;
            }
            DaemonPreset::GamingLaptopSafe => {
                self.mode = DaemonMode::ApplyLowRisk;
                self.safety.max_safety_class = SafetyClass::ReversibleLowRisk;
                self.safety.allowed_action_classes =
                    safety_classes_up_to(SafetyClass::ReversibleLowRisk);
                self.safety
                    .enabled_action_families
                    .insert("cpu_affinity_profile".to_owned());
                self.safety.min_confidence = 0.92;
                self.health.max_cpu_temp_celsius = 82;
                self.health.max_gpu_temp_celsius = 84;
                self.autotune.candidate_window_seconds = 45;
                self.autotune.washout_seconds = 15;
                self.retention.max_history_events = 12_000;
            }
            DaemonPreset::WorkstationLowRisk => {
                self.mode = DaemonMode::ApplyLowRisk;
                self.safety.max_safety_class = SafetyClass::ReversibleLowRisk;
                self.safety.allowed_action_classes =
                    safety_classes_up_to(SafetyClass::ReversibleLowRisk);
                self.safety
                    .enabled_action_families
                    .insert("cpu_affinity_profile".to_owned());
                self.safety.min_confidence = 0.88;
                self.autotune.candidate_window_seconds = 60;
                self.autotune.washout_seconds = 15;
                self.retention.max_history_events = 30_000;
            }
            DaemonPreset::DebugAggressive => {
                self.mode = DaemonMode::ApplyMediumRisk;
                self.safety.max_safety_class = SafetyClass::ReversibleMediumRisk;
                self.safety.allowed_action_classes =
                    safety_classes_up_to(SafetyClass::ReversibleMediumRisk);
                self.safety.enabled_action_families.extend(
                    ["cpu_affinity_profile", "nice", "ionice", "uclamp"].map(str::to_owned),
                );
                self.safety.min_confidence = 0.75;
                self.autotune.candidate_window_seconds = 20;
                self.autotune.washout_seconds = 5;
                self.retention.max_history_events = 50_000;
            }
        }
    }
}
