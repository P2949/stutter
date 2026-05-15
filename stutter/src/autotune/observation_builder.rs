use std::{collections::BTreeMap, fs, os::unix::fs::MetadataExt, path::Path};

use crate::{
    autotune::{
        observation::{
            ActiveTaskSnapshot, AutotuneObservation, ProtectedTask, WorkloadIdentity,
            is_protected_task_class, protected_task_reason,
        },
        quality::OnlineDataQualityPolicy,
        rolling_window::{RollingWindow, WindowScore as RuntimeWindowScore},
        state::SituationKind,
        system_context::{SystemContextSnapshotInput, collect_system_context},
    },
    diagnosis::LiveDiagnosisEntry,
    ebpf_loader::DropCountersSnapshot,
    focus::FocusGroupKind,
    process_tree::TaskInfo,
    scorer::StutterScore,
};

pub struct AutotuneObservationBuilder;

#[derive(Clone, Debug)]
pub struct AutotuneObservationFocus {
    pub kind: FocusGroupKind,
    pub root_pids: Vec<u32>,
    pub member_pids: Vec<u32>,
    pub confidence: f32,
    pub score: f32,
    pub situation: SituationKind,
    pub reasons: Vec<String>,
}

pub struct AutotuneObservationBuilderInput<'a> {
    pub window: &'a RollingWindow,
    pub online_data_quality_policy: &'a OnlineDataQualityPolicy,
    pub focus: Option<&'a AutotuneObservationFocus>,
    pub root_pid: Option<u32>,
    pub active_target_count: usize,
    pub active_tasks: &'a BTreeMap<u32, TaskInfo>,
    pub recent_diagnoses: Vec<LiveDiagnosisEntry>,
    pub drop_counters: DropCountersSnapshot,
    pub proc_root: &'a Path,
    pub sys_root: &'a Path,
}

#[derive(Clone, Debug)]
pub struct AutotuneObservationBuildOutput {
    pub observation: AutotuneObservation,
}

impl AutotuneObservationBuilder {
    pub fn build(input: AutotuneObservationBuilderInput<'_>) -> AutotuneObservationBuildOutput {
        let window_score = input
            .window
            .score_with_quality_policy(input.online_data_quality_policy);
        let focus = input.focus;
        let focus_kind = focus.map(|focus| focus.kind);
        let focus_confidence = focus.map(|focus| focus.confidence).unwrap_or(0.0);
        let focus_roots = focus
            .map(|focus| focus.root_pids.clone())
            .unwrap_or_default();
        let focus_reasons = focus.map(|focus| focus.reasons.clone()).unwrap_or_default();
        let primary_situation = focus
            .map(|focus| focus.situation)
            .unwrap_or(SituationKind::Unknown);
        let system_health = crate::daemon::evaluate_system_health(
            crate::daemon::SystemHealthInputs {
                ebpf_dropped_events: input.drop_counters.total(),
                ..crate::daemon::SystemHealthInputs::default()
            },
            &crate::daemon::SystemHealthThresholds {
                max_ebpf_dropped_events: input.online_data_quality_policy.max_drop_counter_total,
                ..crate::daemon::SystemHealthThresholds::default()
            },
        );
        let active_tasks =
            active_task_snapshots_from_active_tasks(input.proc_root, input.active_tasks);
        let protected_tasks = protected_tasks_from_active_tasks(input.active_tasks);
        let system_context = collect_system_context(SystemContextSnapshotInput {
            proc_root: input.proc_root,
            sys_root: input.sys_root,
            active_tasks: &active_tasks,
            health: system_health,
            sampled_at_unix_nanos: crate::audit::unix_nanos_now(),
        });
        let capabilities = system_context.capabilities.clone();
        let topology_signature = system_context.inventory.inventory_hash.clone();
        let active_config_snapshot = system_context.active_config.clone();
        let target_root_pid = input.root_pid.or_else(|| focus_roots.first().copied());
        let workload_identity = workload_identity_from_runtime_context(
            input.proc_root,
            target_root_pid,
            focus_kind,
            input.active_tasks,
        );
        let mut observation = AutotuneObservation {
            now_unix_nanos: system_context.sampled_at_unix_nanos,
            elapsed_ms: input.window.latest_elapsed_ms().unwrap_or(0),
            target_present: input.active_target_count > 0 || window_score.scored_samples > 0,
            target_root_pid,
            active_target_count: input.active_target_count,
            scored_task_count: window_score.scored_task_count,
            interval_count: window_score.interval_count,
            scored_samples: window_score.scored_samples,
            score: stutter_score_from_runtime_window_score(&window_score),
            data_quality: window_score.data_quality.clone(),
            objective_signals: input.window.objective_signals(),
            primary_situation,
            situation: Default::default(),
            focus_kind,
            focus_confidence,
            focus_roots,
            focus_reasons,
            recent_diagnoses: input.recent_diagnoses,
            system_health: system_context.health.clone(),
            capabilities,
            topology_signature: Some(topology_signature),
            workload_identity,
            active_tasks,
            protected_tasks,
            active_config_snapshot: Some(active_config_snapshot),
            system_context: Some(system_context),
            frame_count: window_score.frame_count,
            frame_p99_ms: window_score.frame_p99_ms,
            frame_max_ms: window_score.frame_max_ms,
            drop_counter_total: input.drop_counters.total(),
        };
        observation.refresh_situation_classification();

        AutotuneObservationBuildOutput { observation }
    }
}

