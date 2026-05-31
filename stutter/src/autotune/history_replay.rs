use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::history::{AutotuneHistoryEvent, ControllerPhase, read_autotune_history_events};

#[derive(Clone, Debug)]
pub struct AutotuneReplayHistoryCommandInput {
    pub history_path: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplayTimelineEntry {
    pub offset_seconds: u64,
    pub text: String,
}

pub fn autotune_replay_history_command(
    input: AutotuneReplayHistoryCommandInput,
) -> anyhow::Result<()> {
    let events = read_autotune_history_events(&input.history_path)?;
    let entries = replay_timeline_from_history_events(&events);

    if entries.is_empty() {
        println!(
            "no autotune history events found in {}",
            input.history_path.display()
        );
        return Ok(());
    }

    print!("{}", render_replay_timeline(&entries));
    Ok(())
}

pub fn replay_timeline_from_history_events(
    events: &[AutotuneHistoryEvent],
) -> Vec<ReplayTimelineEntry> {
    let mut sorted = events.to_vec();
    sorted.sort_by_key(|event| event.unix_nanos);

    let Some(first) = sorted.first() else {
        return Vec::new();
    };
    let start_unix_nanos = first.unix_nanos;
    let mut entries = Vec::new();

    for event in &sorted {
        let offset_seconds = event
            .unix_nanos
            .saturating_sub(start_unix_nanos)
            .checked_div(1_000_000_000)
            .unwrap_or(0)
            .min(u64::MAX as u128) as u64;

        for text in event_to_timeline_text(event) {
            push_deduped_entry(
                &mut entries,
                ReplayTimelineEntry {
                    offset_seconds,
                    text,
                },
            );
        }
    }

    entries
}

pub fn render_replay_timeline(entries: &[ReplayTimelineEntry]) -> String {
    let mut rendered = String::new();

    for entry in entries {
        rendered.push_str(&format!(
            "{} {}\n",
            format_offset(entry.offset_seconds),
            entry.text
        ));
    }

    rendered
}

fn event_to_timeline_text(event: &AutotuneHistoryEvent) -> Vec<String> {
    let mut out = Vec::new();
    let decision = event.decision.decision.as_str();
    let reason_lower = event.reason.clone().to_ascii_lowercase();

    if event.phase == ControllerPhase::Observing && reason_lower.contains("baseline") {
        out.push("observing baseline".to_owned());
    }

    if event.phase == ControllerPhase::Applying || decision == "StartExperiment" {
        let candidate = candidate_name(event);
        out.push(format!("candidate {candidate} applied"));
    }

    if reason_lower.contains("washout complete") {
        out.push("washout complete".to_owned());
    }

    if is_improved_event(event)
        && let (Some(before), Some(after)) = (&event.score_before, &event.score_after)
    {
        out.push(format!(
            "candidate improved score {} → {}",
            before.score.total, after.score.total
        ));
    }

    if is_kept_event(event) {
        out.push("kept candidate".to_owned());
    }

    if event.rollback_performed || decision == "Revert" {
        out.push("rolled back candidate".to_owned());
    }

    if reason_lower.contains("cooldown complete") {
        out.push("cooldown complete".to_owned());
    }

    if out.is_empty() {
        out.push(fallback_timeline_text(event));
    }

    out
}

fn fallback_timeline_text(event: &AutotuneHistoryEvent) -> String {
    let decision = humanize_decision(&event.decision.decision);
    if event.reason.clone().trim().is_empty() {
        decision
    } else {
        format!("{decision}: {}", event.reason.clone())
    }
}

fn candidate_name(event: &AutotuneHistoryEvent) -> String {
    event
        .decision
        .candidate_name
        .clone()
        .or_else(|| event.action_id.as_ref().map(|id| id.as_str().to_owned()))
        .unwrap_or_else(|| "unknown".to_owned())
}

fn is_improved_event(event: &AutotuneHistoryEvent) -> bool {
    let decision = event.decision.decision.as_str();
    if matches!(decision, "Improved" | "KeepCurrent" | "Kept" | "Keep") {
        return true;
    }

    let reason = event.reason.clone().to_ascii_lowercase();
    reason.contains("improved")
}

fn is_kept_event(event: &AutotuneHistoryEvent) -> bool {
    matches!(
        event.decision.decision.as_str(),
        "KeepCurrent" | "Kept" | "Keep"
    ) || event.reason.clone().to_ascii_lowercase().contains("kept")
}

fn push_deduped_entry(entries: &mut Vec<ReplayTimelineEntry>, entry: ReplayTimelineEntry) {
    if entries
        .last()
        .map(|last| last.offset_seconds == entry.offset_seconds && last.text == entry.text)
        .unwrap_or(false)
    {
        return;
    }

    entries.push(entry);
}

fn format_offset(seconds: u64) -> String {
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}

fn humanize_decision(decision: &str) -> String {
    let mut out = String::new();

    for (idx, ch) in decision.chars().enumerate() {
        if idx > 0 && ch.is_uppercase() {
            out.push(' ');
        }
        out.push(ch.to_ascii_lowercase());
    }

    out
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write};

