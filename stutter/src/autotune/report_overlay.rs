use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::{
    autotune::{
        experiment::WindowScore,
        history::{
            AutotuneHistoryEvent, ControllerPhase, default_autotune_history_path,
            read_autotune_history_events,
        },
    },
    recorder::{RecordedTime, SessionFile},
};

#[derive(Clone, Debug, Default, Serialize, PartialEq)]
pub struct AutotuneReportOverlay {
    pub source_path: Option<PathBuf>,
    pub session_start_unix_nanos: u128,
    pub session_end_unix_nanos: u128,
    pub events: Vec<AutotuneReportEvent>,
    pub skipped_outside_session: usize,
    pub warnings: Vec<String>,
}

impl AutotuneReportOverlay {
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct AutotuneReportEvent {
    pub elapsed_ms: u64,
    pub elapsed_label: String,
    pub unix_nanos: u128,
    pub phase: String,
    pub decision: String,
    pub candidate_name: Option<String>,
    pub action_id: Option<String>,
    pub experiment_id: Option<String>,
    pub score_delta_percent: Option<f64>,
    pub rollback_performed: bool,
    pub label: String,
    pub reason: String,
}

pub fn build_autotune_report_overlay(
    report_path: &Path,
    session: &SessionFile,
) -> AutotuneReportOverlay {
    let session_start_unix_nanos = recorded_time_to_unix_nanos(&session.core.started_at);
    let mut session_end_unix_nanos = recorded_time_to_unix_nanos(&session.core.ended_at);

    if session_end_unix_nanos < session_start_unix_nanos {
        session_end_unix_nanos = session_start_unix_nanos
            .saturating_add(u128::from(session.core.duration_ms).saturating_mul(1_000_000));
    }

    let mut overlay = AutotuneReportOverlay {
        source_path: None,
        session_start_unix_nanos,
        session_end_unix_nanos,
        events: Vec::new(),
        skipped_outside_session: 0,
        warnings: Vec::new(),
    };

    let Some(history_path) = first_existing_history_path(report_path) else {
        return overlay;
    };

    overlay.source_path = Some(history_path.clone());

    let history_events = match read_autotune_history_events(&history_path) {
        Ok(events) => events,
        Err(err) => {
            overlay.warnings.push(format!(
                "failed to read autotune history {}: {err:#}",
                history_path.display()
            ));
            return overlay;
        }
    };

    for event in history_events {
        if event.unix_nanos < session_start_unix_nanos || event.unix_nanos > session_end_unix_nanos
        {
            overlay.skipped_outside_session += 1;
            continue;
        }

        overlay.events.push(report_event_from_history_event(
            &event,
            session_start_unix_nanos,
        ));
    }

    overlay.events.sort_by_key(|event| event.unix_nanos);
    overlay
}

pub fn render_autotune_events_text(overlay: &AutotuneReportOverlay) -> String {
    let mut output = String::new();

    if overlay.events.is_empty() && overlay.warnings.is_empty() {
        return output;
    }

    output.push_str("Autotune events:\n");

    if overlay.events.is_empty() {
        output.push_str("  none recorded during this run\n");
    } else {
        for event in &overlay.events {
            output.push_str("  ");
            output.push_str(&event.elapsed_label);
            output.push(' ');
            output.push_str(&event.label);
            output.push('\n');
        }
    }

    for warning in &overlay.warnings {
        output.push_str("  warning: ");
        output.push_str(warning);
        output.push('\n');
    }

    output.push('\n');
    output
}

pub fn append_autotune_overlay_to_legacy_text(
    mut text_report: String,
    overlay: &AutotuneReportOverlay,
) -> String {
    let rendered_overlay = render_autotune_events_text(overlay);

    if rendered_overlay.is_empty() {
        return text_report;
    }

    if !text_report.ends_with('\n') {
        text_report.push('\n');
    }
    text_report.push('\n');
    text_report.push_str(&rendered_overlay);
    text_report
}

fn report_event_from_history_event(
    event: &AutotuneHistoryEvent,
    session_start_unix_nanos: u128,
) -> AutotuneReportEvent {
    let elapsed_ms = event
        .unix_nanos
        .saturating_sub(session_start_unix_nanos)
        .saturating_div(1_000_000)
        .min(u128::from(u64::MAX)) as u64;
    let score_delta_percent =
        score_delta_percent(event.score_before.as_ref(), event.score_after.as_ref());
    let candidate_name = event
        .decision
        .candidate_name
        .clone()
        .or_else(|| candidate_name_from_action_id(event.action_id.as_deref()));
    let label = human_label_for_event(event, candidate_name.as_deref(), score_delta_percent);

    AutotuneReportEvent {
        elapsed_ms,
        elapsed_label: format_elapsed_label(elapsed_ms),
        unix_nanos: event.unix_nanos,
        phase: format!("{:?}", event.phase),
        decision: event.decision.decision.clone(),
        candidate_name,
        action_id: event.action_id.clone(),
        experiment_id: event.experiment_id.clone(),
        score_delta_percent,
        rollback_performed: event.rollback_performed,
        label,
        reason: event.reason.clone(),
    }
}

fn human_label_for_event(
    event: &AutotuneHistoryEvent,
    candidate_name: Option<&str>,
    score_delta_percent: Option<f64>,
) -> String {
    let decision = event.decision.decision.as_str();
    let decision_lc = decision.to_ascii_lowercase();
    let reason_lc = event.reason.to_ascii_lowercase();
    let candidate = candidate_name.unwrap_or("candidate");

    if reason_lc.contains("baseline") && reason_lc.contains("started") {
        return "baseline window started".to_owned();
    }

    if decision_lc.contains("baseline") {
        return "baseline window started".to_owned();
    }

    if decision_lc.contains("startexperiment")
        || decision_lc.contains("start_experiment")
        || decision_lc.contains("apply")
        || matches!(event.phase, ControllerPhase::Applying)
    {
        return format!("applied profile {candidate}");
    }

    if reason_lc.contains("washout")
        && (reason_lc.contains("ended") || reason_lc.contains("complete"))
    {
        return "washout ended".to_owned();
    }

    if decision_lc.contains("keep") || matches!(event.phase, ControllerPhase::Keeping) {
        if let Some(delta) = score_delta_percent {
            if delta >= 0.0 {
                return format!("kept profile, score improved {:.1}%", delta);
            }
            return format!("kept profile, score regressed {:.1}%", delta.abs());
        }

        return "kept profile".to_owned();
    }

    if decision_lc.contains("revert") || matches!(event.phase, ControllerPhase::Reverting) {
        return format!("reverted profile {candidate}");
    }

    if decision_lc.contains("suggest") {
        return format!("suggested profile {candidate}");
    }

    if decision_lc.contains("fault") || matches!(event.phase, ControllerPhase::Faulted) {
        return format!("controller fault: {}", event.reason);
    }

    if !event.reason.trim().is_empty() {
        return event.reason.clone();
    }

    decision.to_owned()
}

fn score_delta_percent(
    score_before: Option<&WindowScore>,
    score_after: Option<&WindowScore>,
) -> Option<f64> {
    let before = score_before?.score.total;
    let after = score_after?.score.total;

    if before == 0 {
        return None;
    }

    Some(((before as f64 - after as f64) / before as f64) * 100.0)
}

fn first_existing_history_path(report_path: &Path) -> Option<PathBuf> {
    candidate_history_paths(report_path)
        .into_iter()
        .find(|path| path.exists())
}

fn candidate_history_paths(report_path: &Path) -> Vec<PathBuf> {
    let run_dir = report_run_dir(report_path);
    let candidates = [
        run_dir.join("autotune_history.jsonl"),
        run_dir.join("autotune").join("history.jsonl"),
        default_autotune_history_path(),
    ];
    let mut seen = BTreeSet::new();
    let mut paths = Vec::new();

    for path in candidates {
        if seen.insert(path.clone()) {
            paths.push(path);
        }
    }

    paths
}

fn report_run_dir(report_path: &Path) -> PathBuf {
    if report_path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "session.json")
    {
        return report_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
    }

