use crate::{
    actions::gpu_power::GpuPowerAction,
    autotune::{
        candidate::{CandidateAction, CandidateEvidence, GpuPowerActionPlan},
        objective::ObjectiveKind,
        providers::{CandidateProposal, CandidateProvider, CandidateProviderInput},
        situation::SituationKind,
    },
    system_inventory::SystemInventory,
};

#[derive(Default)]
pub struct GpuPowerProvider;

impl CandidateProvider for GpuPowerProvider {
    fn family(&self) -> &'static str {
        "gpu_power"
    }

    fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        if !matches!(
            input.observation.primary_situation,
            SituationKind::GameGpuBound | SituationKind::BrowserGpuVideo
        ) || !input.system_health.ok_for_apply
        {
            return Vec::new();
        }

        let inventory = SystemInventory::probe();
        let Some(card) = inventory.drm_devices.first() else {
            return Vec::new();
        };
        let candidate = CandidateAction::GpuPower {
            plan: GpuPowerActionPlan {
                name: format!("gpu-power-{}-high", card.name),
                action: GpuPowerAction {
                    sysfs_root: std::path::PathBuf::from("/sys"),
                    drm_card: card.name.clone(),
                    power_dpm_force_performance_level: Some("high".to_owned()),
                    pp_power_profile_mode: None,
                },
                evidence: vec![CandidateEvidence::new(
                    "inventory",
                    card.render_node.as_deref().unwrap_or(&card.name),
                    0.7,
                )],
                objective: ObjectiveKind::GameFramePacing,
            },
        };

        vec![CandidateProposal {
            candidate,
            provider: self.family(),
            confidence: input.observation.situation.confidence,
            deny_reasons: Vec::new(),
            objective: ObjectiveKind::GameFramePacing,
            rank_hint: 85,
        }]
    }
}