fn protected_tasks_from_active_tasks(active_tasks: &BTreeMap<u32, TaskInfo>) -> Vec<ProtectedTask> {
    active_tasks
        .values()
        .filter(|task| is_protected_task_class(task.class))
        .map(|task| ProtectedTask {
            tid: task.tid,
            process_pid: task.process_pid,
            comm: task.comm.clone(),
            class: task.class,
            reason: protected_task_reason(task.class).to_owned(),
        })
        .collect()
}

fn active_task_snapshots_from_active_tasks(
    proc_root: &Path,
    active_tasks: &BTreeMap<u32, TaskInfo>,
) -> Vec<ActiveTaskSnapshot> {
    active_tasks
        .values()
        .map(|task| ActiveTaskSnapshot {
            tid: task.tid,
            process_pid: task.process_pid,
            comm: task.comm.clone(),
            class: task.class,
            process_starttime_ticks: task.process_starttime_ticks,
            task_starttime_ticks: task.task_starttime_ticks,
            cgroup_path: read_proc_cgroup_path(proc_root, task.tid),
        })
        .collect()
}

fn workload_identity_from_runtime_context(
    proc_root: &Path,
    root_pid: Option<u32>,
    focus_kind: Option<FocusGroupKind>,
    active_tasks: &BTreeMap<u32, TaskInfo>,
) -> Option<WorkloadIdentity> {
    let root_pid = root_pid?;
    let root_task = active_tasks
        .values()
        .find(|task| task.process_pid == root_pid || task.tid == root_pid);
    let process_starttime_ticks = root_task
        .and_then(|task| task.process_starttime_ticks)
        .or_else(|| crate::process_tree::process_starttime_at(proc_root, root_pid));
    let (exe_dev, exe_ino) = root_task
        .and_then(|task| Some((task.exe_dev?, task.exe_ino?)))
        .map(|(dev, ino)| (Some(dev), Some(ino)))
        .unwrap_or_else(|| exe_identity_for_pid(proc_root, root_pid));
    let cgroup_path = read_proc_cgroup_path(proc_root, root_pid);
    let class_distribution = class_distribution_from_tasks(active_tasks);
    let stable_hash = crate::daemon::state::daemon_profile_stable_hash([
        root_pid.to_string().as_str(),
        process_starttime_ticks
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_owned())
            .as_str(),
        exe_dev
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_owned())
            .as_str(),
        exe_ino
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_owned())
            .as_str(),
        cgroup_path.as_deref().unwrap_or(""),
        &format!("{focus_kind:?}"),
    ]);

    Some(WorkloadIdentity {
        root_pid,
        process_starttime_ticks,
        exe_dev,
        exe_ino,
        cgroup_path,
        focus_kind,
        class_distribution,
        stable_hash,
    })
}

