use std::{collections::BTreeMap, fs, path::PathBuf};

use anyhow::Context;
use serde::Serialize;

use crate::{
    ebpf_loader::DropCountersSnapshot,
    process_tree::TaskInfo,
    recorder::SessionTask,
    session_events::MonitorEvent,
    session_io::{self, ArtifactLoadOptions},
};

pub struct AutotuneReplayInput {
    pub run_dir: PathBuf,
    pub config_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct AutotuneReplayReport {
    pub schema_version: u32,
    pub run_dir: PathBuf,
    pub config_path: PathBuf,
    pub mode: String,
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

pub trait ReplayPolicyEngine {
    fn on_event(&mut self, event: &MonitorEvent) -> anyhow::Result<()>;
    fn finish(self) -> AutotuneReplayReport;
}

pub struct ObserveOnlyReplayPolicy {
    report: AutotuneReplayReport,
}

impl ObserveOnlyReplayPolicy {
    pub fn new(run_dir: PathBuf, config_path: PathBuf) -> Self {
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
            MonitorEvent::DataQualityWarning { message } => {
                self.report.data_quality_warnings += 1;
                self.report.validation_warnings.push(message.clone());
            }
            MonitorEvent::Finished { .. } => {
                self.report.finished_events += 1;
            }
        }
        Ok(())
    }

    fn finish(self) -> AutotuneReplayReport {
        self.report
    }
}

pub fn replay_autotune_events(input: AutotuneReplayInput) -> anyhow::Result<AutotuneReplayReport> {
    let config_text = fs::read_to_string(&input.config_path).with_context(|| {
        format!(
            "failed to read autotune config {}",
            input.config_path.display()
        )
    })?;
    let _config_value: toml::Value = toml::from_str(&config_text).with_context(|| {
        format!(
            "failed to parse autotune config {}",
            input.config_path.display()
        )
    })?;

    let artifacts =
        session_io::load_run_artifacts(&input.run_dir, ArtifactLoadOptions::AUTOTUNE_REPLAY)
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
    report.missing_optional_files = artifacts.validation.missing_optional_files;
    Ok(report)
}

pub fn replay_command(input: AutotuneReplayInput) -> anyhow::Result<()> {
    let report = replay_autotune_events(input)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
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
        process_comm: task.process_comm.clone(),
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
