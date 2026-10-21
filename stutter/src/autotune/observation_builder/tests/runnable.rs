use super::*;
use crate::{autotune::objective::ObjectiveSignalQuality, metrics::LatencyHistogram};

#[test]
fn builder_scopes_runnable_histograms_to_the_selected_process_tree() {
    let (_root, proc_root, sys_root) = temp_proc_sys_roots("runnable-target-scope");
    let mut window = RollingWindow::default();
    let mut active_tasks = BTreeMap::new();
    for (pid, parent, latency) in [
        (42, 0, 1_000_000),
        (43, 42, 1_000_000),
        (44, 43, 1_000_000),
        (99, 0, 4_000_000),
    ] {
        let mut histogram = LatencyHistogram::new();
        for _ in 0..1_000 {
            histogram.record(latency);
        }
        window.push_interval(IntervalRecord {
            elapsed_ms: 1_000,
            task: pid,
            process_pid: Some(pid),
            class: TaskClass::Game,
            samples: 1_000,
            histogram: histogram.snapshot(),
            max_ns: latency,
            over_1ms: if latency > 1_000_000 { 1_000 } else { 0 },
            over_2ms: if latency > 2_000_000 { 1_000 } else { 0 },
            ..IntervalRecord::default()
        });
        let mut task = task_info(pid, pid, "game", TaskClass::Game, Some(u64::from(pid)));
        task.process_ppid = parent.into();
        active_tasks.insert(pid.into(), task);
    }
    let focus = AutotuneObservationFocus {
        kind: FocusGroupKind::Game,
        root_pids: vec![99],
        member_pids: vec![99],
        confidence: 0.95,
        situation: SituationKind::GameCpuSchedulerPressure,
        reasons: Vec::new(),
    };
    for (root_pid, focus, expected_samples, expected_p99) in [
        (Some(42), Some(&focus), Some(3_000), Some(1_000_000)),
        (None, Some(&focus), Some(1_000), Some(4_000_000)),
        (None, None, None, None),
    ] {
        let observation = AutotuneObservationBuilder::build(AutotuneObservationBuilderInput {
            window: &window,
            online_data_quality_policy: &OnlineDataQualityPolicy::default(),
            focus,
            root_pid,
            active_target_count: active_tasks.len(),
            active_tasks: &active_tasks,
            recent_diagnoses: Vec::new(),
            drop_counters: DropCountersSnapshot::default(),
            proc_root: &proc_root,
            sys_root: &sys_root,
            activity_level: ActivityLevel::Active,
        })
        .observation;
        let signals = observation.objective_signals;
        assert_eq!(signals.runnable_samples, expected_samples);
        assert_eq!(signals.runnable_latency_p99_ns, expected_p99);
        assert_eq!(
            signals.runnable_target_root_pid,
            root_pid.or_else(|| focus.map(|_| 99))
        );
        assert_eq!(
            signals.signal_quality.runnable_latency,
            if expected_samples.is_some() {
                ObjectiveSignalQuality::Direct
            } else {
                ObjectiveSignalQuality::Missing
            }
        );
    }
    // Retained historical samples must not turn an exited root into a current
    // runnable population, even while the generic rolling score is nonempty.
    let empty_tasks = BTreeMap::new();
    let observation = AutotuneObservationBuilder::build(AutotuneObservationBuilderInput {
        window: &window,
        online_data_quality_policy: &OnlineDataQualityPolicy::default(),
        focus: None,
        root_pid: Some(42),
        active_target_count: 0,
        active_tasks: &empty_tasks,
        recent_diagnoses: Vec::new(),
        drop_counters: DropCountersSnapshot::default(),
        proc_root: &proc_root,
        sys_root: &sys_root,
        activity_level: ActivityLevel::Active,
    })
    .observation;
    assert!(observation.scored_samples > 0);
    assert_eq!(observation.objective_signals.runnable_samples, None);
    assert_eq!(
        observation
            .objective_signals
            .signal_quality
            .runnable_latency,
        ObjectiveSignalQuality::Missing
    );
}
