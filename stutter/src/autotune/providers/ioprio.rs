use crate::{
    actions::ioprio::{IoPrioAction, IoPrioPolicy, IoPrioValue},
    autotune::{
        objective::ObjectiveKind,
        planning::{
            candidate::{CandidateAction, CandidateEvidence},
            executable_plan::IoPrioActionPlan,
        },
        protection::mutation_allowed_for_pid,
        providers::{CandidateProposal, CandidateProvider, CandidateProviderInput},
        situation::SituationKind,
        target_selection::{
            TargetSelectionMode, TaskTargetSelector, mutable_task_targets_for_observation_with_mode,
        },
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
        let selection = mutable_task_targets_for_observation_with_mode(
            input.observation,
            selector,
            TargetSelectionMode::from_daemon_mode(input.daemon_policy.mode),
        );
        let target_selection_denies = selection.deny_reasons();
        let used_fallback_root = selection.used_fallback_root;
        let targets = selection.items;
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
                evidence: {
                    let mut evidence = vec![CandidateEvidence::new(
                        "situation",
                        format!("{:?}", input.observation.primary_situation),
                        input.observation.situation.confidence,
                    )];
                    if used_fallback_root {
                        evidence.push(CandidateEvidence::new(
                            "target_selection_fallback_root",
                            format!("root_pid={root_pid} active_task_snapshots=missing"),
                            0.0,
                        ));
                    }
                    evidence
                },
                objective: ObjectiveKind::IoLatency,
            },
        };

        vec![CandidateProposal {
            candidate,
            provider: self.family(),
            confidence: input.observation.situation.confidence,
            deny_reasons: target_selection_denies,
            objective: ObjectiveKind::IoLatency,
            rank_hint: 20,
        }]
    }
}
