use std::path::{Path, PathBuf};

use crate::{
    actions::cgroup::{CgroupPlacementAction, CgroupPlacementTarget},
    autotune::{
        objective::ObjectiveKind,
        planning::{
            candidate::{CandidateAction, CandidateEvidence},
            executable_plan::CgroupPlacementActionPlan,
        },
        protection::mutation_allowed_for_pid,
        providers::{CandidateProposal, CandidateProvider, CandidateProviderInput},
        situation::SituationKind,
        target_selection::{
            TargetSelectionMode, TaskTargetSelector,
            mutable_cgroup_targets_for_observation_with_mode,
        },
    },
    daemon::CgroupTargetRole,
};

#[derive(Default)]
pub struct CgroupProvider;

impl CandidateProvider for CgroupProvider {
    fn family(&self) -> &'static str {
        "cgroup_placement"
    }

    fn propose(&self, input: &CandidateProviderInput<'_>) -> Vec<CandidateProposal> {
        if input.daemon_policy.cgroup_targets.is_empty() {
            return Vec::new();
        }

        let Some(root_pid) = input.observation.target_root_pid else {
            return Vec::new();
        };
        if !mutation_allowed_for_pid(root_pid, self.family(), input.observation).is_allowed() {
            return Vec::new();
        }

        let Some(plan) = cgroup_plan_for_situation(input, root_pid) else {
            return Vec::new();
        };
        let (role, target_cgroup, objective, rank_hint) = plan;
        let target_selection_mode = TargetSelectionMode::from_daemon_mode(input.daemon_policy.mode);
        let target_selection = cgroup_targets_for_observation(
            input.observation,
            root_pid,
            role,
            target_selection_mode,
        );
        let target_selection_denies = target_selection.deny_reasons();
        let used_fallback_root = target_selection.used_fallback_root;
        let targets = target_selection.items;
        if targets.is_empty() {
            return Vec::new();
        }

        let action = CgroupPlacementAction {
            cgroup_root: PathBuf::from("/sys/fs/cgroup"),
            target_cgroup: target_cgroup.to_path_buf(),
            targets,
            cpuset_cpus: None,
            cpuset_mems: None,
        };
        let candidate = CandidateAction::CgroupPlacement {
            plan: CgroupPlacementActionPlan {
                name: format!(
                    "cgroup-{role}-root-{root_pid}-to-{}",
                    target_cgroup.display()
                ),
                action,
                target_root_pid: Some(root_pid),
                evidence: {
                    let mut evidence = vec![
                        CandidateEvidence::new(
                            "situation",
                            format!("{:?}", input.observation.primary_situation),
                            input.observation.situation.confidence,
                        ),
                        CandidateEvidence::new(
                            "target_cgroup",
                            target_cgroup.display().to_string(),
                            1.0,
                        ),
                        CandidateEvidence::new("cgroup_role", role, 0.8),
                    ];
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
            rank_hint,
        }]
    }
}

fn cgroup_plan_for_situation<'a>(
    input: &'a CandidateProviderInput<'_>,
    _root_pid: u32,
) -> Option<(&'static str, &'a Path, ObjectiveKind, u32)> {
    match input.observation.primary_situation {
        SituationKind::CompileLoad
        | SituationKind::CompileCpuBound
        | SituationKind::CompileLinkerPressure => input
            .daemon_policy
            .cgroup_targets
            .target_for_role(CgroupTargetRole::Compile)
            .or_else(|| {
                input
                    .daemon_policy
                    .cgroup_targets
                    .target_for_role(CgroupTargetRole::Background)
            })
            .map(|target| {
                (
                    "compile",
                    target,
                    ObjectiveKind::CompileThroughputWithForegroundProtection,
                    35,
                )
            }),
        SituationKind::GameFocused
        | SituationKind::GameCpuSchedulerPressure
        | SituationKind::GameGpuBound => input
            .daemon_policy
            .cgroup_targets
            .target_for_role(CgroupTargetRole::Game)
            .map(|target| ("game", target, ObjectiveKind::GameRunnableLatency, 45)),
        SituationKind::VirtualMachineLoad => input
            .daemon_policy
            .cgroup_targets
            .target_for_role(CgroupTargetRole::Background)
            .map(|target| {
                (
                    "virtual_machine",
                    target,
                    ObjectiveKind::DesktopInteractivity,
                    55,
                )
            }),
        _ => None,
    }
}

fn cgroup_targets_for_observation(
    observation: &crate::autotune::observation::AutotuneObservation,
    _root_pid: u32,
    role: &str,
    mode: TargetSelectionMode,
) -> crate::autotune::target_selection::MutableTaskSelection<CgroupPlacementTarget> {
    let selector = match role {
        "compile" => TaskTargetSelector::CompilerAndLinker,
        "game" => TaskTargetSelector::GameRenderAndWorkers,
        "virtual_machine" => TaskTargetSelector::VirtualMachineAndHelpers,
        _ => TaskTargetSelector::FullTargetTree,
    };
    mutable_cgroup_targets_for_observation_with_mode(observation, selector, mode)
}
