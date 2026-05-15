use crate::{
    actions::irq_affinity::{IrqAffinityAction, IrqAffinityEvidence, IrqAffinityRisk},
    autotune::{
        candidate::{CandidateAction, CandidateEvidence, IrqAffinityActionPlan},
        objective::ObjectiveKind,
        providers::{CandidateProposal, CandidateProvider, CandidateProviderInput},
        situation::SituationKind,
    },
};

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

        let Some((irq, hint)) = observed_irq(input) else {
            return Vec::new();
        };

        let evidence = IrqAffinityEvidence {
            strong_irq_evidence: true,
            stable_irq_identity: false,
            known_device_mapping: !hint.is_empty(),
            observed_irq: Some(irq),
            observed_device_hint: Some(hint.clone()),
            reason: "IRQ pressure classified from live diagnosis".to_owned(),
        };
        let candidate = CandidateAction::IrqAffinity {
            plan: IrqAffinityActionPlan {
                name: format!("irq-{irq}-investigate-affinity"),
                action: IrqAffinityAction::new(
                    irq,
                    hint.clone(),
                    "1".to_owned(),
                    IrqAffinityRisk::HighRisk,
                    evidence,
                ),
                evidence: vec![CandidateEvidence::new("irq", hint, 0.8)],
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

fn observed_irq(input: &CandidateProviderInput<'_>) -> Option<(u32, String)> {
    for diagnosis in &input.observation.recent_diagnoses {
        for evidence in &diagnosis.evidence {
            let lower = evidence.to_ascii_lowercase();
            if !lower.contains("irq") {
                continue;
            }
            let irq = lower
                .split(|ch: char| !ch.is_ascii_digit())
                .find_map(|part| {
                    (!part.is_empty())
                        .then(|| part.parse::<u32>().ok())
                        .flatten()
                })?;
            return Some((irq, diagnosis.anchor_comm.clone()));
        }
    }
    None
}