fn exe_identity_for_pid(proc_root: &Path, pid: u32) -> (Option<u64>, Option<u64>) {
    fs::metadata(proc_root.join(pid.to_string()).join("exe"))
        .map(|metadata| (Some(metadata.dev()), Some(metadata.ino())))
        .unwrap_or((None, None))
}

fn read_proc_cgroup_path(proc_root: &Path, pid: u32) -> Option<String> {
    let text = fs::read_to_string(proc_root.join(pid.to_string()).join("cgroup")).ok()?;
    text.lines()
        .filter_map(|line| line.rsplit_once(':').map(|(_, path)| path.trim()))
        .filter(|path| !path.is_empty())
        .max_by_key(|path| path.len())
        .map(str::to_owned)
}

fn class_distribution_from_tasks(
    active_tasks: &BTreeMap<u32, TaskInfo>,
) -> BTreeMap<String, usize> {
    let mut classes = BTreeMap::new();
    for task in active_tasks.values() {
        *classes.entry(task.class.as_str().to_owned()).or_insert(0) += 1;
    }
    classes
}

fn stutter_score_from_runtime_window_score(score: &RuntimeWindowScore) -> StutterScore {
    StutterScore {
        total: score.score_total,
        over_1ms: score.over_1ms,
        over_2ms: score.over_2ms,
        over_5ms: score.over_5ms,
        max_latency_ns: score.max_latency_ns,
        frame_max_ms: score.frame_max_ms,
        frame_p99_ms: score.frame_p99_ms,
        frame_over_16ms: 0,
        frame_over_33ms: 0,
        frame_over_50ms: 0,
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs, path::PathBuf, sync::Arc, time::Duration};

    use super::*;
    use crate::{
        ebpf_loader::DropCountersSnapshot,
        process_tree::TaskClass,
        recorder::{IntervalRecord, IrqEventRecord},
        test_support::TestRoot,
    };

    #[test]
    fn builder_extracts_focus_target_tasks_workload_identity_and_system_context() {
        let (_root, proc_root, sys_root) = temp_proc_sys_roots("observation-builder-context");
        write_cgroup(&proc_root, 1234, "/user.slice/game.scope");
        write_cgroup(&proc_root, 1235, "/user.slice/input.scope");

        let mut window = RollingWindow::new(Duration::from_secs(30));
        window.push_interval(IntervalRecord {
            elapsed_ms: 1_000,
            task: 1234,
            samples: 50,
            over_1ms: 2,
            over_2ms: 1,
            over_5ms: 0,
            max_ns: 2_000_000,
            ..IntervalRecord::default()
        });
        window.push_irq_event(IrqEventRecord {
            elapsed_ms: Some(1_050),
            irq: 44,
            cpu: 2,
            enter_ns: 10,
            exit_ns: 20,
            duration_ns: 10,
        });

        let focus = AutotuneObservationFocus {
            kind: FocusGroupKind::Desktop,
            root_pids: vec![1234],
            member_pids: vec![1234, 1235],
            confidence: 0.95,
            score: 0.99,
            situation: SituationKind::IoPressure,
            reasons: vec!["desktop focus selected".to_owned()],
        };

        let active_tasks = BTreeMap::from([
            (
                1234,
                task_info(1234, 1234, "game-main", TaskClass::Game, Some(10)),
            ),
            (
                1235,
                task_info(1235, 1235, "input", TaskClass::Input, Some(11)),
            ),
        ]);

        let output = AutotuneObservationBuilder::build(AutotuneObservationBuilderInput {
            window: &window,
            online_data_quality_policy: &OnlineDataQualityPolicy::default(),
            focus: Some(&focus),
            root_pid: None,
            active_target_count: active_tasks.len(),
            active_tasks: &active_tasks,
            recent_diagnoses: Vec::new(),
            drop_counters: DropCountersSnapshot::default(),
            proc_root: &proc_root,
            sys_root: &sys_root,
        });
        let observation = output.observation;

        assert_eq!(observation.focus_kind, Some(FocusGroupKind::Desktop));
        assert_eq!(observation.focus_confidence, 0.95);
        assert_eq!(observation.focus_roots, vec![1234]);
        assert_eq!(observation.focus_reasons, vec!["desktop focus selected"]);
        assert_eq!(observation.primary_situation, SituationKind::IoPressure);
        assert_eq!(observation.target_root_pid, Some(1234));
        assert_eq!(observation.active_target_count, 2);
        assert_eq!(observation.active_tasks.len(), 2);
        assert_eq!(
            observation
                .active_tasks
                .iter()
                .find(|task| task.tid == 1234)
                .and_then(|task| task.cgroup_path.as_deref()),
            Some("/user.slice/game.scope")
        );
        assert_eq!(observation.protected_tasks.len(), 1);
        assert_eq!(observation.protected_tasks[0].tid, 1235);
        assert!(observation.workload_identity.is_some());
        assert!(observation.system_context.is_some());
        assert!(observation.active_config_snapshot.is_some());
        assert_eq!(observation.objective_signals.irq_hot_irq, Some(44));
        assert_eq!(observation.drop_counter_total, 0);
    }

    #[test]
    fn builder_uses_configured_online_data_quality_policy() {
        let (_root, proc_root, sys_root) = temp_proc_sys_roots("observation-builder-quality");
        let mut window = RollingWindow::new(Duration::from_secs(30));

        for elapsed_ms in [1_000, 2_000, 3_000, 4_000, 5_000] {
            window.push_interval(IntervalRecord {
                elapsed_ms,
                task: 42,
                samples: 20,
                over_1ms: 1,
                max_ns: 2_000_000,
                ..IntervalRecord::default()
            });
        }

        let active_tasks = BTreeMap::new();
        let output = AutotuneObservationBuilder::build(AutotuneObservationBuilderInput {
            window: &window,
            online_data_quality_policy: &OnlineDataQualityPolicy {
                min_scored_samples: 200,
                ..OnlineDataQualityPolicy::default()
            },
            focus: None,
            root_pid: Some(1234),
            active_target_count: 0,
            active_tasks: &active_tasks,
            recent_diagnoses: Vec::new(),
            drop_counters: DropCountersSnapshot::default(),
            proc_root: &proc_root,
            sys_root: &sys_root,
        });
        let observation = output.observation;

        assert_eq!(observation.scored_samples, 100);
        assert!(observation.data_quality.is_low());
        assert!(
            observation
                .data_quality
                .reasons()
                .iter()
                .any(|reason| reason.contains("fewer than min_scored_samples"))
        );
    }

    fn temp_proc_sys_roots(prefix: &str) -> (TestRoot, PathBuf, PathBuf) {
        let root = TestRoot::new(prefix);
        let proc_root = root.join("proc");
        let sys_root = root.join("sys");
        fs::create_dir_all(&proc_root).unwrap();
        fs::create_dir_all(&sys_root).unwrap();
        (root, proc_root, sys_root)
    }

    fn write_cgroup(proc_root: &std::path::Path, pid: u32, cgroup_path: &str) {
        let task_root = proc_root.join(pid.to_string());
        fs::create_dir_all(&task_root).unwrap();
        fs::write(task_root.join("cgroup"), format!("0::{cgroup_path}\n")).unwrap();
    }

    fn task_info(
        tid: u32,
        process_pid: u32,
        comm: &str,
        class: TaskClass,
        process_starttime_ticks: Option<u64>,
    ) -> TaskInfo {
        TaskInfo {
            tid,
            process_pid,
            process_ppid: 0,
            comm: comm.to_owned(),
            process_comm: Arc::<str>::from(comm),
            process_starttime_ticks,
            task_starttime_ticks: Some(u64::from(tid)),
            exe_dev: None,
            exe_ino: None,
            class,
            sched_policy: None,
            from_cgroup: true,
        }
    }
}
