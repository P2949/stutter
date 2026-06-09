use std::{collections::BTreeMap, fs, os::unix::fs::symlink, path::PathBuf, time::Duration};

use super::*;
use crate::{
    autotune::{
        activity::ActivityLevel, quality::OnlineDataQualityPolicy, rolling_window::RollingWindow,
        state::SituationKind,
    },
    diagnosis::{Confidence, LiveDiagnosisEntry, StutterCause},
    ebpf_loader::DropCountersSnapshot,
    focus::FocusGroupKind,
    process_tree::{TaskClass, TaskInfo},
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
        situation: SituationKind::IoPressure,
        reasons: vec!["desktop focus selected".to_owned()],
    };

    let active_tasks = BTreeMap::from([
        (
            1234.into(),
            task_info(1234, 1234, "game-main", TaskClass::Game, Some(10)),
        ),
        (
            1235.into(),
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
        activity_level: ActivityLevel::Active,
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
            .find(|task| task.tid.as_u32() == 1234)
            .and_then(|task| task.cgroup_path.as_deref()),
        Some("/user.slice/game.scope")
    );
    assert_eq!(observation.protected_tasks.len(), 1);
    assert_eq!(observation.protected_tasks[0].tid.as_u32(), 1235);
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
        recent_diagnoses: vec![LiveDiagnosisEntry {
            elapsed_ms: 1_000,
            cause: StutterCause::GameThreadSchedulerDelay,
            confidence: Confidence::High,
            anchor_class: TaskClass::Game,
            anchor_comm: "game-main".to_owned(),
            evidence: vec!["synthetic runnable latency above 5ms".to_owned()],
        }],
        drop_counters: DropCountersSnapshot::default(),
        proc_root: &proc_root,
        sys_root: &sys_root,
        activity_level: ActivityLevel::Active,
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

#[test]
fn observation_from_game_window_with_high_cpu_latency() {
    let (_root, proc_root, sys_root) = temp_proc_sys_roots("observation-builder-game-cpu-pressure");
    let mut window = RollingWindow::new(Duration::from_secs(30));
    window.push_interval(IntervalRecord {
        elapsed_ms: 1_000,
        task: 4_242,
        samples: 120,
        over_1ms: 30,
        over_2ms: 18,
        over_5ms: 9,
        max_ns: 7_500_000,
        ..IntervalRecord::default()
    });

    let focus = AutotuneObservationFocus {
        kind: FocusGroupKind::Game,
        root_pids: vec![4_242],
        member_pids: vec![4_242],
        confidence: crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE,
        situation: SituationKind::GameCpuSchedulerPressure,
        reasons: vec!["game runnable latency above 5ms".to_owned()],
    };
    let active_tasks = BTreeMap::from([(
        4_242.into(),
        task_info(4_242, 4_242, "game-main", TaskClass::Game, Some(42)),
    )]);

    let observation = AutotuneObservationBuilder::build(AutotuneObservationBuilderInput {
        window: &window,
        online_data_quality_policy: &OnlineDataQualityPolicy::default(),
        focus: Some(&focus),
        root_pid: None,
        active_target_count: active_tasks.len(),
        active_tasks: &active_tasks,
        recent_diagnoses: vec![LiveDiagnosisEntry {
            elapsed_ms: 1_000,
            cause: StutterCause::GameThreadSchedulerDelay,
            confidence: Confidence::High,
            anchor_class: TaskClass::Game,
            anchor_comm: "game-main".to_owned(),
            evidence: vec!["synthetic runnable latency above 5ms".to_owned()],
        }],
        drop_counters: DropCountersSnapshot::default(),
        proc_root: &proc_root,
        sys_root: &sys_root,
        activity_level: ActivityLevel::Active,
    })
    .observation;

    assert!(observation.focus_confidence >= crate::autotune::DEFAULT_MIN_FOCUS_CONFIDENCE);
    assert_eq!(
        observation.primary_situation,
        SituationKind::GameCpuSchedulerPressure
    );
    assert_eq!(observation.score.over_5ms, 9);
}

#[test]
fn observation_preserves_irq_signals() {
    let (_root, proc_root, sys_root) = temp_proc_sys_roots("observation-builder-irq-signals");
    let mut window = RollingWindow::new(Duration::from_secs(30));
    window.push_interval(IntervalRecord {
        elapsed_ms: 1_000,
        task: 1234,
        samples: 100,
        ..IntervalRecord::default()
    });
    window.push_irq_event(IrqEventRecord {
        elapsed_ms: Some(1_010),
        irq: 44,
        cpu: 2,
        enter_ns: 10,
        exit_ns: 3_000_010,
        duration_ns: 3_000_000,
    });
    window.push_irq_event(IrqEventRecord {
        elapsed_ms: Some(1_020),
        irq: 45,
        cpu: 3,
        enter_ns: 20,
        exit_ns: 5_000_020,
        duration_ns: 5_000_000,
    });

    let active_tasks = BTreeMap::new();
    let observation = AutotuneObservationBuilder::build(AutotuneObservationBuilderInput {
        window: &window,
        online_data_quality_policy: &OnlineDataQualityPolicy::default(),
        focus: None,
        root_pid: Some(1234),
        active_target_count: 0,
        active_tasks: &active_tasks,
        recent_diagnoses: Vec::new(),
        drop_counters: DropCountersSnapshot::default(),
        proc_root: &proc_root,
        sys_root: &sys_root,
        activity_level: ActivityLevel::Active,
    })
    .observation;

    assert_eq!(observation.objective_signals.irq_overlap_count, Some(2));
    assert_eq!(
        observation.objective_signals.irq_worst_overlap_ns,
        Some(5_000_000)
    );
    assert_eq!(observation.objective_signals.irq_hot_irq, Some(45));
    assert_eq!(observation.objective_signals.irq_hot_cpu, Some(3));
}

#[test]
fn observation_builder_focus_falls_back_to_unknown_when_confidence_below_threshold() {
    let (_root, proc_root, sys_root) = temp_proc_sys_roots("observation-builder-low-confidence");
    let mut window = RollingWindow::new(Duration::from_secs(30));
    window.push_interval(IntervalRecord {
        elapsed_ms: 1_000,
        task: 9_001,
        samples: 20,
        ..IntervalRecord::default()
    });
    let active_tasks = BTreeMap::from([(
        9_001.into(),
        task_info(9_001, 9_001, "service", TaskClass::Service, Some(9)),
    )]);

    let observation = AutotuneObservationBuilder::build(AutotuneObservationBuilderInput {
        window: &window,
        online_data_quality_policy: &OnlineDataQualityPolicy::default(),
        focus: None,
        root_pid: Some(9_001),
        active_target_count: active_tasks.len(),
        active_tasks: &active_tasks,
        recent_diagnoses: Vec::new(),
        drop_counters: DropCountersSnapshot::default(),
        proc_root: &proc_root,
        sys_root: &sys_root,
        activity_level: ActivityLevel::Active,
    })
    .observation;

    assert!(observation.focus_is_idle_or_unknown());
}

#[test]
fn observation_builder_protected_tasks_exclude_compositor() {
    let (_root, proc_root, sys_root) =
        temp_proc_sys_roots("observation-builder-protected-compositor");
    let mut window = RollingWindow::new(Duration::from_secs(30));
    window.push_interval(IntervalRecord {
        elapsed_ms: 1_000,
        task: 77,
        samples: 20,
        ..IntervalRecord::default()
    });
    let active_tasks = BTreeMap::from([(
        77.into(),
        task_info(77, 77, "kwin_wayland", TaskClass::Compositor, Some(77)),
    )]);

    let observation = AutotuneObservationBuilder::build(AutotuneObservationBuilderInput {
        window: &window,
        online_data_quality_policy: &OnlineDataQualityPolicy::default(),
        focus: None,
        root_pid: Some(77),
        active_target_count: active_tasks.len(),
        active_tasks: &active_tasks,
        recent_diagnoses: Vec::new(),
        drop_counters: DropCountersSnapshot::default(),
        proc_root: &proc_root,
        sys_root: &sys_root,
        activity_level: ActivityLevel::Active,
    })
    .observation;

    assert_eq!(observation.protected_tasks.len(), 1);
    assert_eq!(observation.protected_tasks[0].class, TaskClass::Compositor);
}

#[test]
fn observation_builder_resolves_focused_gpu_from_target_fd() {
    let (_root, proc_root, sys_root) = temp_proc_sys_roots("observation-builder-gpu-focus");
    write_cgroup(&proc_root, 1234, "/game.scope");
    write_drm_device(&sys_root, "card0", "renderD128");
    write_drm_device(&sys_root, "card1", "renderD129");
    let fd_dir = proc_root.join("1234/fd");
    fs::create_dir_all(&fd_dir).unwrap();
    symlink("/dev/dri/renderD129", fd_dir.join("8")).unwrap();

    let mut window = RollingWindow::new(Duration::from_secs(30));
    window.push_interval(IntervalRecord {
        elapsed_ms: 1_000,
        task: 1234,
        samples: 20,
        ..IntervalRecord::default()
    });
    let active_tasks = BTreeMap::from([(
        1234.into(),
        task_info(1234, 1234, "game", TaskClass::Game, Some(1234)),
    )]);

    let observation = AutotuneObservationBuilder::build(AutotuneObservationBuilderInput {
        window: &window,
        online_data_quality_policy: &OnlineDataQualityPolicy::default(),
        focus: None,
        root_pid: Some(1234),
        active_target_count: active_tasks.len(),
        active_tasks: &active_tasks,
        recent_diagnoses: Vec::new(),
        drop_counters: DropCountersSnapshot::default(),
        proc_root: &proc_root,
        sys_root: &sys_root,
        activity_level: ActivityLevel::Active,
    })
    .observation;

    assert_eq!(
        observation
            .objective_signals
            .gpu_active_render_node
            .as_deref(),
        Some("renderD129")
    );
    assert_eq!(
        observation.objective_signals.gpu_drm_card.as_deref(),
        Some("card1")
    );
    assert_eq!(
        observation.objective_signals.gpu_focus_source.as_deref(),
        Some("target_process_fd")
    );
    assert!(
        observation
            .objective_signals
            .gpu_focus_confidence
            .is_some_and(|confidence| confidence >= 0.90)
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

fn write_drm_device(sys_root: &std::path::Path, card: &str, render_node: &str) {
    let device_root = sys_root.join("class/drm").join(card).join("device");
    fs::create_dir_all(device_root.join("drm").join(render_node)).unwrap();
    fs::write(device_root.join("vendor"), "0x1002\n").unwrap();
    fs::write(device_root.join("device"), "0x744c\n").unwrap();
}

fn task_info(
    tid: u32,
    process_pid: u32,
    comm: &str,
    class: TaskClass,
    process_starttime_ticks: Option<u64>,
) -> TaskInfo {
    TaskInfo {
        tid: tid.into(),
        process_pid: process_pid.into(),
        process_ppid: 0.into(),
        comm: comm.to_owned(),
        process_comm: comm.to_owned(),
        process_starttime_ticks,
        task_starttime_ticks: Some(u64::from(tid)),
        exe_dev: None,
        exe_ino: None,
        class,
        sched_policy: None,
        from_cgroup: true,
    }
}