    report_path.to_path_buf()
}

fn recorded_time_to_unix_nanos(recorded: &RecordedTime) -> u128 {
    u128::from(recorded.unix_seconds)
        .saturating_mul(1_000_000_000)
        .saturating_add(u128::from(recorded.unix_nanos))
}

fn format_elapsed_label(elapsed_ms: u64) -> String {
    let total_seconds = elapsed_ms / 1_000;
    let hours = total_seconds / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;

    if hours > 0 {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes:02}:{seconds:02}")
    }
}

fn candidate_name_from_action_id(action_id: Option<&str>) -> Option<String> {
    action_id
        .and_then(|action_id| action_id.split_once(':').map(|(_, value)| value.to_owned()))
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::{
        autotune::{
            experiment::WindowScore,
            history::{
                AutotuneDecisionSummary, AutotuneHistoryEvent, AutotuneMode, ControllerPhase,
                ObservationSummary, SituationKind, append_autotune_history_event,
            },
        },
        recorder::{RecordedTime, SessionFile},
        scorer::StutterScore,
    };

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-autotune-report-overlay-{name}-{}-{}",
            std::process::id(),
            crate::audit::unix_nanos_now()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn session() -> SessionFile {
        let mut session = SessionFile::default();
        session.core.started_at = RecordedTime {
            unix_seconds: 100,
            unix_nanos: 0,
            system_time_debug: "start".to_owned(),
        };
        session.core.ended_at = RecordedTime {
            unix_seconds: 200,
            unix_nanos: 0,
            system_time_debug: "end".to_owned(),
        };
        session.core.duration_ms = 100_000;
        session
    }

    fn observation_summary(total: u64) -> ObservationSummary {
        ObservationSummary {
            target_present: true,
            active_target_count: 1,
            scored_task_count: 1,
            interval_count: 1,
            scored_samples: 1,
            score_total: total,
            over_1ms: 0,
            over_2ms: 0,
            over_5ms: 0,
            frame_p99_ms: 0.0,
            frame_max_ms: 0.0,
            drop_counter_total: 0,
            data_quality: "High".to_owned(),
        }
    }

    fn window_score(total: u64) -> WindowScore {
        WindowScore {
            started_unix_nanos: 100_000_000_000,
            finished_unix_nanos: 110_000_000_000,
            interval_count: 1,
            scored_samples: 1,
            scored_task_count: 1,
            score: StutterScore {
                total,
                ..StutterScore::default()
            },
        }
    }

    fn history_event(
        unix_nanos: u128,
        phase: ControllerPhase,
        decision: &str,
        reason: &str,
    ) -> AutotuneHistoryEvent {
        let mut event = AutotuneHistoryEvent::new(
            "controller-1",
            phase,
            AutotuneMode::ApplyLowRisk,
            None,
            SituationKind::GameCpuSchedulerPressure,
            observation_summary(820),
            AutotuneDecisionSummary {
                decision: decision.to_owned(),
                candidate_name: Some("game-main-suggested".to_owned()),
                action_kind: Some("cpu_affinity_profile".to_owned()),
                safety_class: Some(crate::actions::SafetyClass::ReversibleLowRisk),
                eligible: true,
                rollback_policy: "rollback-on-exit".to_owned(),
            },
            reason,
        )
        .with_experiment_id("experiment-1")
        .with_action_id("cpu-affinity-profile:game-main-suggested");

        event.unix_nanos = unix_nanos;
        event
    }

    #[test]
    fn formats_elapsed_labels() {
        assert_eq!(format_elapsed_label(30_000), "00:30");
        assert_eq!(format_elapsed_label(61_000), "01:01");
        assert_eq!(format_elapsed_label(3_661_000), "01:01:01");
    }

    #[test]
    fn builds_overlay_from_run_local_history_and_filters_session_window() {
        let dir = temp_dir("local-history");
        let history_path = dir.join("autotune_history.jsonl");

        append_autotune_history_event(
            &history_path,
            &history_event(
                90_000_000_000,
                ControllerPhase::Observing,
                "Noop",
                "outside before run",
            ),
        )
        .unwrap();

        append_autotune_history_event(
            &history_path,
            &history_event(
                130_000_000_000,
                ControllerPhase::Applying,
                "StartExperiment",
                "candidate passed gates",
            ),
        )
        .unwrap();

        append_autotune_history_event(
            &history_path,
            &history_event(
                210_000_000_000,
                ControllerPhase::Cooldown,
                "Noop",
                "outside after run",
            ),
        )
        .unwrap();

        let overlay = build_autotune_report_overlay(&dir, &session());

        assert_eq!(overlay.source_path.as_deref(), Some(history_path.as_path()));
        assert_eq!(overlay.events.len(), 1);
        assert_eq!(overlay.skipped_outside_session, 2);
        assert_eq!(overlay.events[0].elapsed_label, "00:30");
        assert_eq!(
            overlay.events[0].label,
            "applied profile game-main-suggested"
        );

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn renders_kept_profile_score_improvement() {
        let mut event = history_event(
            140_000_000_000,
            ControllerPhase::Keeping,
            "KeepCurrent",
            "candidate improved by enough",
        )
        .with_scores(Some(window_score(1_000)), Some(window_score(818)));

        event.unix_nanos = 140_000_000_000;

        let report_event = report_event_from_history_event(&event, 100_000_000_000);

        assert_eq!(report_event.label, "kept profile, score improved 18.2%");
    }

    #[test]
    fn renders_text_overlay() {
        let overlay = AutotuneReportOverlay {
            events: vec![AutotuneReportEvent {
                elapsed_ms: 30_000,
                elapsed_label: "00:30".to_owned(),
                unix_nanos: 130_000_000_000,
                phase: "Applying".to_owned(),
                decision: "StartExperiment".to_owned(),
                candidate_name: Some("game-main-suggested".to_owned()),
                action_id: Some("cpu-affinity-profile:game-main-suggested".to_owned()),
                experiment_id: Some("experiment-1".to_owned()),
                score_delta_percent: None,
                rollback_performed: false,
                label: "applied profile game-main-suggested".to_owned(),
                reason: "candidate passed gates".to_owned(),
            }],
            ..AutotuneReportOverlay::default()
        };

        let rendered = render_autotune_events_text(&overlay);

        assert!(rendered.contains("Autotune events:"));
        assert!(rendered.contains("  00:30 applied profile game-main-suggested"));
    }

    #[test]
    fn appends_overlay_to_legacy_text_report() {
        let overlay = AutotuneReportOverlay {
            events: vec![AutotuneReportEvent {
                elapsed_ms: 70_000,
                elapsed_label: "01:10".to_owned(),
                unix_nanos: 170_000_000_000,
                phase: "Measuring".to_owned(),
                decision: "Noop".to_owned(),
                candidate_name: None,
                action_id: None,
                experiment_id: None,
                score_delta_percent: None,
                rollback_performed: false,
                label: "washout ended".to_owned(),
                reason: "washout ended".to_owned(),
            }],
            ..AutotuneReportOverlay::default()
        };

        let rendered = append_autotune_overlay_to_legacy_text("report body\n".to_owned(), &overlay);

        assert!(rendered.contains("report body"));
        assert!(rendered.contains("Autotune events:"));
        assert!(rendered.contains("01:10 washout ended"));
    }
}
