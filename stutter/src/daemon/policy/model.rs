//! Core daemon policy model types; this module owns stable policy DTO shape, not evaluation.

use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use super::PolicyRejection;
use crate::actions::{ActionId, SafetyClass};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DaemonMode {
    Observe,
    Suggest,
    ApplyLowRisk,
    ApplyMediumRisk,
    ApplyHighRisk,
}

impl DaemonMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Suggest => "suggest",
            Self::ApplyLowRisk => "apply-low-risk",
            Self::ApplyMediumRisk => "apply-medium-risk",
            Self::ApplyHighRisk => "apply-high-risk",
        }
    }

    pub fn supports_apply(self) -> bool {
        matches!(
            self,
            Self::ApplyLowRisk | Self::ApplyMediumRisk | Self::ApplyHighRisk
        )
    }
}

impl fmt::Display for DaemonMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DaemonMode {
    type Err = PolicyRejection;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "observe" => Ok(Self::Observe),
            "suggest" => Ok(Self::Suggest),
            "apply-low-risk" => Ok(Self::ApplyLowRisk),
            "apply-medium-risk" => Ok(Self::ApplyMediumRisk),
            "apply-high-risk" => Ok(Self::ApplyHighRisk),
            other => Err(PolicyRejection::UnsupportedMode {
                mode: other.to_owned(),
            }),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionEffectScope {
    ObserveOnly,
    LocalProcess,
    LocalProcessTree,
    UserStateFile,
    Cgroup,
    Irq,
    Sysfs,
    CpuPower,
    GpuPower,
    VmKnob,
    SystemWide,
}

impl ActionEffectScope {
    pub(crate) fn is_low_risk_apply_scope(self) -> bool {
        matches!(self, Self::LocalProcess | Self::LocalProcessTree)
    }

    pub(crate) fn is_explicit_target_scope(self) -> bool {
        matches!(
            self,
            Self::LocalProcess
                | Self::LocalProcessTree
                | Self::Cgroup
                | Self::Irq
                | Self::CpuPower
                | Self::GpuPower
                | Self::VmKnob
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RollbackRequirement {
    NotRequiredForDryRun,
    RequiredBeforeApply,
    BestEffortOnly,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionSource {
    Cli,
    AutotuneRuntime,
    RemoteAgent,
    Tune,
    ApplyProfileWatch,
    Test,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionDescriptor {
    pub action_id: ActionId,
    pub action_kind: String,
    pub safety_class: SafetyClass,
    pub effect_scope: ActionEffectScope,
    pub rollback: RollbackRequirement,
    pub persistent_effect: bool,
    pub touches_system_wide_state: bool,
    pub requires_explicit_target: bool,
    pub confidence: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyIntent {
    Observe,
    Suggest,
    DryRun,
    Apply,
    Verify,
    Rollback,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DaemonPolicyVerdict {
    Allow,
    Reject,
    Delay,
    RequireObserveOnly,
    RequireManualConfirmation,
}

impl DaemonPolicyVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Reject => "reject",
            Self::Delay => "delay",
            Self::RequireObserveOnly => "require_observe_only",
            Self::RequireManualConfirmation => "require_manual_confirmation",
        }
    }

    pub fn is_allowed(self) -> bool {
        matches!(self, Self::Allow)
    }
}
