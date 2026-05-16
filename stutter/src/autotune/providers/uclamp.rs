use crate::{
    actions::uclamp::{UclampAction, UclampValues},
    autotune::{
        candidate::{CandidateAction, CandidateEvidence, UclampActionPlan},
        objective::ObjectiveKind,
        protection::mutation_allowed_for_pid,
        providers::{CandidateProposal, CandidateProvider, CandidateProviderInput},
        situation::SituationKind,
        target_selection::{
            TargetSelectionMode, TaskTargetSelector, mutable_task_targets_for_observation_with_mode,
        },
    },
};

#[derive(Default)]
pub struct UclampProvider;

impl CandidateProvider for UclampProvider {
    fn family(&self) -> &'static str {
        "uclamp"
    }

    fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        if !input.capabilities.uclamp_available || !input.system_health.ok_for_apply {
            return Vec::new();
        }
        let Some(root_pid) = input.observation.target_root_pid else {
            return Vec::new();
        };
        if !mutation_allowed_for_pid(root_pid, self.family(), input.observation).is_allowed() {
            return Vec::new();
        }

        let (values, objective, name_suffix) = match input.observation.primary_situation {
            SituationKind::GameCpuSchedulerPressure | SituationKind::CompositorPressure => (
                UclampValues {
                    sched_util_min: Some(128),
                    sched_util_max: None,
                },
                ObjectiveKind::GameRunnableLatency,
                "min-128",
            ),
            SituationKind::CompileLoad | SituationKind::CompileCpuBound => (
                UclampValues {
                    sched_util_min: None,
                    sched_util_max: Some(768),
                },
                ObjectiveKind::DesktopInteractivity,
                "max-768",
            ),
            _ => return Vec::new(),
        };

        let selector = match input.observation.primary_situation {
            SituationKind::GameCpuSchedulerPressure => TaskTargetSelector::GameRenderAndWorkers,
            SituationKind::CompositorPressure => TaskTargetSelector::FullTargetTree,
            SituationKind::CompileLoad | SituationKind::CompileCpuBound => {
                TaskTargetSelector::CompilerAndLinker
            }
            _ => TaskTargetSelector::ForegroundRootOnly,
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

        let action = UclampAction { targets, values };
        let candidate = CandidateAction::Uclamp {
            plan: UclampActionPlan {
                name: format!("uclamp-root-{root_pid}-{name_suffix}"),
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
                objective,
            },
        };

        vec![CandidateProposal {
            candidate,
            provider: self.family(),
            confidence: input.observation.situation.confidence,
            deny_reasons: target_selection_denies,
            objective,
            rank_hint: 25,
        }]
    }
}
