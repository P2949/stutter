use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::autotune::{
    controller_journal::{
        ControllerJournalRecord, default_controller_journal_path, read_controller_journal,
    },
    history::{
        AutotuneHistoryEvent, ControllerPhase, default_autotune_history_path,
        read_autotune_history_events,
    },
};

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct AutotuneTuiPanelSnapshot {
    pub mode: String,
    pub phase: String,
    pub current_profile: Option<String>,
    pub baseline_score: Option<u64>,
    pub candidate_score: Option<u64>,
    pub decision_in_seconds: Option<u64>,
    pub rollback_available: bool,
    pub history_path: PathBuf,
    pub journal_path: PathBuf,
    pub warning: Option<String>,
}

impl AutotuneTuiPanelSnapshot {
    pub fn disabled(history_path: PathBuf, journal_path: PathBuf) -> Self {
        Self {
            mode: "Observe".to_owned(),
            phase: "Disabled".to_owned(),
            current_profile: None,
            baseline_score: None,
            candidate_score: None,
            decision_in_seconds: None,
            rollback_available: false,
            history_path,
            journal_path,
            warning: None,
        }
    }
}

pub fn load_default_autotune_tui_panel_snapshot() -> AutotuneTuiPanelSnapshot {
    let history_path = default_autotune_history_path();
    let journal_path = default_controller_journal_path();
    load_autotune_tui_panel_snapshot(&history_path, &journal_path, crate::audit::unix_nanos_now())
}

pub fn load_autotune_tui_panel_snapshot(
    history_path: &Path,
    journal_path: &Path,
    now_unix_nanos: u128,
) -> AutotuneTuiPanelSnapshot {
    let mut warning = None;

    let history_events = match read_autotune_history_events(history_path) {
        Ok(events) => events,
        Err(err) => {
            warning = Some(format!(
                "failed to read autotune history {}: {err:#}",
                history_path.display()
            ));
            Vec::new()
        }
    };

    let journal_record = match read_controller_journal(journal_path) {
        Ok(record) => Some(record),
        Err(err) => {
            let message = format!(
                "failed to read autotune controller journal {}: {err:#}",
                journal_path.display()
            );
            warning = Some(match warning {
                Some(existing) => format!("{existing}; {message}"),
                None => message,
            });
            None
        }
    };

    let Some(last) = history_events.last() else {
        let mut snapshot = AutotuneTuiPanelSnapshot::disabled(
            history_path.to_path_buf(),
            journal_path.to_path_buf(),
        );
        snapshot.current_profile = current_profile_from_journal(journal_record.as_ref());
        snapshot.rollback_available = journal_record_has_active_rollback(journal_record.as_ref());
        snapshot.warning = warning;
        return snapshot;
    };

    AutotuneTuiPanelSnapshot {
        mode: format!("{:?}", last.mode),
        phase: format!("{:?}", last.phase),
        current_profile: current_profile_from_events(&history_events)
            .or_else(|| current_profile_from_journal(journal_record.as_ref())),
        baseline_score: latest_baseline_score(&history_events),
        candidate_score: latest_candidate_score(&history_events),
        decision_in_seconds: decision_in_seconds_from_event(last, now_unix_nanos),
        rollback_available: rollback_available_from_events(&history_events)
            || journal_record_has_active_rollback(journal_record.as_ref()),
        history_path: history_path.to_path_buf(),
        journal_path: journal_path.to_path_buf(),
        warning,
    }
}

fn current_profile_from_events(events: &[AutotuneHistoryEvent]) -> Option<String> {
    for event in events.iter().rev() {
        if event.rollback_performed {
            return None;
        }

        if let Some(candidate) = event.decision.candidate_name.as_ref()
            && !candidate.trim().is_empty()
        {
            return Some(candidate.clone());
        }

        if let Some(candidate) = candidate_name_from_action_id(event.action_id.as_deref()) {
            return Some(candidate);
        }
    }

    None
}

fn current_profile_from_journal(record: Option<&ControllerJournalRecord>) -> Option<String> {
    record
        .filter(|record| record.is_active_experiment_state())
        .and_then(|record| candidate_name_from_action_id(record.action_id.as_deref()))
}

fn latest_baseline_score(events: &[AutotuneHistoryEvent]) -> Option<u64> {
    for event in events.iter().rev() {
        if let Some(score) = event.score_before.as_ref() {
            return Some(score.score.total);
        }
    }

    None
}

