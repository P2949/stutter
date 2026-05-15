use std::path::{Path, PathBuf};

use crate::{
    actions::{
        TaskIdentity,
        cgroup::{CgroupPlacementAction, CgroupPlacementTarget},
    },
    autotune::{
        candidate::{CandidateAction, CandidateEvidence, CgroupPlacementActionPlan},
        objective::ObjectiveKind,
        observation::{ActiveTaskSnapshot, is_protected_task_class},
        protection::mutation_allowed_for_pid,
        providers::{CandidateProposal, CandidateProvider, CandidateProviderInput},
        situation::SituationKind,
    },
    daemon::CgroupTargetRole,
    process_tree::TaskClass,
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
        let targets = cgroup_targets_for_observation(input.observation, root_pid, role);
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
                evidence: vec![
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
                ],
                objective,
            },
        };

        vec![CandidateProposal {
            candidate,
            provider: self.family(),
            confidence: input.observation.situation.confidence,
            deny_reasons: Vec::new(),
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
    root_pid: u32,
    role: &str,
) -> Vec<CgroupPlacementTarget> {
    let selected = if observation.active_tasks.is_empty() {
        vec![fallback_task_snapshot(observation, root_pid, role)]
    } else {
        observation.active_tasks.clone()
    };

    selected
        .into_iter()
        .filter(|task| cgroup_candidate_allows_task(task, role))
        .map(|task| CgroupPlacementTarget {
            identity: TaskIdentity {
                tid: task.tid,
                process_pid: Some(task.process_pid),
                comm: Some(task.comm),
                starttime_ticks: task.task_starttime_ticks.or(task.process_starttime_ticks),
            },
            class: task.class,
        })
        .collect()
}

fn fallback_task_snapshot(
    observation: &crate::autotune::observation::AutotuneObservation,
    root_pid: u32,
    role: &str,
) -> ActiveTaskSnapshot {
    ActiveTaskSnapshot {
        tid: root_pid,
        process_pid: root_pid,
        comm: observation
            .workload_identity
            .as_ref()
            .map(|identity| format!("pid-{}", identity.root_pid))
            .unwrap_or_else(|| format!("pid-{root_pid}")),
        class: fallback_class_for_role(role),
        process_starttime_ticks: observation
            .workload_identity
            .as_ref()
            .and_then(|identity| identity.process_starttime_ticks),
        task_starttime_ticks: observation
            .workload_identity
            .as_ref()
            .and_then(|identity| identity.process_starttime_ticks),
        cgroup_path: observation
            .workload_identity
            .as_ref()
            .and_then(|identity| identity.cgroup_path.clone()),
    }
}

fn fallback_class_for_role(role: &str) -> TaskClass {
    match role {
        "compile" => TaskClass::Compiler,
        "game" => TaskClass::Game,
        "virtual_machine" => TaskClass::VirtualMachine,
        _ => TaskClass::Helper,
    }
}

fn cgroup_candidate_allows_task(task: &ActiveTaskSnapshot, role: &str) -> bool {
    if task.tid == 0 || is_protected_task_class(task.class) {
        return false;
    }

    match role {
        "compile" => matches!(
            task.class,
            TaskClass::BuildJob
                | TaskClass::Compiler
                | TaskClass::Linker
                | TaskClass::PackageManager
                | TaskClass::Indexer
                | TaskClass::Helper
        ),
        "game" => matches!(
            task.class,
            TaskClass::Game
                | TaskClass::GameHelper
                | TaskClass::GameRenderThread
                | TaskClass::GameWorkerThread
                | TaskClass::WineServer
                | TaskClass::SteamRuntime
                | TaskClass::Helper
        ),
        "virtual_machine" => matches!(task.class, TaskClass::VirtualMachine | TaskClass::Helper),
        _ => !matches!(
            task.class,
            TaskClass::AudioRealtime
                | TaskClass::Input
                | TaskClass::KernelThread
                | TaskClass::IrqThread
                | TaskClass::Service
                | TaskClass::NetworkDaemon
                | TaskClass::StorageDaemon
                | TaskClass::Unknown
        ),
    }
}
