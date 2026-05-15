use std::collections::BTreeSet;

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
}

pub fn workload_policy_for_situation(situation: SituationKind) -> WorkloadPolicyRule {
    WorkloadPolicyMatrix::default_rules().rule_for(situation)
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
        let candidate = CandidateAction::Fake {
            action_id: crate::actions::ActionId("fake-autonomous-test".to_owned()),
            safety_class: crate::actions::SafetyClass::ReversibleLowRisk,
        };
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
}
