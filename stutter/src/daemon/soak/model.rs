use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{actions::SafetyClass, daemon::policy::DaemonMode};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum DaemonSoakProfile {
    ObserveOnly,
    ApplyLowRiskFake,
}

impl DaemonSoakProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ObserveOnly => "observe-only",
            Self::ApplyLowRiskFake => "apply-low-risk-fake",
        }
    }
}

impl fmt::Display for DaemonSoakProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DaemonSoakProfile {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim() {
            "observe-only" | "observe" => Ok(Self::ObserveOnly),
            "apply-low-risk-fake" | "low-risk-fake" => Ok(Self::ApplyLowRiskFake),
            other => anyhow::bail!(
                "unknown soak profile {other:?}; expected observe-only or apply-low-risk-fake"
            ),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonSoakConfig {
    pub profile: DaemonSoakProfile,
    pub duration_seconds: u64,
    pub tick_millis: u64,
    pub budget: DaemonSoakBudget,
}

impl Default for DaemonSoakConfig {
    fn default() -> Self {
        Self {
            profile: DaemonSoakProfile::ObserveOnly,
            duration_seconds: 60,
            tick_millis: 1_000,
            budget: DaemonSoakBudget::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonSoakBudget {
    pub max_memory_growth_bytes: u64,
    pub max_file_descriptors: u64,
    pub max_disk_growth_bytes: u64,
    pub max_event_queue_len: u64,
    pub max_task_count: u64,
    pub max_history_bytes: u64,
    pub max_cpu_millis_per_second: u64,
    pub max_wakeups_per_second: u64,
    pub max_event_drops: u64,
}

impl Default for DaemonSoakBudget {
    fn default() -> Self {
        Self {
            max_memory_growth_bytes: 8 * 1024 * 1024,
            max_file_descriptors: 128,
            max_disk_growth_bytes: 32 * 1024 * 1024,
            max_event_queue_len: 4096,
            max_task_count: 2048,
            max_history_bytes: 16 * 1024 * 1024,
            max_cpu_millis_per_second: 25,
            max_wakeups_per_second: 30,
            max_event_drops: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonSoakReport {
    pub profile: DaemonSoakProfile,
    pub duration_seconds: u64,
    pub ticks: u64,
    pub passed: bool,
    pub metrics: DaemonSoakMetrics,
    pub failures: Vec<DaemonSoakFailure>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scenarios: Vec<DaemonSoakScenarioReport>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonSoakMetrics {
    pub scenario_count: u64,
    pub planner_decisions: u64,
    pub memory_growth_bytes: u64,
    pub file_descriptors: u64,
    pub disk_growth_bytes: u64,
    pub max_event_queue_len: u64,
    pub task_count: u64,
    pub history_bytes: u64,
    pub cpu_millis_per_second: u64,
    pub wakeups_per_second: u64,
    pub event_drops: u64,
    pub fake_actions_started: u64,
    pub fake_rollbacks: u64,
    pub max_active_experiments: u64,
    pub low_data_quality_ticks: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonSoakFailure {
    pub reason_code: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DaemonSoakScenarioReport {
    pub name: String,
    pub mode: DaemonMode,
    pub ticks: u64,
    pub decisions: Vec<String>,
    pub passed: bool,
    pub failures: Vec<DaemonSoakFailure>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SoakScenario {
    pub name: String,
    pub mode: DaemonMode,
    #[serde(default = "default_soak_candidate_safety_class")]
    pub candidate_safety_class: SafetyClass,
    pub ticks: Vec<SoakTick>,
    #[serde(default)]
    pub assertions: Vec<SoakAssertion>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SoakTick {
    FocusGame {
        #[serde(default = "default_focus_confidence")]
        confidence: f32,
    },
    FocusCleared {
        #[serde(default = "default_focus_clear_reason")]
        reason: String,
    },
    TargetPresent,
    TargetMissing,
    Interval {
        diagnostic_score_total: u64,
        samples: u64,
    },
    DroppedInterval {
        dropped_events: u64,
    },
    EvaluationTick {
        reason: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SoakAssertion {
    OneActiveExperimentMaximum,
    NoHighRiskAutonomousApply,
    NoApplyDuringLowDataQuality,
    NoProtectedTaskMutation,
    RollbackTokenBeforeApply,
    ShutdownRestoresActiveActions,
    CooldownRespected,
    FocusFlappingDoesNotCauseActionFlapping,
    HighRiskManualOnly,
}

pub fn default_focus_confidence() -> f32 {
    0.95
}

pub fn default_focus_clear_reason() -> String {
    "focus cleared".to_owned()
}

pub fn default_soak_candidate_safety_class() -> SafetyClass {
    SafetyClass::ReversibleLowRisk
}
