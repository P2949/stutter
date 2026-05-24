use std::{collections::BTreeMap, fs, path::PathBuf};

use anyhow::Context;
use serde::Serialize;

use crate::{
    artifacts::ArtifactSelection,
    ebpf_loader::DropCountersSnapshot,
    process_tree::{TaskClass, TaskInfo},
    recorder::SessionTask,
    session_events::MonitorEvent,
    session_io,
};

pub struct AutotuneReplayInput {
    pub run_dir: PathBuf,
    pub config_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct AutotuneReplayReport {
    pub schema_version: u32,
    pub run_dir: PathBuf,
    pub config_path: Option<PathBuf>,
    pub mode: String,
    pub decision: AutotuneReplayDecision,
    pub total_events: usize,
    pub target_snapshots: usize,
    pub intervals: usize,
    pub interval_records: usize,
    pub spikes: usize,
    pub frames: usize,
    pub gpu_samples: usize,
    pub irq_events: usize,
    pub io_events: usize,
    pub data_quality_warnings: usize,
    pub finished_events: usize,
    pub validation_warnings: Vec<String>,
    pub missing_optional_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AutotuneReplayDecision {
    pub expected_behavior: String,
    pub action: String,
    pub candidate_profile: Option<String>,
    pub apply_allowed: bool,
    pub reasons: Vec<String>,
}

impl Default for AutotuneReplayDecision {
    fn default() -> Self {
        Self {
            expected_behavior: "observe_only_no_action".to_owned(),
            action: "observe_only".to_owned(),
            candidate_profile: None,
            apply_allowed: false,
            reasons: Vec::new(),
        }
    }
}

pub trait ReplayPolicyEngine {
    fn on_event(&mut self, event: &MonitorEvent) -> anyhow::Result<()>;
    fn finish(self) -> AutotuneReplayReport;
}

pub struct ObserveOnlyReplayPolicy {
    report: AutotuneReplayReport,
}

impl ObserveOnlyReplayPolicy {
    pub fn new(run_dir: PathBuf, config_path: Option<PathBuf>) -> Self {
        Self {
            report: AutotuneReplayReport {
                schema_version: 1,
                run_dir,
                config_path,
                mode: "observe-only-replay".to_owned(),
                ..Default::default()
            },
        }
    }
}

impl ReplayPolicyEngine for ObserveOnlyReplayPolicy {
    fn on_event(&mut self, event: &MonitorEvent) -> anyhow::Result<()> {
        self.report.total_events += 1;
        match event {
            MonitorEvent::TargetSnapshot { .. } => {
                self.report.target_snapshots += 1;
            }
            MonitorEvent::Interval { records, .. } => {
                self.report.intervals += 1;
                self.report.interval_records += records.len();
            }
            MonitorEvent::Spike { .. } => {
                self.report.spikes += 1;
            }
            MonitorEvent::Frame { .. } => {
                self.report.frames += 1;
            }
            MonitorEvent::GpuSample { .. } => {
                self.report.gpu_samples += 1;
            }
            MonitorEvent::IrqEvent { .. } => {
                self.report.irq_events += 1;
            }
            MonitorEvent::IoEvent { .. } => {
                self.report.io_events += 1;
            }
            MonitorEvent::LiveDiagnosis { .. } => {}
            MonitorEvent::FocusChanged { .. } => {}
            MonitorEvent::FocusCleared { .. } => {}
            MonitorEvent::DataQualityWarning { message } => {
                self.report.data_quality_warnings += 1;
                self.report.validation_warnings.push(message.clone());
            }
            MonitorEvent::Finished { .. } => {
                self.report.finished_events += 1;
            }
            MonitorEvent::Alert { .. }
            | MonitorEvent::MigrationEvent { .. }
            | MonitorEvent::CpuFreqSample { .. }
            | MonitorEvent::ForegroundEvent { .. }
            | MonitorEvent::GpuEngineSample { .. }
            | MonitorEvent::SchedulerSample { .. }
            | MonitorEvent::ScxEvent { .. }
            | MonitorEvent::KmsFlipEvent { .. }
            | MonitorEvent::DrmFenceEvent { .. }
            | MonitorEvent::WaylandPresentationEvent { .. }
            | MonitorEvent::DmaBufEvent { .. }
            | MonitorEvent::Exec { .. } => {}
        }
        Ok(())
    }

    fn finish(self) -> AutotuneReplayReport {
        self.report
    }
}

pub fn replay_autotune_events(input: AutotuneReplayInput) -> anyhow::Result<AutotuneReplayReport> {
    if let Some(config_path) = input.config_path.as_ref() {
        let config_text = fs::read_to_string(config_path)
            .with_context(|| format!("failed to read autotune config {}", config_path.display()))?;
        let _config_value: toml::Value = toml::from_str(&config_text).with_context(|| {
            format!("failed to parse autotune config {}", config_path.display())
        })?;
    }

    let artifacts =
        session_io::load_run_artifacts(&input.run_dir, ArtifactSelection::autotune_replay())
            .with_context(|| {
                format!(
                    "failed to load run artifacts from {}",
                    input.run_dir.display()
                )
            })?;

    let mut events = events_from_artifacts(&artifacts);
    events.sort_by_key(|event| event.elapsed_ms().unwrap_or(u64::MAX));

    let mut policy = ObserveOnlyReplayPolicy::new(input.run_dir, input.config_path);

    for warning in &artifacts.validation.warnings {
        policy.on_event(&MonitorEvent::DataQualityWarning {
            message: warning.clone(),
        })?;
    }

    for missing in &artifacts.validation.missing_optional_files {
        policy.on_event(&MonitorEvent::DataQualityWarning {
            message: format!("missing optional artifact: {missing}"),
        })?;
    }

    for event in &events {
        policy.on_event(event)?;
    }

    policy.on_event(&MonitorEvent::Finished {
        reason: artifacts.session.stop_reason.clone(),
    })?;

    let mut report = policy.finish();
    report.decision = decide_replay_behavior(&artifacts);
    report.missing_optional_files = artifacts.validation.missing_optional_files;
    Ok(report)
}

pub fn replay_command(input: AutotuneReplayInput) -> anyhow::Result<()> {
    let report = replay_autotune_events(input)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn decide_replay_behavior(artifacts: &session_io::RunArtifacts) -> AutotuneReplayDecision {
    if is_low_quality_replay(artifacts) {
        return AutotuneReplayDecision {
            expected_behavior: "collect_more_data_no_action".to_owned(),
            action: "collect_more_data".to_owned(),
            candidate_profile: None,
            apply_allowed: false,
            reasons: low_quality_reasons(artifacts),
        };
    }

    if is_gpu_bound_replay(artifacts) {
        return AutotuneReplayDecision {
            expected_behavior: "observe_suggest_investigation_only".to_owned(),
            action: "investigate_non_cpu_bottleneck".to_owned(),
            candidate_profile: None,
            apply_allowed: false,
            reasons: vec![
                "GPU-bound evidence present; CPU affinity should not be auto-applied".to_owned(),
            ],
        };
    }

    if is_game_scheduler_pressure_replay(artifacts) {
        return AutotuneReplayDecision {
            expected_behavior: "suggest_apply_affinity_profile".to_owned(),
            action: "suggest_or_apply_affinity_profile".to_owned(),
            candidate_profile: Some("game-main-suggested".to_owned()),
            apply_allowed: true,
            reasons: vec![
                "game scheduler pressure replay shows runnable latency on game-class tasks"
                    .to_owned(),
            ],
        };
    }

    AutotuneReplayDecision {
        expected_behavior: "observe_only_no_action".to_owned(),
        action: "observe_only".to_owned(),
        candidate_profile: None,
        apply_allowed: false,
        reasons: vec!["no replay evidence strong enough for autotune action".to_owned()],
    }
}

fn is_low_quality_replay(artifacts: &session_io::RunArtifacts) -> bool {
    !artifacts.validation.errors.is_empty()
        || artifacts.session.core.spike_events_truncated
        || artifacts.session.core.spike_events_dropped_count > 0
        || artifacts.session.core.intervals_dropped > 0
        || artifacts.session.core.event_stream_write_errors > 0
        || artifacts.session.core.drop_counters.total() > 0
        || artifacts.intervals.is_empty()
}

fn low_quality_reasons(artifacts: &session_io::RunArtifacts) -> Vec<String> {
    let mut reasons = Vec::new();

    for error in &artifacts.validation.errors {
        reasons.push(format!("validation error: {error}"));
    }

    if artifacts.session.core.spike_events_truncated {
        reasons.push("spike events were truncated".to_owned());
    }

    if artifacts.session.core.spike_events_dropped_count > 0 {
        reasons.push(format!(
            "spike events were dropped: {}",
            artifacts.session.core.spike_events_dropped_count
        ));
    }

    if artifacts.session.core.intervals_dropped > 0 {
        reasons.push(format!(
            "intervals were dropped: {}",
            artifacts.session.core.intervals_dropped
        ));
    }

    if artifacts.session.core.event_stream_write_errors > 0 {
        reasons.push(format!(
            "event stream write errors observed: {}",
            artifacts.session.core.event_stream_write_errors
        ));
    }

    if artifacts.session.core.drop_counters.total() > 0 {
        reasons.push(format!(
            "eBPF drop counters observed: {}",
            artifacts.session.core.drop_counters.total()
        ));
    }

    if artifacts.intervals.is_empty() {
        reasons.push("no interval records available".to_owned());
    }

    if reasons.is_empty() {
        reasons.push("low-quality replay evidence".to_owned());
    }

    reasons
}

fn is_gpu_bound_replay(artifacts: &session_io::RunArtifacts) -> bool {
    let high_gpu_busy = artifacts.gpu_samples.iter().any(|sample| {
        sample
            .gpu_busy_percent
            .is_some_and(|busy_percent| busy_percent >= 95)
    });
    let frame_spike = artifacts
        .frame_events
        .iter()
        .any(|frame| frame.frametime_ms >= 25.0);

    high_gpu_busy && frame_spike && !is_game_scheduler_pressure_replay(artifacts)
}

fn is_game_scheduler_pressure_replay(artifacts: &session_io::RunArtifacts) -> bool {
    artifacts
        .intervals
        .iter()
        .any(interval_is_game_scheduler_pressure)
        || artifacts.spikes.iter().any(|spike| {
            task_class_is_game_like(spike.class) && spike.latency_ns >= 1_000_000 && spike.active
        })
        || artifacts.session.tasks.iter().any(|task| {
            task.active
                && task_class_is_game_like(task.class)
                && (task.latency.over_1ms > 0 || task.latency.max_ns >= 1_000_000)
        })
}

fn interval_is_game_scheduler_pressure(record: &crate::metrics::IntervalRecord) -> bool {
    record.active
        && task_class_is_game_like(record.class)
        && (record.over_1ms > 0 || record.p99_ns >= 1_000_000 || record.max_ns >= 1_000_000)
}

fn task_class_is_game_like(class: TaskClass) -> bool {
    matches!(
        class,
        TaskClass::Game
            | TaskClass::GameHelper
            | TaskClass::GameRenderThread
            | TaskClass::GameWorkerThread
            | TaskClass::WineServer
            | TaskClass::GameScope
    )
}

fn events_from_artifacts(artifacts: &session_io::RunArtifacts) -> Vec<MonitorEvent> {
    let mut events = Vec::new();

    events.push(MonitorEvent::TargetSnapshot {
        elapsed_ms: artifacts.session.core.duration_ms,
        active_targets: active_targets_from_session_tasks(&artifacts.session.tasks),
        removed_targets: removed_targets_from_session_tasks(&artifacts.session.tasks),
    });

    for record in &artifacts.intervals {
        events.push(MonitorEvent::Interval {
            elapsed_ms: record.elapsed_ms,
            records: vec![record.clone()],
            drop_counters: artifacts.session.core.drop_counters.clone(),
        });
    }

    for event in &artifacts.spikes {
        events.push(MonitorEvent::Spike {
            event: Box::new(event.clone()),
        });
    }

    for event in &artifacts.frame_events {
        events.push(MonitorEvent::Frame {
            event: Box::new(event.clone()),
        });
    }

    for sample in &artifacts.gpu_samples {
        events.push(MonitorEvent::GpuSample {
            sample: Box::new(sample.clone()),
        });
    }

    for event in &artifacts.irq_events {
        events.push(MonitorEvent::IrqEvent {
            event: Box::new(event.clone()),
        });
    }

    for event in &artifacts.block_io_events {
        events.push(MonitorEvent::IoEvent {
            event: Box::new(event.clone()),
        });
    }

    events.push(MonitorEvent::Interval {
        elapsed_ms: artifacts.session.core.duration_ms,
        records: Vec::new(),
        drop_counters: final_drop_counters(&artifacts.session.core.drop_counters),
    });

    events
}

fn final_drop_counters(drop_counters: &DropCountersSnapshot) -> DropCountersSnapshot {
    drop_counters.clone()
}

fn active_targets_from_session_tasks(tasks: &[SessionTask]) -> BTreeMap<u32, TaskInfo> {
    tasks
        .iter()
        .filter(|task| task.active)
        .map(|task| (task.task, task_info_from_session_task(task)))
        .collect()
}

fn removed_targets_from_session_tasks(tasks: &[SessionTask]) -> Vec<u32> {
    tasks
        .iter()
        .filter(|task| !task.active)
        .map(|task| task.task)
        .collect()
}

fn task_info_from_session_task(task: &SessionTask) -> TaskInfo {
    TaskInfo {
        tid: task.task,
        process_pid: task.process_pid.unwrap_or(task.task),
        process_ppid: 0,
        comm: task.comm.clone(),
        process_comm: task.process_comm.clone().into(),
        process_starttime_ticks: task.process_starttime_ticks,
        task_starttime_ticks: task.task_starttime_ticks,
        exe_dev: task.exe_dev,
        exe_ino: task.exe_ino,
        class: task.class,
        sched_policy: task
            .sched_policy
            .as_deref()
            .and_then(recorded_sched_policy_to_raw)
            .map(|p| p as u32),
        from_cgroup: false,
    }
}

fn recorded_sched_policy_to_raw(policy: &str) -> Option<i32> {
    match policy {
        "SCHED_OTHER" | "OTHER" => Some(libc::SCHED_OTHER),
        "SCHED_FIFO" | "FIFO" => Some(libc::SCHED_FIFO),
        "SCHED_RR" | "RR" => Some(libc::SCHED_RR),
        "SCHED_BATCH" | "BATCH" => Some(libc::SCHED_BATCH),
        "SCHED_IDLE" | "IDLE" => Some(libc::SCHED_IDLE),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        process_tree::TaskClass,
        recorder::{RecordedCpuSnapshot, RecordedLatency, SessionTask},
        test_fixture_builder,
    };

    fn session_task(task: u32, active: bool, class: TaskClass) -> SessionTask {
        SessionTask {
            task,
            active,
            first_seen_ms: 0,
            last_seen_ms: 100,
            removed_ms: if active { None } else { Some(100) },
            class,
            process_pid: Some(42),
            process_comm: "Game.exe".into(),
            process_starttime_ticks: Some(420),
            task_starttime_ticks: Some(u64::from(task) * 10),
            exe_dev: Some(1),
            exe_ino: Some(2),
            comm: format!("task-{task}"),
            latency: RecordedLatency::default(),
            cpu: RecordedCpuSnapshot::default(),
            top_spikes: Vec::new(),
            migration_count: 0,
            cross_numa_migrations: 0,
            top_wakers: Vec::new(),
            sched_policy: Some("SCHED_OTHER".to_owned()),
            stat_wait_sum_ns: None,
            stat_wait_sum_ns_saturated: false,
            stat_wait_count: None,
            cpu_perf: None,
        }
    }

    fn replay_fixture_root(name: &str) -> PathBuf {
        let mut root = std::env::temp_dir();
        root.push(format!(
            "stutter-autotune-replay-fixtures-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        std::fs::create_dir_all(&root).unwrap();
        test_fixture_builder::write_autotune_replay_corpus(&root).unwrap();
        root
    }

    #[test]
    fn replay_game_scheduler_pressure_suggests_or_applies_affinity_profile() {
        let root = replay_fixture_root("game-scheduler-pressure");
        let run_dir = root.join("game_scheduler_pressure");

        let report = replay_autotune_events(AutotuneReplayInput {
            run_dir,
            config_path: None,
        })
        .unwrap();

        assert_eq!(
            report.decision.expected_behavior,
            "suggest_apply_affinity_profile"
        );
        assert_eq!(report.decision.action, "suggest_or_apply_affinity_profile");
        assert_eq!(
            report.decision.candidate_profile.as_deref(),
            Some("game-main-suggested")
        );
        assert!(report.decision.apply_allowed);
        assert!(
            report
                .decision
                .reasons
                .iter()
                .any(|reason| reason.contains("game scheduler pressure")),
            "unexpected reasons: {:?}",
            report.decision.reasons
        );

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn replay_gpu_bound_observes_and_suggests_investigation_only() {
        let root = replay_fixture_root("gpu-bound");
        let run_dir = root.join("gpu_bound");

        let report = replay_autotune_events(AutotuneReplayInput {
            run_dir,
            config_path: None,
        })
        .unwrap();

        assert_eq!(
            report.decision.expected_behavior,
            "observe_suggest_investigation_only"
        );
        assert_eq!(report.decision.action, "investigate_non_cpu_bottleneck");
        assert_eq!(report.decision.candidate_profile, None);
        assert!(!report.decision.apply_allowed);
        assert!(
            report
                .decision
                .reasons
                .iter()
                .any(|reason| reason.contains("GPU-bound evidence")),
            "unexpected reasons: {:?}",
            report.decision.reasons
        );

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn replay_low_quality_collects_more_data_and_takes_no_action() {
        let root = replay_fixture_root("low-quality");
        let run_dir = root.join("low_quality");

        let report = replay_autotune_events(AutotuneReplayInput {
            run_dir,
            config_path: None,
        })
        .unwrap();

        assert_eq!(
            report.decision.expected_behavior,
            "collect_more_data_no_action"
        );
        assert_eq!(report.decision.action, "collect_more_data");
        assert_eq!(report.decision.candidate_profile, None);
        assert!(!report.decision.apply_allowed);
        assert!(
            report
                .decision
                .reasons
                .iter()
                .any(|reason| reason.contains("drop counters") || reason.contains("truncated")),
            "unexpected reasons: {:?}",
            report.decision.reasons
        );

        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn session_tasks_convert_to_target_snapshot_parts() {
        let tasks = vec![
            session_task(7, true, TaskClass::Game),
            session_task(8, false, TaskClass::Render),
        ];

        let active = active_targets_from_session_tasks(&tasks);
        let removed = removed_targets_from_session_tasks(&tasks);

        assert_eq!(active.len(), 1);
        assert!(active.contains_key(&7));
        assert_eq!(active.get(&7).unwrap().process_pid, 42);
        assert_eq!(active.get(&7).unwrap().comm, "task-7");
        assert_eq!(removed, vec![8]);
    }
}
