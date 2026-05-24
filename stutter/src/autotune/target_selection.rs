use crate::{
    actions::{TaskIdentity, cgroup::CgroupPlacementTarget},
    autotune::observation::{ActiveTaskSnapshot, AutotuneObservation, is_protected_task_class},
    process_tree::TaskClass,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskTargetSelector {
    ForegroundRootOnly,
    FullTargetTree,
    CompilerAndLinker,
    VirtualMachineAndHelpers,
    GameRenderAndWorkers,
    BrowserRenderersAndHelpers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetSelectionMode {
    Suggest,
    ApplyCapable,
}

impl TargetSelectionMode {
    pub fn from_daemon_mode(mode: crate::daemon::policy::DaemonMode) -> Self {
        if mode == crate::daemon::policy::DaemonMode::Suggest {
            Self::Suggest
        } else {
            Self::ApplyCapable
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MutableTaskSelection<T> {
    pub items: Vec<T>,
    pub used_fallback_root: bool,
    pub missing_active_snapshots: bool,
}

impl<T> MutableTaskSelection<T> {
    pub fn deny_reasons(&self) -> Vec<String> {
        if self.used_fallback_root {
            vec![
                "target_selection_fallback_root: active task snapshots missing; using root PID only for suggest-mode display".to_owned(),
            ]
        } else if self.missing_active_snapshots {
            vec!["target_snapshot_missing: active task snapshots missing; no mutable targets selected".to_owned()]
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
pub fn mutable_task_targets_for_observation(
    observation: &AutotuneObservation,
    selector: TaskTargetSelector,
) -> Vec<TaskIdentity> {
    mutable_task_targets_for_observation_with_mode(
        observation,
        selector,
        TargetSelectionMode::ApplyCapable,
    )
    .items
}

pub fn mutable_task_targets_for_observation_with_mode(
    observation: &AutotuneObservation,
    selector: TaskTargetSelector,
    mode: TargetSelectionMode,
) -> MutableTaskSelection<TaskIdentity> {
    let selection = mutable_task_snapshots_for_observation_with_mode(observation, selector, mode);

    MutableTaskSelection {
        items: selection
            .items
            .into_iter()
            .map(task_identity_from_snapshot)
            .collect(),
        used_fallback_root: selection.used_fallback_root,
        missing_active_snapshots: selection.missing_active_snapshots,
    }
}

#[cfg(test)]
pub fn mutable_task_snapshots_for_observation(
    observation: &AutotuneObservation,
    selector: TaskTargetSelector,
) -> Vec<ActiveTaskSnapshot> {
    mutable_task_snapshots_for_observation_with_mode(
        observation,
        selector,
        TargetSelectionMode::ApplyCapable,
    )
    .items
}

pub fn mutable_cgroup_targets_for_observation_with_mode(
    observation: &AutotuneObservation,
    selector: TaskTargetSelector,
    mode: TargetSelectionMode,
) -> MutableTaskSelection<CgroupPlacementTarget> {
    let selection = mutable_task_snapshots_for_observation_with_mode(observation, selector, mode);

    MutableTaskSelection {
        items: selection
            .items
            .into_iter()
            .map(|task| {
                let class = task.class;
                CgroupPlacementTarget {
                    identity: task_identity_from_snapshot(task),
                    class,
                }
            })
            .collect(),
        used_fallback_root: selection.used_fallback_root,
        missing_active_snapshots: selection.missing_active_snapshots,
    }
}

pub fn mutable_task_snapshots_for_observation_with_mode(
    observation: &AutotuneObservation,
    selector: TaskTargetSelector,
    mode: TargetSelectionMode,
) -> MutableTaskSelection<ActiveTaskSnapshot> {
    let missing_active_snapshots = observation.active_tasks.is_empty();
    let used_fallback_root = missing_active_snapshots && mode == TargetSelectionMode::Suggest;
    let mut snapshots = if missing_active_snapshots {
        if used_fallback_root {
            fallback_root_snapshot(observation).into_iter().collect()
        } else {
            Vec::new()
        }
    } else {
        observation.active_tasks.clone()
    };

    snapshots.sort_by_key(|task| task.tid);
    snapshots.dedup_by_key(|task| task.tid);

    let expected_process_starttime_ticks = observation
        .workload_identity
        .as_ref()
        .and_then(|identity| identity.process_starttime_ticks);

    let items = snapshots
        .into_iter()
        .filter(|task| {
            mutable_task_matches_workload_starttime(task, expected_process_starttime_ticks)
        })
        .filter(|task| {
            mutable_task_allowed(
                task,
                selector,
                used_fallback_root,
                observation.target_root_pid,
                observation,
            )
        })
        .collect();

    MutableTaskSelection {
        items,
        used_fallback_root,
        missing_active_snapshots,
    }
}

fn mutable_task_matches_workload_starttime(
    task: &ActiveTaskSnapshot,
    expected_process_starttime_ticks: Option<u64>,
) -> bool {
    let Some(expected_process_starttime_ticks) = expected_process_starttime_ticks else {
        return true;
    };

    if let Some(process_starttime_ticks) = task.process_starttime_ticks
        && process_starttime_ticks != expected_process_starttime_ticks
    {
        return false;
    }

    if task.tid.as_u32() == task.process_pid.as_u32()
        && let Some(task_starttime_ticks) = task.task_starttime_ticks
        && task_starttime_ticks != expected_process_starttime_ticks
    {
        return false;
    }

    true
}

fn is_explicitly_protected_task(
    task: &ActiveTaskSnapshot,
    observation: &AutotuneObservation,
) -> bool {
    observation.protected_tasks.iter().any(|protected| {
        let task_tid = task.tid.as_u32();
        let task_process_pid = task.process_pid.as_u32();
        let protected_tid = protected.tid.as_u32();
        let protected_process_pid = protected.process_pid.as_u32();

        protected_tid == task_tid
            || protected_tid == task_process_pid
            || protected_process_pid == task_tid
            || protected_process_pid == task_process_pid
    })
}

fn mutable_task_allowed(
    task: &ActiveTaskSnapshot,
    selector: TaskTargetSelector,
    allow_unknown_fallback_root: bool,
    target_root_pid: Option<u32>,
    observation: &AutotuneObservation,
) -> bool {
    if task.tid.as_u32() == 0
        || is_protected_task_class(task.class)
        || is_explicitly_protected_task(task, observation)
    {
        return false;
    }

    if allow_unknown_fallback_root
        && task.class == TaskClass::Unknown
        && task.tid.as_u32() == task.process_pid.as_u32()
    {
        return true;
    }

    match selector {
        TaskTargetSelector::ForegroundRootOnly => {
            Some(task.tid.as_u32()) == target_root_pid
                && task.tid.as_u32() == task.process_pid.as_u32()
        }
        TaskTargetSelector::FullTargetTree => !matches!(task.class, TaskClass::Unknown),
        TaskTargetSelector::CompilerAndLinker => matches!(
            task.class,
            TaskClass::BuildJob
                | TaskClass::Compiler
                | TaskClass::Linker
                | TaskClass::PackageManager
                | TaskClass::Indexer
                | TaskClass::Helper
        ),
        TaskTargetSelector::VirtualMachineAndHelpers => {
            matches!(task.class, TaskClass::VirtualMachine | TaskClass::Helper)
        }
        TaskTargetSelector::GameRenderAndWorkers => matches!(
            task.class,
            TaskClass::Game
                | TaskClass::GameHelper
                | TaskClass::GameRenderThread
                | TaskClass::GameWorkerThread
                | TaskClass::WineServer
                | TaskClass::SteamRuntime
                | TaskClass::Helper
        ),
        TaskTargetSelector::BrowserRenderersAndHelpers => matches!(
            task.class,
            TaskClass::BrowserForeground
                | TaskClass::BrowserBackground
                | TaskClass::BrowserRenderer
                | TaskClass::BrowserGpu
                | TaskClass::BrowserNetwork
                | TaskClass::Helper
        ),
    }
}

fn fallback_root_snapshot(observation: &AutotuneObservation) -> Option<ActiveTaskSnapshot> {
    let root_pid = observation.target_root_pid?;
    Some(ActiveTaskSnapshot {
        tid: root_pid.into(),
        process_pid: root_pid.into(),
        comm: observation
            .workload_identity
            .as_ref()
            .map(|identity| format!("pid-{}", identity.root_pid))
            .unwrap_or_else(|| format!("pid-{root_pid}")),
        class: TaskClass::Unknown,
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
    })
}

fn task_identity_from_snapshot(task: ActiveTaskSnapshot) -> TaskIdentity {
    TaskIdentity {
        tid: task.tid.as_u32(),
        process_pid: Some(task.process_pid.as_u32()),
        comm: Some(task.comm),
        starttime_ticks: task.task_starttime_ticks.or(task.process_starttime_ticks),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autotune::observation::{ProtectedTask, WorkloadIdentity};

    fn task(tid: u32, class: TaskClass) -> ActiveTaskSnapshot {
        ActiveTaskSnapshot {
            tid: tid.into(),
            process_pid: (10).into(),
            comm: format!("{class:?}"),
            class,
            process_starttime_ticks: Some(100),
            task_starttime_ticks: Some(u64::from(tid)),
            cgroup_path: None,
        }
    }

    #[test]
    fn protected_task_classes_are_never_selected() {
        let protected = [
            TaskClass::AudioRealtime,
            TaskClass::Input,
            TaskClass::Compositor,
            TaskClass::KernelThread,
            TaskClass::IrqThread,
            TaskClass::Service,
            TaskClass::Unknown,
        ];
        let observation = AutotuneObservation {
            target_root_pid: Some(10),
            active_tasks: protected
                .into_iter()
                .enumerate()
                .map(|(idx, class)| task(idx as u32 + 1, class))
                .collect(),
            ..AutotuneObservation::default()
        };

        let targets =
            mutable_task_targets_for_observation(&observation, TaskTargetSelector::FullTargetTree);

        assert!(targets.is_empty());
    }

    #[test]
    fn explicit_protected_tasks_are_never_selected() {
        let observation = AutotuneObservation {
            target_root_pid: Some(10),
            active_tasks: vec![
                ActiveTaskSnapshot {
                    tid: (10).into(),
                    process_pid: (10).into(),
                    comm: "game-root".to_owned(),
                    class: TaskClass::Game,
                    process_starttime_ticks: Some(100),
                    task_starttime_ticks: Some(100),
                    cgroup_path: None,
                },
                ActiveTaskSnapshot {
                    tid: (11).into(),
                    process_pid: (10).into(),
                    comm: "protected-worker".to_owned(),
                    class: TaskClass::GameWorkerThread,
                    process_starttime_ticks: Some(100),
                    task_starttime_ticks: Some(101),
                    cgroup_path: None,
                },
                ActiveTaskSnapshot {
                    tid: (12).into(),
                    process_pid: (10).into(),
                    comm: "same-process-helper".to_owned(),
                    class: TaskClass::Helper,
                    process_starttime_ticks: Some(100),
                    task_starttime_ticks: Some(102),
                    cgroup_path: None,
                },
                ActiveTaskSnapshot {
                    tid: (20).into(),
                    process_pid: (20).into(),
                    comm: "unprotected-helper".to_owned(),
                    class: TaskClass::Helper,
                    process_starttime_ticks: Some(200),
                    task_starttime_ticks: Some(200),
                    cgroup_path: None,
                },
            ],
            protected_tasks: vec![ProtectedTask {
                tid: (11).into(),
                process_pid: (10).into(),
                comm: "protected-worker".to_owned(),
                class: TaskClass::GameWorkerThread,
                reason: "explicit protected task".to_owned(),
            }],
            ..AutotuneObservation::default()
        };

        let targets = mutable_task_targets_for_observation(
            &observation,
            TaskTargetSelector::GameRenderAndWorkers,
        );

        assert_eq!(
            targets.iter().map(|target| target.tid).collect::<Vec<_>>(),
            vec![20]
        );
    }

    #[test]
    fn game_selector_keeps_render_and_worker_tasks() {
        let observation = AutotuneObservation {
            target_root_pid: Some(10),
            active_tasks: vec![
                task(10, TaskClass::Game),
                task(11, TaskClass::GameRenderThread),
                task(12, TaskClass::GameWorkerThread),
                task(13, TaskClass::AudioRealtime),
            ],
            ..AutotuneObservation::default()
        };

        let targets = mutable_task_targets_for_observation(
            &observation,
            TaskTargetSelector::GameRenderAndWorkers,
        );

        assert_eq!(
            targets.iter().map(|target| target.tid).collect::<Vec<_>>(),
            vec![10, 11, 12]
        );
    }

    #[test]
    fn snapshot_selection_helper_returns_mutable_snapshots_for_tests() {
        let observation = AutotuneObservation {
            target_root_pid: Some(10),
            active_tasks: vec![
                task(10, TaskClass::Game),
                task(11, TaskClass::GameRenderThread),
                task(12, TaskClass::GameWorkerThread),
                task(13, TaskClass::AudioRealtime),
            ],
            ..AutotuneObservation::default()
        };

        let snapshots = mutable_task_snapshots_for_observation(
            &observation,
            TaskTargetSelector::GameRenderAndWorkers,
        );

        assert_eq!(
            snapshots
                .iter()
                .map(|task| task.tid.as_u32())
                .collect::<Vec<_>>(),
            vec![10, 11, 12]
        );
        assert!(snapshots.iter().all(|task| task.process_pid.as_u32() == 10));
    }

    #[test]
    fn foreground_root_selector_keeps_only_target_root_pid() {
        let observation = AutotuneObservation {
            target_root_pid: Some(10),
            active_tasks: vec![
                ActiveTaskSnapshot {
                    tid: (10).into(),
                    process_pid: (10).into(),
                    comm: "foreground-root".to_owned(),
                    class: TaskClass::Game,
                    process_starttime_ticks: Some(100),
                    task_starttime_ticks: Some(100),
                    cgroup_path: None,
                },
                ActiveTaskSnapshot {
                    tid: (11).into(),
                    process_pid: (11).into(),
                    comm: "child-process-main-thread".to_owned(),
                    class: TaskClass::GameWorkerThread,
                    process_starttime_ticks: Some(101),
                    task_starttime_ticks: Some(101),
                    cgroup_path: None,
                },
                ActiveTaskSnapshot {
                    tid: (12).into(),
                    process_pid: (10).into(),
                    comm: "foreground-worker-thread".to_owned(),
                    class: TaskClass::GameWorkerThread,
                    process_starttime_ticks: Some(100),
                    task_starttime_ticks: Some(102),
                    cgroup_path: None,
                },
            ],
            ..AutotuneObservation::default()
        };

        let targets = mutable_task_targets_for_observation(
            &observation,
            TaskTargetSelector::ForegroundRootOnly,
        );

        assert_eq!(
            targets.iter().map(|target| target.tid).collect::<Vec<_>>(),
            vec![10]
        );
    }

    #[test]
    fn stale_process_starttime_snapshots_are_not_selected() {
        let observation = AutotuneObservation {
            target_root_pid: Some(10),
            workload_identity: Some(WorkloadIdentity {
                root_pid: 10,
                process_starttime_ticks: Some(100),
                exe_dev: None,
                exe_ino: None,
                cgroup_path: None,
                focus_kind: None,
                class_distribution: std::collections::BTreeMap::new(),
                stable_hash: "test-hash".to_owned(),
            }),
            active_tasks: vec![
                ActiveTaskSnapshot {
                    tid: (10).into(),
                    process_pid: (10).into(),
                    comm: "current-root".to_owned(),
                    class: TaskClass::Game,
                    process_starttime_ticks: Some(100),
                    task_starttime_ticks: Some(100),
                    cgroup_path: None,
                },
                ActiveTaskSnapshot {
                    tid: (11).into(),
                    process_pid: (10).into(),
                    comm: "current-worker".to_owned(),
                    class: TaskClass::GameWorkerThread,
                    process_starttime_ticks: Some(100),
                    task_starttime_ticks: Some(101),
                    cgroup_path: None,
                },
                ActiveTaskSnapshot {
                    tid: (12).into(),
                    process_pid: (10).into(),
                    comm: "stale-worker".to_owned(),
                    class: TaskClass::GameWorkerThread,
                    process_starttime_ticks: Some(99),
                    task_starttime_ticks: Some(102),
                    cgroup_path: None,
                },
                ActiveTaskSnapshot {
                    tid: (13).into(),
                    process_pid: (13).into(),
                    comm: "stale-root".to_owned(),
                    class: TaskClass::Game,
                    process_starttime_ticks: Some(99),
                    task_starttime_ticks: Some(99),
                    cgroup_path: None,
                },
                ActiveTaskSnapshot {
                    tid: (14).into(),
                    process_pid: (10).into(),
                    comm: "missing-starttime-worker".to_owned(),
                    class: TaskClass::GameWorkerThread,
                    process_starttime_ticks: None,
                    task_starttime_ticks: None,
                    cgroup_path: None,
                },
            ],
            ..AutotuneObservation::default()
        };

        let targets = mutable_task_targets_for_observation(
            &observation,
            TaskTargetSelector::GameRenderAndWorkers,
        );

        assert_eq!(
            targets.iter().map(|target| target.tid).collect::<Vec<_>>(),
            vec![10, 11, 14]
        );
    }

    #[test]
    fn cgroup_target_selection_preserves_identity_and_uses_shared_filters() {
        let observation = AutotuneObservation {
            target_root_pid: Some(10),
            active_tasks: vec![
                task(10, TaskClass::VirtualMachine),
                task(11, TaskClass::Helper),
                task(12, TaskClass::Compiler),
                task(13, TaskClass::AudioRealtime),
            ],
            ..AutotuneObservation::default()
        };

        let selection = mutable_cgroup_targets_for_observation_with_mode(
            &observation,
            TaskTargetSelector::VirtualMachineAndHelpers,
            TargetSelectionMode::ApplyCapable,
        );

        assert_eq!(
            selection
                .items
                .iter()
                .map(|target| target.identity.tid)
                .collect::<Vec<_>>(),
            vec![10, 11]
        );
        assert_eq!(
            selection
                .items
                .iter()
                .map(|target| target.class)
                .collect::<Vec<_>>(),
            vec![TaskClass::VirtualMachine, TaskClass::Helper]
        );
        assert_eq!(selection.items[0].identity.process_pid, Some(10));
        assert_eq!(selection.items[0].identity.starttime_ticks, Some(10));
        assert!(!selection.used_fallback_root);
        assert!(!selection.missing_active_snapshots);
    }

    #[test]
    fn apply_capable_selection_does_not_use_fallback_root() {
        let observation = AutotuneObservation {
            target_root_pid: Some(10),
            active_tasks: Vec::new(),
            ..AutotuneObservation::default()
        };

        let selection = mutable_task_targets_for_observation_with_mode(
            &observation,
            TaskTargetSelector::CompilerAndLinker,
            TargetSelectionMode::ApplyCapable,
        );

        assert!(selection.items.is_empty());
        assert!(!selection.used_fallback_root);
        assert!(selection.missing_active_snapshots);
    }

    #[test]
    fn suggest_selection_marks_unknown_fallback_root() {
        let observation = AutotuneObservation {
            target_root_pid: Some(10),
            active_tasks: Vec::new(),
            ..AutotuneObservation::default()
        };

        let selection = mutable_task_snapshots_for_observation_with_mode(
            &observation,
            TaskTargetSelector::CompilerAndLinker,
            TargetSelectionMode::Suggest,
        );

        assert_eq!(selection.items.len(), 1);
        assert!(selection.used_fallback_root);
        assert_eq!(selection.items[0].class, TaskClass::Unknown);
        assert!(
            selection
                .deny_reasons()
                .iter()
                .any(|reason| reason.contains("target_selection_fallback_root"))
        );
    }
}