    use super::*;
    use crate::{
        autotune::history::{
            AutotuneDecisionSummary, AutotuneMode, ObservationSummary, SituationKind,
            TargetIdentity,
        },
        scorer::StutterScore,
    };

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-autotune-history-replay-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn score(total: u64) -> crate::autotune::experiment::WindowScore {
        crate::autotune::experiment::WindowScore {
            started_unix_nanos: 100,
            finished_unix_nanos: 200,
            interval_count: 10,
            scored_samples: 100,
            scored_task_count: 2,
            score: StutterScore {
                total,
                ..StutterScore::default()
            },
        }
    }

    fn observation(diagnostic_raw_score_total: u64) -> ObservationSummary {
        ObservationSummary {
            target_present: true,
            active_target_count: 31,
            scored_task_count: 2,
            interval_count: 10,
            scored_samples: 100,
            diagnostic_raw_score_total,
            over_1ms: 0,
            over_2ms: 0,
            over_5ms: 0,
            frame_p99_ms: 12.0,
            frame_max_ms: 20.0,
            drop_counter_total: 0,
            data_quality: "High".to_owned(),
        }
    }

    fn target() -> TargetIdentity {
        TargetIdentity {
            root_pid: 1234,
            process_comm: "KingdomCome.exe".to_owned(),
            process_starttime_ticks: Some(99),
            exe_dev: Some(1),
            exe_ino: Some(2),
            active_task_count: 31,
        }
    }

    fn event(
        unix_seconds: u64,
        phase: ControllerPhase,
        decision: &str,
        candidate: Option<&str>,
        reason: &str,
    ) -> AutotuneHistoryEvent {
        AutotuneHistoryEvent {
            schema_version: 1,
            unix_nanos: unix_seconds as u128 * 1_000_000_000,
            controller_id: "controller-1".to_owned(),
            phase,
            mode: AutotuneMode::ApplyLowRisk,
            target: Some(target()),
            situation: SituationKind::GameCpuSchedulerPressure,
            observation_summary: observation(301),
            decision: AutotuneDecisionSummary {
                decision: decision.to_owned(),
                candidate_name: candidate.map(str::to_owned),
                action_kind: Some("cpu_affinity_profile".to_owned()),
                safety_class: candidate.map(|_| crate::actions::SafetyClass::ReversibleLowRisk),
                eligible: true,
                rollback_policy: "rollback-on-exit".to_owned(),
            },
            experiment_id: Some("experiment-1".into()),
            action_id: candidate.map(|name| format!("cpu-affinity-profile:{name}").into()),
            score_before: None,
            score_after: None,
            planner: None,
            rollback_performed: false,
            reason: reason.to_owned(),
        }
    }

