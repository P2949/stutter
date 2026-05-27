use serde::{Deserialize, Deserializer, Serialize};

use crate::{actions::SafetyClass, daemon::policy::DaemonMode};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AgentAutotuneLimits {
    pub max_active_controllers: usize,
    pub max_mode: DaemonMode,
    pub max_safety_class: SafetyClass,
    pub allow_high_risk: bool,
    pub max_candidate_window_seconds: u64,
    pub max_targets: usize,
    pub allow_system_wide_suggestions: bool,
    pub allow_system_wide_apply: bool,
}

impl Default for AgentAutotuneLimits {
    fn default() -> Self {
        Self {
            max_active_controllers: 1,
            max_mode: DaemonMode::ApplyLowRisk,
            max_safety_class: SafetyClass::ReversibleLowRisk,
            allow_high_risk: false,
            max_candidate_window_seconds: 120,
            max_targets: 1,
            allow_system_wide_suggestions: false,
            allow_system_wide_apply: false,
        }
    }
}

#[derive(Debug, Deserialize)]
struct AgentAutotuneLimitsCompat {
    #[serde(default = "default_max_active_controllers")]
    max_active_controllers: usize,
    #[serde(default)]
    max_mode: Option<DaemonMode>,
    #[serde(default)]
    max_safety_class: Option<String>,
    #[serde(default)]
    allow_high_risk: bool,
    #[serde(default = "default_max_candidate_window_seconds")]
    max_candidate_window_seconds: u64,
    #[serde(default = "default_max_targets")]
    max_targets: usize,
    #[serde(default)]
    allow_system_wide_suggestions: bool,
    #[serde(default)]
    allow_system_wide_apply: bool,
}

fn default_max_active_controllers() -> usize {
    1
}

fn default_max_candidate_window_seconds() -> u64 {
    120
}

fn default_max_targets() -> usize {
    1
}

pub fn parse_legacy_safety_class(value: Option<&str>) -> Result<SafetyClass, String> {
    match value {
        Some("ObserveOnly") | Some("observe_only") => Ok(SafetyClass::ObserveOnly),
        Some("ReversibleMediumRisk") | Some("reversible_medium_risk") => {
            Ok(SafetyClass::ReversibleMediumRisk)
        }
        Some("HighRisk") | Some("high_risk") => Ok(SafetyClass::HighRisk),
        Some("ReversibleLowRisk") | Some("reversible_low_risk") | None => {
            Ok(SafetyClass::ReversibleLowRisk)
        }
        Some(other) => Err(format!(
            "invalid safety class {:?}; valid values are ObserveOnly, ReversibleLowRisk, ReversibleMediumRisk, HighRisk",
            other
        )),
    }
}

pub fn mode_for_safety_class(safety_class: SafetyClass) -> DaemonMode {
    match safety_class {
        SafetyClass::ObserveOnly => DaemonMode::Suggest,
        SafetyClass::ReversibleLowRisk => DaemonMode::ApplyLowRisk,
        SafetyClass::ReversibleMediumRisk => DaemonMode::ApplyMediumRisk,
        SafetyClass::HighRisk => DaemonMode::ApplyHighRisk,
    }
}

impl<'de> Deserialize<'de> for AgentAutotuneLimits {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let compat = AgentAutotuneLimitsCompat::deserialize(deserializer)?;
        let max_safety_class = parse_legacy_safety_class(compat.max_safety_class.as_deref())
            .map_err(serde::de::Error::custom)?;
        let max_mode = compat
            .max_mode
            .unwrap_or_else(|| mode_for_safety_class(max_safety_class.clone()));

        Ok(Self {
            max_active_controllers: compat.max_active_controllers,
            max_mode,
            max_safety_class,
            allow_high_risk: compat.allow_high_risk,
            max_candidate_window_seconds: compat.max_candidate_window_seconds,
            max_targets: compat.max_targets,
            allow_system_wide_suggestions: compat.allow_system_wide_suggestions,
            allow_system_wide_apply: compat.allow_system_wide_apply,
        })
    }
}