fn latest_candidate_score(events: &[AutotuneHistoryEvent]) -> Option<u64> {
    for event in events.iter().rev() {
        if let Some(score) = event.score_after.as_ref() {
            return Some(score.score.total);
        }

        if event.observation_summary.score_total > 0 {
            return Some(event.observation_summary.score_total);
        }
    }

    None
}

fn rollback_available_from_events(events: &[AutotuneHistoryEvent]) -> bool {
    for event in events.iter().rev() {
        if event.rollback_performed {
            return false;
        }

        if event.action_id.is_some() && event.decision.rollback_policy.contains("rollback") {
            return true;
        }
    }

    false
}

fn journal_record_has_active_rollback(record: Option<&ControllerJournalRecord>) -> bool {
    record.is_some_and(|record| {
        record.is_active_experiment_state() && record.rollback_token().is_some()
    })
}

fn decision_in_seconds_from_event(
    event: &AutotuneHistoryEvent,
    now_unix_nanos: u128,
) -> Option<u64> {
    if let Some(deadline) = deadline_from_scores(event) {
        if now_unix_nanos >= deadline {
            return Some(0);
        }

        return Some(nanos_to_ceil_seconds(
            deadline.saturating_sub(now_unix_nanos),
        ));
    }

    if let Some(remaining_ms) = parse_u64_after_key(&event.reason, "remaining_ms=") {
        return Some(ms_to_ceil_seconds(remaining_ms));
    }

    if let (Some(elapsed_ms), Some(required_ms)) = (
        parse_u64_after_key(&event.reason, "elapsed_ms="),
        parse_u64_after_key(&event.reason, "required_ms="),
    ) {
        return Some(ms_to_ceil_seconds(required_ms.saturating_sub(elapsed_ms)));
    }

    if matches!(event.phase, ControllerPhase::Measuring) {
        return Some(30);
    }

    None
}

fn deadline_from_scores(event: &AutotuneHistoryEvent) -> Option<u128> {
    let score = event.score_after.as_ref().or(event.score_before.as_ref())?;

    if matches!(event.phase, ControllerPhase::Measuring) {
        return Some(score.finished_unix_nanos);
    }

    None
}

fn parse_u64_after_key(text: &str, key: &str) -> Option<u64> {
    let (_, tail) = text.split_once(key)?;
    let digits = tail
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();

    if digits.is_empty() {
        None
    } else {
        digits.parse::<u64>().ok()
    }
}

fn ms_to_ceil_seconds(ms: u64) -> u64 {
    ms.saturating_add(999) / 1_000
}

fn nanos_to_ceil_seconds(nanos: u128) -> u64 {
    nanos
        .saturating_add(999_999_999)
        .saturating_div(1_000_000_000)
        .min(u128::from(u64::MAX)) as u64
}

