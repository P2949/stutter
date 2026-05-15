use crate::{
    actions::irq_affinity::{IrqAffinityAction, IrqAffinityEvidence, IrqAffinityRisk},
    autotune::{
        candidate::{CandidateAction, CandidateEvidence, IrqAffinityActionPlan},
        objective::ObjectiveKind,
        providers::{CandidateProposal, CandidateProvider, CandidateProviderInput},
        situation::SituationKind,
    },
};

#[derive(Clone, Debug, PartialEq)]
pub struct IrqCandidateEvidence {
    pub irq: u32,
    pub device: String,
    pub current_mask: String,
    pub suggested_mask: String,
    pub overlap_score: f32,
    pub stable_identity: bool,
    pub known_device_mapping: bool,
}

#[derive(Default)]
pub struct IrqAffinityProvider;

impl CandidateProvider for IrqAffinityProvider {
    fn family(&self) -> &'static str {
        "irq_affinity"
    }

    fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        if !input.capabilities.irq_affinity_available
            || input.observation.primary_situation != SituationKind::IrqPressure
        {
            return Vec::new();
        }

        let Some(evidence_model) = observed_irq(input) else {
            return Vec::new();
        };

        let evidence = IrqAffinityEvidence {
            strong_irq_evidence: true,
            stable_irq_identity: evidence_model.stable_identity,
            known_device_mapping: evidence_model.known_device_mapping,
            observed_irq: Some(evidence_model.irq),
            observed_device_hint: Some(evidence_model.device.clone()),
            reason: format!(
                "IRQ pressure classified from structured active IRQ state; current_mask={} suggested_mask={}",
                evidence_model.current_mask, evidence_model.suggested_mask
            ),
        };
        let candidate = CandidateAction::IrqAffinity {
            plan: IrqAffinityActionPlan {
                name: format!("irq-{}-investigate-affinity", evidence_model.irq),
                action: IrqAffinityAction::new(
                    evidence_model.irq,
                    evidence_model.device.clone(),
                    evidence_model.suggested_mask.clone(),
                    IrqAffinityRisk::HighRisk,
                    evidence,
                ),
                evidence: vec![CandidateEvidence::new(
                    "irq",
                    format!(
                        "irq={} device={} current_mask={} suggested_mask={} overlap_score={:.3}",
                        evidence_model.irq,
                        evidence_model.device,
                        evidence_model.current_mask,
                        evidence_model.suggested_mask,
                        evidence_model.overlap_score
                    ),
                    0.8,
                )],
                objective: ObjectiveKind::IrqOverlapReduction,
            },
        };

        vec![CandidateProposal {
            candidate,
            provider: self.family(),
            confidence: input.observation.situation.confidence,
            deny_reasons: Vec::new(),
            objective: ObjectiveKind::IrqOverlapReduction,
            rank_hint: 60,
        }]
    }
}

fn observed_irq(input: &CandidateProviderInput<'_>) -> Option<IrqCandidateEvidence> {
    let active_irq = &input
        .observation
        .active_config_snapshot
        .as_ref()?
        .irq
        .per_irq;
    let (&irq, current_mask) = active_irq.iter().next()?;
    if active_irq.len() != 1 {
        return None;
    }
    let default_mask = input
        .system_context
        .inventory
        .irq_default_smp_affinity
        .clone()
        .unwrap_or_else(|| current_mask.clone());
    let suggested_mask = suggested_irq_mask(&default_mask)?;

    Some(IrqCandidateEvidence {
        irq,
        device: format!("irq-{irq}"),
        current_mask: current_mask.clone(),
        suggested_mask,
        overlap_score: input.observation.situation.confidence,
        stable_identity: true,
        known_device_mapping: false,
    })
}

fn suggested_irq_mask(default_mask: &str) -> Option<String> {
    let trimmed = default_mask.trim();
    if trimmed.is_empty() || trimmed == "unknown" {
        None
    } else {
        Some(trimmed.to_owned())
    }
}
