use std::collections::BTreeSet;

use anyhow::Context;

use super::{
    defaults::all_situations,
    model::{
        DaemonWorkloadPolicyConfig, DaemonWorkloadPolicyConfigFile, WorkloadPolicyMatrix,
        WorkloadPolicyRule, WorkloadPolicyRuleConfigFile,
    },
};
use crate::autotune::{objective::ObjectiveKind, situation::SituationKind};

impl DaemonWorkloadPolicyConfig {
    pub fn resolved_matrix(&self) -> anyhow::Result<WorkloadPolicyMatrix> {
        WorkloadPolicyMatrix::with_overrides(self.rules.clone())
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

impl DaemonWorkloadPolicyConfigFile {
    pub fn into_config(self) -> anyhow::Result<DaemonWorkloadPolicyConfig> {
        parse_workload_policy_rule_configs(&self.rules)
    }
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
