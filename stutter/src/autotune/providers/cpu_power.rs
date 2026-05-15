use crate::{
    actions::cpu_power::CpuPowerAction,
    autotune::{
        candidate::{CandidateAction, CandidateEvidence, CpuPowerActionPlan},
        objective::ObjectiveKind,
        providers::{CandidateProposal, CandidateProvider, CandidateProviderInput},
        situation::SituationKind,
    },
    system_inventory::SystemInventory,
};

#[derive(Default)]
pub struct CpuPowerProvider;

impl CandidateProvider for CpuPowerProvider {
    fn family(&self) -> &'static str {
        "cpu_power"
    }

    fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        if !matches!(
            input.observation.primary_situation,
            SituationKind::GameCpuSchedulerPressure | SituationKind::CompileCpuBound
        ) || !input.system_health.ok_for_apply
        {
            return Vec::new();
        }

        let inventory = SystemInventory::probe();
        let Some(policy) = inventory.cpu_policies.first() else {
            return Vec::new();
        };
        let cpu = first_cpu(policy.related_cpus.as_deref()).unwrap_or(0);
        let action = CpuPowerAction {
            sysfs_root: std::path::PathBuf::from("/sys"),
            cpus: vec![cpu],
            scaling_governor: Some("performance".to_owned()),
            energy_performance_preference: Some("performance".to_owned()),
        };
        let objective = match input.observation.primary_situation {
            SituationKind::CompileCpuBound => {
                ObjectiveKind::CompileThroughputWithForegroundProtection
            }
            _ => ObjectiveKind::GameRunnableLatency,
        };
        let candidate = CandidateAction::CpuPower {
            plan: CpuPowerActionPlan {
                name: format!("cpu-power-policy-{}-performance", policy.policy),
                action,
                evidence: vec![CandidateEvidence::new(
                    "inventory",
                    format!("{} cpu={cpu}", policy.policy),
                    0.7,
                )],
                objective,
            },
        };

        vec![CandidateProposal {
            candidate,
            provider: self.family(),
            confidence: input.observation.situation.confidence,
            deny_reasons: Vec::new(),
            objective,
            rank_hint: 80,
        }]
    }
}

fn first_cpu(related_cpus: Option<&str>) -> Option<u32> {
    related_cpus?
        .split_whitespace()
        .find_map(|part| part.parse::<u32>().ok())
}
