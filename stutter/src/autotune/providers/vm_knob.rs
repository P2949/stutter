use std::path::PathBuf;

use crate::{
    actions::vm_knobs::{VmKnobAction, VmKnobChange},
    autotune::{
        candidate::{CandidateAction, CandidateEvidence, VmKnobActionPlan},
        objective::ObjectiveKind,
        providers::{CandidateProposal, CandidateProvider, CandidateProviderInput},
        situation::SituationKind,
    },
};

#[derive(Default)]
pub struct VmKnobProvider;

impl CandidateProvider for VmKnobProvider {
    fn family(&self) -> &'static str {
        "vm_knob"
    }

    fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        if !matches!(
            input.observation.primary_situation,
            SituationKind::IoPressure | SituationKind::BrowserIoPressure
        ) {
            return Vec::new();
        }

        let current_swappiness = input
            .system_context
            .inventory
            .vm_knobs
            .get("sys/vm/swappiness")
            .cloned()
            .unwrap_or_else(|| "unknown".to_owned());

        let candidate = CandidateAction::VmKnob {
            plan: VmKnobActionPlan {
                name: "vm-swappiness-investigate-10".to_owned(),
                action: VmKnobAction {
                    root: PathBuf::from("/"),
                    changes: vec![VmKnobChange {
                        path: PathBuf::from("proc/sys/vm/swappiness"),
                        value: "10".to_owned(),
                    }],
                },
                evidence: vec![CandidateEvidence::new(
                    "situation",
                    format!(
                        "{:?}; current_swappiness={current_swappiness}",
                        input.observation.primary_situation
                    ),
                    input.observation.situation.confidence,
                )],
                objective: ObjectiveKind::IoLatency,
            },
        };

        vec![CandidateProposal {
            candidate,
            provider: self.family(),
            confidence: input.observation.situation.confidence,
            deny_reasons: Vec::new(),
            objective: ObjectiveKind::IoLatency,
            rank_hint: 90,
        }]
    }
}
