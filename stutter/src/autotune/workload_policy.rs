use std::collections::BTreeSet;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::autotune::{
    candidate::CandidateAction, objective::ObjectiveKind, situation::SituationKind,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadPolicyRule {
    pub situation: SituationKind,
    pub allowed_families: BTreeSet<String>,
    pub allowed_objectives: BTreeSet<ObjectiveKind>,
    pub autonomous_families: BTreeSet<String>,
}

impl WorkloadPolicyRule {
    pub fn allows_candidate(&self, candidate: &CandidateAction) -> bool {
        self.allowed_families
            .iter()
            .any(|family| family_matches(candidate.action_kind(), family))
    }

    pub fn allows_autonomous_candidate(&self, candidate: &CandidateAction) -> bool {
        self.autonomous_families
            .iter()
            .any(|family| family_matches(candidate.action_kind(), family))
    }

    pub fn allows_objective(&self, objective: ObjectiveKind) -> bool {
        self.allowed_objectives.is_empty() || self.allowed_objectives.contains(&objective)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadPolicyMatrix {
    pub rules: Vec<WorkloadPolicyRule>,
}

impl WorkloadPolicyMatrix {
    pub fn default_rules() -> Self {
        Self {
            rules: all_situations()
                .into_iter()
                .map(default_rule_for_situation)
                .collect(),
        }
    }

    pub fn rule_for(&self, situation: SituationKind) -> WorkloadPolicyRule {
        self.rules
            .iter()
            .find(|rule| rule.situation == situation)
            .cloned()
            .unwrap_or_else(|| default_rule_for_situation(situation))
    }

    pub fn with_overrides(overrides: Vec<WorkloadPolicyRule>) -> anyhow::Result<Self> {
        let mut matrix = Self::default_rules();
        let mut seen = Vec::new();

        for override_rule in overrides {
            validate_workload_policy_rule(&override_rule)?;
            if seen.contains(&override_rule.situation) {
                anyhow::bail!(
                    "duplicate workload policy rule for situation {:?}",
                    override_rule.situation
                );
            }
            seen.push(override_rule.situation);

            if let Some(existing) = matrix
                .rules
                .iter_mut()
                .find(|rule| rule.situation == override_rule.situation)
            {
                *existing = override_rule;
            } else {
                matrix.rules.push(override_rule);
            }
        }

        Ok(matrix)
    }
}

pub fn workload_policy_for_situation(situation: SituationKind) -> WorkloadPolicyRule {
    WorkloadPolicyMatrix::default_rules().rule_for(situation)
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DaemonWorkloadPolicyConfig {
    pub rules: Vec<WorkloadPolicyRule>,
}

impl DaemonWorkloadPolicyConfig {
    pub fn resolved_matrix(&self) -> anyhow::Result<WorkloadPolicyMatrix> {
        WorkloadPolicyMatrix::with_overrides(self.rules.clone())
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
pub struct WorkloadPolicyRuleConfigFile {
    pub situation: String,
    pub allowed_families: Vec<String>,
    pub allowed_objectives: Vec<String>,
    pub autonomous_families: Vec<String>,
}

impl WorkloadPolicyRuleConfigFile {
    pub fn into_rule(self) -> anyhow::Result<WorkloadPolicyRule> {
        let situation = parse_situation_kind(&self.situation)
            .with_context(|| format!("invalid workload policy situation {:?}", self.situation))?;
        let allowed_families = parse_family_set("allowed_families", &self.allowed_families)?;
        let autonomous_families =
            parse_family_set("autonomous_families", &self.autonomous_families)?;
        let allowed_objectives = self
            .allowed_objectives
            .iter()
            .map(|objective| {
                parse_objective_kind(objective)
                    .with_context(|| format!("invalid workload policy objective {objective:?}"))
            })
            .collect::<anyhow::Result<BTreeSet<_>>>()?;

        let rule = WorkloadPolicyRule {
            situation,
            allowed_families,
            allowed_objectives,
            autonomous_families,
        };
        validate_workload_policy_rule(&rule)?;
        Ok(rule)
    }
}

pub fn parse_workload_policy_rule_configs(
    configs: &[WorkloadPolicyRuleConfigFile],
) -> anyhow::Result<DaemonWorkloadPolicyConfig> {
    let mut rules = Vec::new();
    let mut seen = Vec::new();

    for config in configs {
        let rule = config.clone().into_rule()?;
        if seen.contains(&rule.situation) {
            anyhow::bail!(
                "duplicate workload policy rule for situation {:?}",
                rule.situation
            );
        }
        seen.push(rule.situation);
        rules.push(rule);
    }

    Ok(DaemonWorkloadPolicyConfig { rules })
}

pub fn validate_workload_policy_rule(rule: &WorkloadPolicyRule) -> anyhow::Result<()> {
    for family in rule
        .allowed_families
        .iter()
        .chain(rule.autonomous_families.iter())
    {
        validate_action_family_name(family)?;
    }

    for family in &rule.autonomous_families {
        if !rule.allowed_families.contains(family) {
            anyhow::bail!(
                "autonomous workload policy family {family:?} must also be listed in allowed_families"
            );
        }
    }

    Ok(())
}

pub fn validate_action_family_name(family: &str) -> anyhow::Result<()> {
    if !known_action_family(family) {
        anyhow::bail!(
            "unknown workload policy action family {family:?}; valid families are {}",
            known_action_families().join(", ")
        );
    }
    Ok(())
}

pub fn parse_situation_kind(value: &str) -> anyhow::Result<SituationKind> {
    let normalized = normalize_name(value);
    all_situations()
        .into_iter()
        .find(|situation| normalize_name(&format!("{situation:?}")) == normalized)
        .ok_or_else(|| anyhow::anyhow!("unknown situation {value:?}"))
}

pub fn parse_objective_kind(value: &str) -> anyhow::Result<ObjectiveKind> {
    let normalized = normalize_name(value);
    all_objectives()
        .into_iter()
        .find(|objective| normalize_name(&format!("{objective:?}")) == normalized)
        .ok_or_else(|| anyhow::anyhow!("unknown objective {value:?}"))
}

fn default_rule_for_situation(situation: SituationKind) -> WorkloadPolicyRule {
    use ObjectiveKind::*;
    use SituationKind::*;

    match situation {
        GameFocused | GameCpuSchedulerPressure | GameGpuBound => rule(
            situation,
            [
                "cpu_affinity_profile",
                "uclamp",
                "gpu_power",
                "irq_affinity",
                "cpu_power",
                "nice",
                "ionice",
            ],
            [
                StutterScore,
                GameFramePacing,
                GameRunnableLatency,
                DesktopInteractivity,
                IoLatency,
                IrqOverlapReduction,
                ThermalRecovery,
            ],
            ["cpu_affinity_profile"],
        ),
        BrowserFocused | BrowserCpuPressure | BrowserGpuVideo | BrowserIoPressure => rule(
            situation,
            [
                "nice",
                "ionice",
                "uclamp",
                "cpu_affinity_profile",
                "gpu_power",
            ],
            [
                GameFramePacing,
                BrowserInteractivity,
                DesktopInteractivity,
                IoLatency,
                StutterScore,
                ThermalRecovery,
            ],
            [],
        ),
        CompileLoad | CompileCpuBound | CompileLinkerPressure => rule(
            situation,
            [
                "cpu_affinity_profile",
                "cgroup_placement",
                "uclamp",
                "nice",
                "ionice",
            ],
            [
                CompileThroughputWithForegroundProtection,
                DesktopInteractivity,
                IoLatency,
                StutterScore,
            ],
            [],
        ),
        Recording => rule(
            situation,
            ["nice", "ionice", "uclamp"],
            [DesktopInteractivity, IoLatency, StutterScore],
            [],
        ),
        MediaPlayback => rule(
            situation,
            ["nice", "ionice", "uclamp"],
            [DesktopInteractivity, IoLatency, StutterScore],
            [],
        ),
        VirtualMachineLoad => rule(
            situation,
            ["cgroup_placement", "uclamp", "cpu_affinity_profile"],
            [DesktopInteractivity, StutterScore],
            [],
        ),
        CompositorPressure => rule(
            situation,
            ["uclamp", "nice", "ionice", "cpu_affinity_profile"],
            [DesktopInteractivity, StutterScore],
            [],
        ),
        CpuPressure => rule(
            situation,
            ["cpu_affinity_profile", "uclamp", "nice"],
            [DesktopInteractivity, StutterScore],
            [],
        ),
        IoPressure => rule(
            situation,
            ["ionice", "vm_knob"],
            [IoLatency, StutterScore],
            [],
        ),
        IrqPressure => rule(
            situation,
            ["irq_affinity"],
            [IrqOverlapReduction, StutterScore],
            [],
        ),
        ThermalOrPowerLimit => rule(
            situation,
            ["cpu_power", "gpu_power"],
            [ThermalRecovery, StutterScore],
            [],
        ),
        Idle => rule(situation, ["cpu_power", "gpu_power"], [ThermalRecovery], []),
        Unknown => rule(situation, [], [], []),
    }
}

fn rule<const F: usize, const O: usize, const A: usize>(
    situation: SituationKind,
    families: [&str; F],
    objectives: [ObjectiveKind; O],
    autonomous: [&str; A],
) -> WorkloadPolicyRule {
    WorkloadPolicyRule {
        situation,
        allowed_families: families.into_iter().map(str::to_owned).collect(),
        allowed_objectives: objectives.into_iter().collect(),
        autonomous_families: autonomous.into_iter().map(str::to_owned).collect(),
    }
}

fn family_matches(action_kind: &str, family: &str) -> bool {
    action_kind == family
        || action_kind.strip_prefix(family).is_some_and(|suffix| {
            matches!(
                suffix.as_bytes().first(),
                Some(b':') | Some(b'-') | Some(b'_')
            )
        })
}

fn parse_family_set(field: &str, values: &[String]) -> anyhow::Result<BTreeSet<String>> {
    let mut families = BTreeSet::new();

    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            anyhow::bail!("{field} contains an empty action family");
        }
        if trimmed != value {
            anyhow::bail!("{field} entries must not contain leading or trailing whitespace");
        }
        validate_action_family_name(trimmed)?;
        families.insert(trimmed.to_owned());
    }

    Ok(families)
}

fn known_action_family(family: &str) -> bool {
    known_action_families().contains(&family)
}

pub fn known_action_families() -> [&'static str; 9] {
    [
        "cpu_affinity_profile",
        "nice",
        "ionice",
        "uclamp",
        "cgroup_placement",
        "irq_affinity",
        "cpu_power",
        "gpu_power",
        "vm_knob",
    ]
}

fn all_objectives() -> [ObjectiveKind; 9] {
    use ObjectiveKind::*;

    [
        StutterScore,
        GameFramePacing,
        GameRunnableLatency,
        DesktopInteractivity,
        BrowserInteractivity,
        CompileThroughputWithForegroundProtection,
        IoLatency,
        IrqOverlapReduction,
        ThermalRecovery,
    ]
}

fn normalize_name(value: &str) -> String {
    value
        .chars()
        .filter(|ch| *ch != '_' && *ch != '-' && !ch.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

fn all_situations() -> [SituationKind; 20] {
    use SituationKind::*;

    [
        Unknown,
        Idle,
        GameFocused,
        GameCpuSchedulerPressure,
        GameGpuBound,
        CompositorPressure,
        CpuPressure,
        IoPressure,
        IrqPressure,
        ThermalOrPowerLimit,
        CompileLoad,
        BrowserFocused,
        BrowserCpuPressure,
        BrowserGpuVideo,
        BrowserIoPressure,
        CompileCpuBound,
        CompileLinkerPressure,
        MediaPlayback,
        Recording,
        VirtualMachineLoad,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_policy_enables_game_provider_families() {
        let rule = workload_policy_for_situation(SituationKind::GameCpuSchedulerPressure);

        assert!(rule.allowed_families.contains("cpu_affinity_profile"));
        assert!(rule.allowed_families.contains("uclamp"));
        assert!(rule.allowed_families.contains("gpu_power"));
        assert!(
            rule.allowed_objectives
                .contains(&ObjectiveKind::GameRunnableLatency)
        );
        assert!(rule.autonomous_families.contains("cpu_affinity_profile"));
    }

    #[test]
    fn recording_policy_blocks_game_only_aggressive_families() {
        let rule = workload_policy_for_situation(SituationKind::Recording);

        assert!(!rule.allowed_families.contains("cpu_power"));
        assert!(!rule.allowed_families.contains("gpu_power"));
        assert!(!rule.allowed_families.contains("cpu_affinity_profile"));
        assert!(rule.allowed_families.contains("uclamp"));
    }

    #[test]
    fn autonomous_policy_uses_autonomous_families_not_allowed_families() {
        let candidate = CandidateAction::fake(
            crate::actions::ActionId("fake-autonomous-test".to_owned()),
            crate::actions::SafetyClass::ReversibleLowRisk,
        );
        let allowed_only = WorkloadPolicyRule {
            situation: SituationKind::Unknown,
            allowed_families: std::collections::BTreeSet::from(["fake".to_owned()]),
            allowed_objectives: std::collections::BTreeSet::new(),
            autonomous_families: std::collections::BTreeSet::new(),
        };

        assert!(allowed_only.allows_candidate(&candidate));
        assert!(!allowed_only.allows_autonomous_candidate(&candidate));

        let autonomous = WorkloadPolicyRule {
            autonomous_families: std::collections::BTreeSet::from(["fake".to_owned()]),
            ..allowed_only
        };

        assert!(autonomous.allows_autonomous_candidate(&candidate));
    }

    #[test]
    fn browser_policy_rejects_compile_throughput_objective() {
        let rule = workload_policy_for_situation(SituationKind::BrowserFocused);

        assert!(!rule.allows_objective(ObjectiveKind::CompileThroughputWithForegroundProtection));
    }

    #[test]
    fn matrix_overrides_specific_situation_and_keeps_defaults() {
        let override_rule = WorkloadPolicyRule {
            situation: SituationKind::BrowserFocused,
            allowed_families: ["nice"].into_iter().map(str::to_owned).collect(),
            allowed_objectives: [ObjectiveKind::BrowserInteractivity].into_iter().collect(),
            autonomous_families: BTreeSet::new(),
        };

        let matrix = WorkloadPolicyMatrix::with_overrides(vec![override_rule]).unwrap();

        assert_eq!(
            matrix
                .rule_for(SituationKind::BrowserFocused)
                .allowed_families,
            ["nice"].into_iter().map(str::to_owned).collect()
        );
        assert!(
            matrix
                .rule_for(SituationKind::GameCpuSchedulerPressure)
                .allowed_families
                .contains("cpu_affinity_profile")
        );
    }

    #[test]
    fn config_rule_validation_rejects_unknown_family_and_duplicate_situations() {
        let unknown_family = WorkloadPolicyRuleConfigFile {
            situation: "browser_focused".to_owned(),
            allowed_families: vec!["mystery_knob".to_owned()],
            allowed_objectives: vec!["browser_interactivity".to_owned()],
            autonomous_families: Vec::new(),
        };
        assert!(unknown_family.into_rule().is_err());

        let duplicate = WorkloadPolicyRuleConfigFile {
            situation: "browser_focused".to_owned(),
            allowed_families: vec!["nice".to_owned()],
            allowed_objectives: vec!["browser_interactivity".to_owned()],
            autonomous_families: Vec::new(),
        };

        assert!(
            parse_workload_policy_rule_configs(&[duplicate.clone(), duplicate])
                .unwrap_err()
                .to_string()
                .contains("duplicate workload policy rule")
        );
    }
}
