use crate::{
    actions::TaskIdentity,
    autotune::observation::{ActiveTaskSnapshot, AutotuneObservation, is_protected_task_class},
    process_tree::TaskClass,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TaskTargetSelector {
    ForegroundRootOnly,
    FullTargetTree,
    CompilerAndLinker,
    BackgroundHelpers,
    GameRenderAndWorkers,
    BrowserRenderersAndHelpers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetSelectionMode {
    Suggest,
    ApplyCapable,
}

impl TargetSelectionMode {
    pub fn from_daemon_mode(mode: crate::daemon::DaemonMode) -> Self {
        if mode == crate::daemon::DaemonMode::Suggest {
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

    let items = snapshots
        .into_iter()
        .filter(|task| mutable_task_allowed(task, selector, used_fallback_root))
        .collect();

    MutableTaskSelection {
        items,
        used_fallback_root,
        missing_active_snapshots,
    }
}

fn mutable_task_allowed(
    task: &ActiveTaskSnapshot,
    selector: TaskTargetSelector,
    allow_unknown_fallback_root: bool,
) -> bool {
    if task.tid == 0 || is_protected_task_class(task.class) {
        return false;
    }

    if allow_unknown_fallback_root
        && task.class == TaskClass::Unknown
        && task.tid == task.process_pid
    {
        return true;
    }

    match selector {
        TaskTargetSelector::ForegroundRootOnly => task.tid == task.process_pid,
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
        TaskTargetSelector::BackgroundHelpers => matches!(
            task.class,
            TaskClass::Helper
                | TaskClass::BuildJob
                | TaskClass::Compiler
                | TaskClass::Linker
                | TaskClass::PackageManager
                | TaskClass::Indexer
                | TaskClass::VirtualMachine
        ),
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
        tid: root_pid,
        process_pid: root_pid,
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
        tid: task.tid,
        process_pid: Some(task.process_pid),
        comm: Some(task.comm),
        starttime_ticks: task.task_starttime_ticks.or(task.process_starttime_ticks),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(tid: u32, class: TaskClass) -> ActiveTaskSnapshot {
        ActiveTaskSnapshot {
            tid,
            process_pid: 10,
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
