use crate::{
    actions::ioprio::{IoPrioAction, IoPrioPolicy, IoPrioValue},
    autotune::{
        candidate::{CandidateAction, CandidateEvidence, IoPrioActionPlan},
        objective::ObjectiveKind,
        protection::mutation_allowed_for_pid,
        providers::{CandidateProposal, CandidateProvider, CandidateProviderInput},
        situation::SituationKind,
        target_selection::{TaskTargetSelector, mutable_task_targets_for_observation},
    },
};

#[derive(Default)]
pub struct IoPrioProvider;

impl CandidateProvider for IoPrioProvider {
    fn family(&self) -> &'static str {
        "ionice"
    }

    fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        if !input.capabilities.ionice_available {
            return Vec::new();
        }
        if !matches!(
            input.observation.primary_situation,
            SituationKind::IoPressure
                | SituationKind::BrowserIoPressure
                | SituationKind::CompileLinkerPressure
        ) {
            return Vec::new();
        }
        let Some(root_pid) = input.observation.target_root_pid else {
            return Vec::new();
        };
        if !mutation_allowed_for_pid(root_pid, self.family(), input.observation).is_allowed() {
            return Vec::new();
        }

        let selector = match input.observation.primary_situation {
            SituationKind::CompileLinkerPressure => TaskTargetSelector::CompilerAndLinker,
            SituationKind::BrowserIoPressure => TaskTargetSelector::BrowserRenderersAndHelpers,
            _ => TaskTargetSelector::FullTargetTree,
        };
        let targets = mutable_task_targets_for_observation(input.observation, selector);
        if targets.is_empty() {
            return Vec::new();
        }

        let action = IoPrioAction {
            targets,
            ioprio: IoPrioValue::idle(),
            policy: IoPrioPolicy {
                allow_ioprio_changes: true,
                strong_block_io_evidence: true,
                ..IoPrioPolicy::default()
            },
        };
        let candidate = CandidateAction::IoPrio {
            plan: IoPrioActionPlan {
                name: format!("ionice-root-{root_pid}-idle"),
                action,
                target_root_pid: Some(root_pid),
                evidence: vec![CandidateEvidence::new(
                    "situation",
                    format!("{:?}", input.observation.primary_situation),
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
            rank_hint: 20,
        }]
    }
}
