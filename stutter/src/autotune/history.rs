use std::{
    fs::{self, OpenOptions},
    io::{BufRead, Write},
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use super::experiment::WindowScore;
pub use super::situation::SituationKind;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ControllerPhase {
    Disabled,
    Observing,
    Planning,
    Applying,
    Measuring,
    Keeping,
    Reverting,
    Cooldown,
    Faulted,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AutotuneMode {
    Observe,
    Suggest,
    ApplyLowRisk,
    ApplyMediumRisk,
    ApplyHighRisk,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetIdentity {
    pub root_pid: u32,
    pub process_comm: String,
    pub process_starttime_ticks: Option<u64>,
    pub exe_dev: Option<u64>,
    pub exe_ino: Option<u64>,
    pub active_task_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ObservationSummary {
    pub target_present: bool,
    pub active_target_count: usize,
    pub scored_task_count: usize,
    pub interval_count: usize,
    pub scored_samples: u64,
    pub score_total: u64,
    pub over_1ms: u64,
    pub over_2ms: u64,
    pub over_5ms: u64,
    pub frame_p99_ms: f64,
    pub frame_max_ms: f64,
    pub drop_counter_total: u64,
    pub data_quality: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AutotuneDecisionSummary {
    pub decision: String,
    pub candidate_name: Option<String>,
    pub action_kind: Option<String>,
    pub eligible: bool,
    pub rollback_policy: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AutotuneHistoryEvent {
    pub schema_version: u32,
    pub unix_nanos: u128,
    pub controller_id: String,
    pub phase: ControllerPhase,
    pub mode: AutotuneMode,
    pub target: Option<TargetIdentity>,
    pub situation: SituationKind,
    pub observation_summary: ObservationSummary,
    pub decision: AutotuneDecisionSummary,
    pub experiment_id: Option<String>,
    pub action_id: Option<String>,
    pub score_before: Option<WindowScore>,
    pub score_after: Option<WindowScore>,
    pub rollback_performed: bool,
    pub reason: String,
}

impl AutotuneHistoryEvent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        controller_id: impl Into<String>,
        phase: ControllerPhase,
        mode: AutotuneMode,
        target: Option<TargetIdentity>,
        situation: SituationKind,
        observation_summary: ObservationSummary,
        decision: AutotuneDecisionSummary,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: 1,
            unix_nanos: crate::audit::unix_nanos_now(),
            controller_id: controller_id.into(),
            phase,
            mode,
            target,
            situation,
            observation_summary,
            decision,
            experiment_id: None,
            action_id: None,
            score_before: None,
            score_after: None,
            rollback_performed: false,
            reason: reason.into(),
        }
    }

    pub fn with_experiment_id(mut self, experiment_id: impl Into<String>) -> Self {
        self.experiment_id = Some(experiment_id.into());
        self
    }

    pub fn with_action_id(mut self, action_id: impl Into<String>) -> Self {
        self.action_id = Some(action_id.into());
        self
    }

    pub fn with_scores(
        mut self,
        score_before: Option<WindowScore>,
        score_after: Option<WindowScore>,
    ) -> Self {
        self.score_before = score_before;
        self.score_after = score_after;
        self
    }

    pub fn with_rollback_performed(mut self, rollback_performed: bool) -> Self {
        self.rollback_performed = rollback_performed;
        self
    }
}

pub fn default_autotune_history_path() -> PathBuf {
    let mut path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    path.push(".local");
    path.push("state");
    path.push("stutter");
    path.push("autotune");
    path.push("history.jsonl");
    path
}

pub fn append_autotune_history_event(
    path: &Path,
    event: &AutotuneHistoryEvent,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create autotune history directory {}",
                parent.display()
            )
        })?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open autotune history log {}", path.display()))?;

    serde_json::to_writer(&mut file, event)
        .with_context(|| format!("failed to write autotune history event {}", path.display()))?;
    file.write_all(b"\n").with_context(|| {
        format!(
            "failed to terminate autotune history event {}",
            path.display()
        )
    })?;

    Ok(())
}

pub fn append_autotune_history_event_to_default_path(
    event: &AutotuneHistoryEvent,
) -> anyhow::Result<PathBuf> {
    let path = default_autotune_history_path();
    append_autotune_history_event(&path, event)?;
    Ok(path)
}