fn candidate_name_from_action_id(action_id: Option<&str>) -> Option<String> {
    action_id
        .and_then(|action_id| action_id.split_once(':').map(|(_, value)| value.to_owned()))
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write};

    use super::*;
    use crate::{
        actions::RollbackToken,
        autotune::{
            controller_journal::write_controller_journal_applied,
            history::{
                AutotuneDecisionSummary, AutotuneHistoryEvent, AutotuneMode, ObservationSummary,
                SituationKind, append_autotune_history_event,
            },
        },
        scorer::StutterScore,
    };

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-autotune-tui-panel-test-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn observation(total: u64) -> ObservationSummary {
        ObservationSummary {
            target_present: true,
            active_target_count: 31,
            scored_task_count: 2,
            interval_count: 10,
            scored_samples: 100,
            score_total: total,
            over_1ms: 0,
            over_2ms: 0,
            over_5ms: 0,
            frame_p99_ms: 12.0,
            frame_max_ms: 20.0,
            drop_counter_total: 0,
            data_quality: "High".to_owned(),
        }
    }

    fn score(total: u64) -> crate::autotune::experiment::WindowScore {
        crate::autotune::experiment::WindowScore {
            started_unix_nanos: 100_000_000_000,
            finished_unix_nanos: 130_000_000_000,
            interval_count: 10,
            scored_samples: 100,
            scored_task_count: 2,
            score: StutterScore {
                total,
                ..StutterScore::default()
            },
        }
    }

    fn measuring_event() -> AutotuneHistoryEvent {
        AutotuneHistoryEvent {
            schema_version: 1,
            unix_nanos: 118_000_000_000,
            controller_id: "controller-1".to_owned(),
            phase: ControllerPhase::Measuring,
            mode: AutotuneMode::ApplyLowRisk,
            target: None,
            situation: SituationKind::GameCpuSchedulerPressure,
            observation_summary: observation(330),
            decision: AutotuneDecisionSummary {
                decision: "Noop".to_owned(),
                candidate_name: Some("game-main-suggested".to_owned()),
                action_kind: Some("cpu_affinity_profile".to_owned()),
                safety_class: Some(crate::actions::SafetyClass::ReversibleLowRisk),
                eligible: true,
                rollback_policy: "rollback-on-exit".to_owned(),
            },
            experiment_id: Some("experiment-1".to_owned()),
            action_id: Some("cpu-affinity-profile:game-main-suggested".to_owned()),
            score_before: Some(score(412)),
            score_after: None,
            planner: None,
            rollback_performed: false,
            reason: "candidate measurement window not complete: elapsed_ms=18000 required_ms=30000"
                .to_owned(),
        }
    }

    #[test]
    fn snapshot_from_history_matches_panel_example() {
        let dir = temp_dir("history");
        let history_path = dir.join("history.jsonl");
        let journal_path = dir.join("controller_journal.json");

        append_autotune_history_event(&history_path, &measuring_event()).unwrap();

        let snapshot =
            load_autotune_tui_panel_snapshot(&history_path, &journal_path, 118_000_000_000);

        assert_eq!(snapshot.mode, "ApplyLowRisk");
        assert_eq!(snapshot.phase, "Measuring");
        assert_eq!(
            snapshot.current_profile.as_deref(),
            Some("game-main-suggested")
        );
        assert_eq!(snapshot.baseline_score, Some(412));
        assert_eq!(snapshot.candidate_score, Some(330));
        assert_eq!(snapshot.decision_in_seconds, Some(12));
        assert!(snapshot.rollback_available);
        assert_eq!(snapshot.warning, None);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn applied_journal_sets_rollback_available_without_history() {
        let dir = temp_dir("journal");
        let history_path = dir.join("missing-history.jsonl");
        let journal_path = dir.join("controller_journal.json");

        write_controller_journal_applied(
            &journal_path,
            "experiment-1",
            "cpu-affinity-profile:game-main-suggested",
            RollbackToken::CpuAffinityRestoreFile {
                path: dir.join("restore.json"),
                affected_tasks: 31,
            },
        )
        .unwrap();

        let snapshot =
            load_autotune_tui_panel_snapshot(&history_path, &journal_path, 118_000_000_000);

        assert_eq!(snapshot.phase, "Disabled");
        assert_eq!(
            snapshot.current_profile.as_deref(),
            Some("game-main-suggested")
        );
        assert!(snapshot.rollback_available);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn rollback_event_clears_profile_and_rollback_available() {
        let dir = temp_dir("rollback");
        let history_path = dir.join("history.jsonl");
        let journal_path = dir.join("controller_journal.json");
        let mut reverted = measuring_event();
        reverted.unix_nanos = 140_000_000_000;
        reverted.phase = ControllerPhase::Reverting;
        reverted.decision.decision = "Revert".to_owned();
        reverted.rollback_performed = true;
        reverted.reason = "rollback performed".to_owned();

        append_autotune_history_event(&history_path, &measuring_event()).unwrap();
        append_autotune_history_event(&history_path, &reverted).unwrap();

        let snapshot =
            load_autotune_tui_panel_snapshot(&history_path, &journal_path, 140_000_000_000);

        assert_eq!(snapshot.current_profile, None);
        assert!(!snapshot.rollback_available);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn malformed_history_becomes_warning_snapshot() {
        let dir = temp_dir("malformed");
        let history_path = dir.join("history.jsonl");
        let journal_path = dir.join("controller_journal.json");

        let mut file = fs::File::create(&history_path).unwrap();
        file.write_all(b"{not-json}\n").unwrap();

        let snapshot =
            load_autotune_tui_panel_snapshot(&history_path, &journal_path, 118_000_000_000);

        assert_eq!(snapshot.phase, "Disabled");
        assert!(
            snapshot
                .warning
                .unwrap()
                .contains("failed to read autotune history")
        );

        fs::remove_dir_all(dir).ok();
    }
}