    #[test]
    fn timeline_matches_expected_autotune_flow() {
        let mut improved = event(
            70,
            ControllerPhase::Measuring,
            "KeepCurrent",
            Some("game-main-suggested"),
            "candidate improved by 26.94%; kept as current active profile",
        );
        improved.score_before = Some(score(412));
        improved.score_after = Some(score(301));

        let events = vec![
            event(
                0,
                ControllerPhase::Observing,
                "Noop",
                None,
                "observing baseline",
            ),
            event(
                30,
                ControllerPhase::Applying,
                "StartExperiment",
                Some("game-main-suggested"),
                "candidate applied",
            ),
            event(
                40,
                ControllerPhase::Measuring,
                "Noop",
                Some("game-main-suggested"),
                "washout complete",
            ),
            improved,
            event(
                71,
                ControllerPhase::Cooldown,
                "KeepCurrent",
                Some("game-main-suggested"),
                "kept candidate",
            ),
            event(
                131,
                ControllerPhase::Cooldown,
                "Noop",
                Some("game-main-suggested"),
                "cooldown complete",
            ),
        ];

        let timeline = replay_timeline_from_history_events(&events);
        let rendered = render_replay_timeline(&timeline);

        assert!(rendered.contains("00:00 observing baseline"));
        assert!(rendered.contains("00:30 candidate game-main-suggested applied"));
        assert!(rendered.contains("00:40 washout complete"));
        assert!(rendered.contains("01:10 candidate improved score 412 → 301"));
        assert!(rendered.contains("01:10 kept candidate"));
        assert!(rendered.contains("01:11 kept candidate"));
        assert!(rendered.contains("02:11 cooldown complete"));
    }

    #[test]
    fn replay_sorts_events_by_unix_nanos_before_rendering() {
        let events = vec![
            event(
                30,
                ControllerPhase::Applying,
                "StartExperiment",
                Some("game-main-suggested"),
                "candidate applied",
            ),
            event(
                0,
                ControllerPhase::Observing,
                "Noop",
                None,
                "observing baseline",
            ),
        ];

        let rendered = render_replay_timeline(&replay_timeline_from_history_events(&events));

        let observing_index = rendered.find("00:00 observing baseline").unwrap();
        let applied_index = rendered
            .find("00:30 candidate game-main-suggested applied")
            .unwrap();
        assert!(observing_index < applied_index);
    }

    #[test]
    fn rollback_event_renders_rolled_back_candidate() {
        let mut rollback = event(
            90,
            ControllerPhase::Reverting,
            "Revert",
            Some("game-main-suggested"),
            "candidate regressed; rollback performed",
        );
        rollback.rollback_performed = true;

        let rendered = render_replay_timeline(&replay_timeline_from_history_events(&[rollback]));

        assert!(rendered.contains("00:00 rolled back candidate"));
    }

    #[test]
    fn fallback_renders_humanized_decision_and_reason() {
        let event = event(
            0,
            ControllerPhase::Planning,
            "Suggest",
            Some("game-main-suggested"),
            "scheduler pressure detected",
        );

        let rendered = render_replay_timeline(&replay_timeline_from_history_events(&[event]));

        assert!(rendered.contains("00:00 suggest: scheduler pressure detected"));
    }

    #[test]
    fn command_reads_jsonl_file_and_builds_timeline() {
        let dir = temp_dir("command");
        let path = dir.join("history.jsonl");
        let first = event(
            0,
            ControllerPhase::Observing,
            "Noop",
            None,
            "observing baseline",
        );
        let second = event(
            30,
            ControllerPhase::Applying,
            "StartExperiment",
            Some("game-main-suggested"),
            "candidate applied",
        );
        let mut file = fs::File::create(&path).unwrap();
        serde_json::to_writer(&mut file, &first).unwrap();
        file.write_all(b"\n").unwrap();
        serde_json::to_writer(&mut file, &second).unwrap();
        file.write_all(b"\n").unwrap();

        let events = read_autotune_history_events(&path).unwrap();
        let rendered = render_replay_timeline(&replay_timeline_from_history_events(&events));

        assert!(rendered.contains("00:00 observing baseline"));
        assert!(rendered.contains("00:30 candidate game-main-suggested applied"));

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn empty_history_renders_no_entries() {
        let entries = replay_timeline_from_history_events(&[]);

        assert!(entries.is_empty());
        assert_eq!(render_replay_timeline(&entries), "");
    }
}
