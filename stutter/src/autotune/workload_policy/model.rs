use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::autotune::{objective::ObjectiveKind, situation::SituationKind};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadPolicyRule {
    pub situation: SituationKind,
    pub allowed_families: BTreeSet<String>,
    pub allowed_objectives: BTreeSet<ObjectiveKind>,
    pub autonomous_families: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadPolicyMatrix {
    pub rules: Vec<WorkloadPolicyRule>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LintSeverity {
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadPolicyLintKind {
    EmptyAutonomousFamilies,
    DeniedFamilyIsAutonomous,
    HighRiskFamilyIsAutonomous,
    MediumRiskSystemWideDenied,
    ApplyLowRiskAutonomousFamilyTooRisky,
    ObjectiveWithoutCapableFamily,
    UnknownFamily,
    UnsupportedObjective,
    DuplicateRule,
}

impl WorkloadPolicyLintKind {
    pub fn reason_code(self) -> &'static str {
        match self {
            Self::EmptyAutonomousFamilies => "empty_autonomous_families",
            Self::DeniedFamilyIsAutonomous => "denied_family_is_autonomous",
            Self::HighRiskFamilyIsAutonomous => "high_risk_family_is_autonomous",
            Self::MediumRiskSystemWideDenied => "system_wide_family_is_autonomous",
            Self::ApplyLowRiskAutonomousFamilyTooRisky => {
                "apply_low_risk_autonomous_family_too_risky"
            }
            Self::ObjectiveWithoutCapableFamily => "objective_without_capable_family",
            Self::UnknownFamily => "unknown_family",
            Self::UnsupportedObjective => "unsupported_objective",
            Self::DuplicateRule => "duplicate_rule",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadPolicyLint {
    pub severity: LintSeverity,
    pub kind: WorkloadPolicyLintKind,
    pub reason_code: String,
    pub message: String,
    pub situation: Option<SituationKind>,
    pub family: Option<String>,
}

impl WorkloadPolicyLint {
    pub(super) fn warning(
        kind: WorkloadPolicyLintKind,
        message: String,
        situation: Option<SituationKind>,
        family: Option<&str>,
    ) -> Self {
        Self {
            severity: LintSeverity::Warning,
            kind,
            reason_code: kind.reason_code().to_owned(),
            message,
            situation,
            family: family.map(str::to_owned),
        }
    }

    pub(super) fn error(
        kind: WorkloadPolicyLintKind,
        message: String,
        situation: Option<SituationKind>,
        family: Option<&str>,
    ) -> Self {
        Self {
            severity: LintSeverity::Error,
            kind,
            reason_code: kind.reason_code().to_owned(),
            message,
            situation,
            family: family.map(str::to_owned),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonWorkloadPolicyConfig {
    pub rules: Vec<WorkloadPolicyRule>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct DaemonWorkloadPolicyConfigFile {
    #[serde(default)]
    pub rules: Vec<WorkloadPolicyRuleConfigFile>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct WorkloadPolicyRuleConfigFile {
    pub situation: String,
    #[serde(default)]
    pub allowed_families: Vec<String>,
    #[serde(default)]
    pub allowed_objectives: Vec<String>,
    #[serde(default)]
    pub autonomous_families: Vec<String>,
}
