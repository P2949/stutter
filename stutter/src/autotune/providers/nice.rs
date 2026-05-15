use crate::{
    actions::nice::{NiceAction, NicePolicy},
    autotune::{
        candidate::{CandidateAction, CandidateEvidence, NiceActionPlan},
        objective::ObjectiveKind,
        protection::mutation_allowed_for_pid,
        providers::{CandidateProposal, CandidateProvider, CandidateProviderInput},
        situation::SituationKind,
        target_selection::{TaskTargetSelector, mutable_task_targets_for_observation},
    },
};

#[derive(Default)]
pub struct NiceProvider;

impl CandidateProvider for NiceProvider {
    fn family(&self) -> &'static str {
        "nice"
    }

    fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        let Some(root_pid) = input.observation.target_root_pid else {
            return Vec::new();
        };
        if !mutation_allowed_for_pid(root_pid, self.family(), input.observation).is_allowed() {
            return Vec::new();
        }

        let (nice, objective, _reason) = match input.observation.primary_situation {
            SituationKind::CompileLoad
            | SituationKind::CompileCpuBound
            | SituationKind::CompileLinkerPressure => (
                5,
                ObjectiveKind::DesktopInteractivity,
                "compile/background work can be lowered to protect interactivity",
            ),
            SituationKind::BrowserCpuPressure | SituationKind::BrowserFocused => (
                -1,
                ObjectiveKind::BrowserInteractivity,
                "browser foreground CPU pressure",
            ),
            _ => return Vec::new(),
        };

        let selector = match input.observation.primary_situation {
            SituationKind::CompileLoad
            | SituationKind::CompileCpuBound
            | SituationKind::CompileLinkerPressure => TaskTargetSelector::CompilerAndLinker,
            SituationKind::BrowserCpuPressure | SituationKind::BrowserFocused => {
                TaskTargetSelector::BrowserRenderersAndHelpers
            }
            _ => TaskTargetSelector::ForegroundRootOnly,
        };
        let targets = mutable_task_targets_for_observation(input.observation, selector);
        if targets.is_empty() {
            return Vec::new();
        }

        let action = NiceAction {
            targets,
            nice,
            policy: NicePolicy {
                allow_nice_changes: true,
                min_nice: -1,
                max_nice: 19,
            },
        };
        let candidate = CandidateAction::Nice {
            plan: NiceActionPlan {
                name: format!("nice-root-{root_pid}-to-{nice}"),
                action,
                target_root_pid: Some(root_pid),
                evidence: vec![CandidateEvidence::new(
                    "situation",
                    format!("{:?}", input.observation.primary_situation),
                    input.observation.situation.confidence,
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
            rank_hint: 30,
        }]
    }
}
