use super::{
    model::{WorkloadPolicyMatrix, WorkloadPolicyRule},
    parse::validate_workload_policy_rule,
};
use crate::autotune::{objective::ObjectiveKind, situation::SituationKind};

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

pub(super) fn all_situations() -> [SituationKind; 20] {
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