pub fn read_autotune_history_events(path: &Path) -> anyhow::Result<Vec<AutotuneHistoryEvent>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let file = fs::File::open(path)
        .with_context(|| format!("failed to open autotune history log {}", path.display()))?;
    let reader = std::io::BufReader::new(file);
    let mut events = Vec::new();

    for line in reader.lines() {
        let line = line
            .with_context(|| format!("failed to read autotune history log {}", path.display()))?;
        if line.trim().is_empty() {
            continue;
        }

        let event = serde_json::from_str::<AutotuneHistoryEvent>(&line).with_context(|| {
            format!("failed to parse autotune history event {}", path.display())
        })?;
        events.push(event);
    }

    Ok(events)
}

pub fn observation_summary_from_window_score(
    target_present: bool,
    active_target_count: usize,
    drop_counter_total: u64,
    data_quality: impl Into<String>,
    score: &WindowScore,
) -> ObservationSummary {
    ObservationSummary {
        target_present,
        active_target_count,
        scored_task_count: score.scored_task_count,
        interval_count: score.interval_count,
        scored_samples: score.scored_samples,
        score_total: score.score.total,
        over_1ms: score.score.over_1ms,
        over_2ms: score.score.over_2ms,
        over_5ms: score.score.over_5ms,
        frame_p99_ms: score.score.frame_p99_ms,
        frame_max_ms: score.score.frame_max_ms,
        drop_counter_total,
        data_quality: data_quality.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scorer::StutterScore;

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-autotune-history-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn window_score(total: u64) -> WindowScore {
        WindowScore {
            started_unix_nanos: 100,
            finished_unix_nanos: 200,
            interval_count: 10,
            scored_samples: 100,
            scored_task_count: 2,
            score: StutterScore {
                total,
                over_1ms: 3,
                over_2ms: 2,
                over_5ms: 1,
                frame_p99_ms: 12.0,
                frame_max_ms: 20.0,
                ..StutterScore::default()
            },
        }
    }

    fn target() -> TargetIdentity {
        TargetIdentity {
            root_pid: 1234,
            process_comm: "Game.exe".to_owned(),
            process_starttime_ticks: Some(99),
            exe_dev: Some(1),
            exe_ino: Some(2),
            active_task_count: 31,
        }
    }

    fn decision() -> AutotuneDecisionSummary {
        AutotuneDecisionSummary {
            decision: "KeepCurrent".to_owned(),
            candidate_name: Some("game-main".to_owned()),
            action_kind: Some("cpu_affinity_profile".to_owned()),
            eligible: true,
            rollback_policy: "rollback-on-exit".to_owned(),
        }
    }

    #[test]
    fn default_history_path_matches_policy() {
        let path = default_autotune_history_path();
        let rendered = path.to_string_lossy();

        assert!(rendered.ends_with(".local/state/stutter/autotune/history.jsonl"));
    }

    #[test]
    fn observation_summary_copies_window_score_fields() {
        let score = window_score(143);

        let summary = observation_summary_from_window_score(true, 31, 0, "High", &score);

        assert!(summary.target_present);
        assert_eq!(summary.active_target_count, 31);
        assert_eq!(summary.scored_task_count, 2);
        assert_eq!(summary.interval_count, 10);
        assert_eq!(summary.scored_samples, 100);
        assert_eq!(summary.score_total, 143);
        assert_eq!(summary.over_1ms, 3);
        assert_eq!(summary.over_2ms, 2);
        assert_eq!(summary.over_5ms, 1);
        assert_eq!(summary.frame_p99_ms, 12.0);
        assert_eq!(summary.frame_max_ms, 20.0);
        assert_eq!(summary.drop_counter_total, 0);
        assert_eq!(summary.data_quality, "High");
    }

    #[test]
    fn history_event_round_trips_as_jsonl() {
        let dir = temp_dir("round-trip");
        let path = dir.join("history.jsonl");
        let before = window_score(1_000);
        let after = window_score(850);
        let event = AutotuneHistoryEvent::new(
            "controller-1",
            ControllerPhase::Cooldown,
            AutotuneMode::ApplyLowRisk,
            Some(target()),
            SituationKind::GameCpuSchedulerPressure,
            observation_summary_from_window_score(true, 31, 0, "High", &after),
            decision(),
            "candidate improved by 15.00%; kept as current active profile",
        )
        .with_experiment_id("experiment-1")
        .with_action_id("cpu-affinity-profile:game-main")
        .with_scores(Some(before), Some(after))
        .with_rollback_performed(false);

        append_autotune_history_event(&path, &event).unwrap();

        let events = read_autotune_history_events(&path).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], event);
        assert_eq!(events[0].schema_version, 1);
        assert_eq!(events[0].controller_id, "controller-1");
        assert_eq!(events[0].phase, ControllerPhase::Cooldown);
        assert_eq!(events[0].mode, AutotuneMode::ApplyLowRisk);
        assert_eq!(
            events[0].target.as_ref().map(|target| target.root_pid),
            Some(1234)
        );
        assert_eq!(events[0].experiment_id.as_deref(), Some("experiment-1"));
        assert_eq!(
            events[0].action_id.as_deref(),
            Some("cpu-affinity-profile:game-main")
        );
        assert_eq!(
            events[0]
                .score_before
                .as_ref()
                .map(|score| score.score.total),
            Some(1_000)
        );
        assert_eq!(
            events[0]
                .score_after
                .as_ref()
                .map(|score| score.score.total),
            Some(850)
        );
        assert!(!events[0].rollback_performed);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn all_situation_kinds_round_trip_through_history_json() {
        let variants = [
            SituationKind::Unknown,
            SituationKind::Idle,
            SituationKind::GameFocused,
            SituationKind::GameCpuSchedulerPressure,
            SituationKind::GameGpuBound,
            SituationKind::CompositorPressure,
            SituationKind::CpuPressure,
            SituationKind::IoPressure,
            SituationKind::IrqPressure,
            SituationKind::ThermalOrPowerLimit,
            SituationKind::CompileLoad,
            SituationKind::BrowserFocused,
            SituationKind::BrowserCpuPressure,
            SituationKind::BrowserGpuVideo,
            SituationKind::BrowserIoPressure,
            SituationKind::CompileCpuBound,
            SituationKind::CompileLinkerPressure,
            SituationKind::MediaPlayback,
            SituationKind::Recording,
            SituationKind::VirtualMachineLoad,
        ];

        for situation in variants {
            let event = AutotuneHistoryEvent::new(
                "controller-1",
                ControllerPhase::Observing,
                AutotuneMode::Observe,
                None,
                situation,
                observation_summary_from_window_score(true, 1, 0, "High", &window_score(143)),
                AutotuneDecisionSummary {
                    decision: "Noop".to_owned(),
                    candidate_name: None,
                    action_kind: None,
                    eligible: false,
                    rollback_policy: "none".to_owned(),
                },
                "observe mode",
            );

            let json = serde_json::to_string(&event).unwrap();
            let parsed: AutotuneHistoryEvent = serde_json::from_str(&json).unwrap();

            assert_eq!(parsed.situation, situation);
        }
    }

    #[test]
    fn browser_focused_does_not_serialize_as_compile_load() {
        let event = AutotuneHistoryEvent::new(
            "controller-1",
            ControllerPhase::Observing,
            AutotuneMode::Observe,
            None,
            SituationKind::BrowserFocused,
            observation_summary_from_window_score(true, 1, 0, "High", &window_score(143)),
            AutotuneDecisionSummary {
                decision: "Noop".to_owned(),
                candidate_name: None,
                action_kind: None,
                eligible: false,
                rollback_policy: "none".to_owned(),
            },
            "observe mode",
        );

        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains("\"situation\":\"BrowserFocused\""));
        assert!(!json.contains("\"situation\":\"CompileLoad\""));
    }

    #[test]
    fn history_reader_ignores_blank_lines() {
        let dir = temp_dir("blank-lines");
        let path = dir.join("history.jsonl");
        let event = AutotuneHistoryEvent::new(
            "controller-1",
            ControllerPhase::Observing,
            AutotuneMode::Observe,
            None,
            SituationKind::Unknown,
            observation_summary_from_window_score(true, 1, 0, "High", &window_score(143)),
            AutotuneDecisionSummary {
                decision: "Noop".to_owned(),
                candidate_name: None,
                action_kind: None,
                eligible: false,
                rollback_policy: "none".to_owned(),
            },
            "observe mode",
        );

        fs::write(&path, "\n").unwrap();
        append_autotune_history_event(&path, &event).unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"\n")
            .unwrap();

        let events = read_autotune_history_events(&path).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0], event);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn reading_missing_history_file_returns_empty_list() {
        let dir = temp_dir("missing");
        let path = dir.join("missing-history.jsonl");

        let events = read_autotune_history_events(&path).unwrap();

        assert!(events.is_empty());
        fs::remove_dir_all(dir).ok();
    }
}
