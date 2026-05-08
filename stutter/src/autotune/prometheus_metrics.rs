#![allow(dead_code)]

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use crate::autotune::{
    controller_journal::{
        ControllerJournalRecord, default_controller_journal_path, read_controller_journal,
    },
    history::{
        AutotuneHistoryEvent, AutotuneMode, ControllerPhase, default_autotune_history_path,
        read_autotune_history_events,
    },
};

#[derive(Clone, Debug, PartialEq)]
pub struct AutotunePrometheusMetrics {
    pub phase: ControllerPhase,
    pub mode: AutotuneMode,
    pub active_experiment: bool,
    pub last_score: Option<u64>,
    pub candidate_score: Option<u64>,
    pub rollbacks_total: u64,
    pub actions_applied_total: u64,
    pub actions_blocked_total: u64,
    pub history_path: PathBuf,
    pub journal_path: PathBuf,
    pub scrape_error: Option<String>,
}

impl AutotunePrometheusMetrics {
    pub fn empty(history_path: PathBuf, journal_path: PathBuf) -> Self {
        Self {
            phase: ControllerPhase::Disabled,
            mode: AutotuneMode::Observe,
            active_experiment: false,
            last_score: None,
            candidate_score: None,
            rollbacks_total: 0,
            actions_applied_total: 0,
            actions_blocked_total: 0,
            history_path,
            journal_path,
            scrape_error: None,
        }
    }
}

pub fn load_default_autotune_prometheus_metrics() -> AutotunePrometheusMetrics {
    let history_path = default_autotune_history_path();
    let journal_path = default_controller_journal_path();
    load_autotune_prometheus_metrics(&history_path, &journal_path)
}

pub fn load_autotune_prometheus_metrics(
    history_path: &Path,
    journal_path: &Path,
) -> AutotunePrometheusMetrics {
    let mut metrics =
        AutotunePrometheusMetrics::empty(history_path.to_path_buf(), journal_path.to_path_buf());
    let mut scrape_errors = Vec::new();

    let history_events = match read_autotune_history_events(history_path) {
        Ok(events) => events,
        Err(err) => {
            scrape_errors.push(format!(
                "failed to read autotune history {}: {err:#}",
                history_path.display()
            ));
            Vec::new()
        }
    };

    let journal_record = match read_controller_journal(journal_path) {
        Ok(record) => Some(record),
        Err(err) => {
            scrape_errors.push(format!(
                "failed to read autotune controller journal {}: {err:#}",
                journal_path.display()
            ));
            None
        }
    };

    if let Some(last) = history_events.last() {
        metrics.phase = last.phase;
        metrics.mode = last.mode;
        metrics.last_score = latest_score(&history_events);
        metrics.candidate_score = latest_candidate_score(&history_events);
    }

    metrics.active_experiment = active_experiment_from_history(&history_events)
        || active_experiment_from_journal(journal_record.as_ref());
    metrics.rollbacks_total = history_events
        .iter()
        .filter(|event| event.rollback_performed)
        .count()
        .min(u64::MAX as usize) as u64;
    metrics.actions_applied_total = count_unique_applied_actions(&history_events);
    metrics.actions_blocked_total = history_events
        .iter()
        .filter(|event| is_blocked_event(event))
        .count()
        .min(u64::MAX as usize) as u64;
    metrics.scrape_error = if scrape_errors.is_empty() {
        None
    } else {
        Some(scrape_errors.join("; "))
    };

    metrics
}

pub fn render_default_autotune_prometheus_metrics() -> String {
    render_autotune_prometheus_metrics(&load_default_autotune_prometheus_metrics())
}

pub fn render_autotune_prometheus_metrics(metrics: &AutotunePrometheusMetrics) -> String {
    let phase_label = format!("{:?}", metrics.phase);
    let mode_label = format!("{:?}", metrics.mode);
    let active_experiment = if metrics.active_experiment { 1 } else { 0 };
    let last_score = metrics.last_score.unwrap_or(0);
    let candidate_score = metrics.candidate_score.unwrap_or(0);
    let scrape_error = metrics.scrape_error.as_deref().unwrap_or("");
    let scrape_error_value = if scrape_error.is_empty() { 0 } else { 1 };

    format!(
        concat!(
            "# HELP stutter_autotune_phase Current autotune controller phase as a numeric enum gauge. Labels expose the phase name. disabled=0 observing=1 planning=2 applying=3 measuring=4 keeping=5 reverting=6 cooldown=7 faulted=8.\n",
            "# TYPE stutter_autotune_phase gauge\n",
            "stutter_autotune_phase{{phase=\"{phase_label}\"}} {phase_value}\n",
            "# HELP stutter_autotune_mode Current autotune mode as a numeric enum gauge. Labels expose the mode name. observe=0 suggest=1 apply_low_risk=2 apply_medium_risk=3 apply_high_risk=4.\n",
            "# TYPE stutter_autotune_mode gauge\n",
            "stutter_autotune_mode{{mode=\"{mode_label}\"}} {mode_value}\n",
            "# HELP stutter_autotune_active_experiment Whether autotune currently has an active applying/applied/measuring experiment according to history or controller journal.\n",
            "# TYPE stutter_autotune_active_experiment gauge\n",
            "stutter_autotune_active_experiment {active_experiment}\n",
            "# HELP stutter_autotune_last_score Last observed autotune score_total. Lower is better.\n",
            "# TYPE stutter_autotune_last_score gauge\n",
            "stutter_autotune_last_score {last_score}\n",
            "# HELP stutter_autotune_candidate_score Last candidate autotune score_total. Lower is better.\n",
            "# TYPE stutter_autotune_candidate_score gauge\n",
            "stutter_autotune_candidate_score {candidate_score}\n",
            "# HELP stutter_autotune_rollbacks_total Total autotune rollback events recorded in autotune history.\n",
            "# TYPE stutter_autotune_rollbacks_total counter\n",
            "stutter_autotune_rollbacks_total {rollbacks_total}\n",
            "# HELP stutter_autotune_actions_applied_total Total unique autotune action IDs that reached an applying/start/apply decision in autotune history.\n",
            "# TYPE stutter_autotune_actions_applied_total counter\n",
            "stutter_autotune_actions_applied_total {actions_applied_total}\n",
            "# HELP stutter_autotune_actions_blocked_total Total autotune decisions blocked by safety, cooldown, focus, data-quality, regression, or missing-candidate gates in autotune history.\n",
            "# TYPE stutter_autotune_actions_blocked_total counter\n",
            "stutter_autotune_actions_blocked_total {actions_blocked_total}\n",
            "# HELP stutter_autotune_metrics_scrape_error Whether the autotune Prometheus scrape could not fully read history or journal state.\n",
            "# TYPE stutter_autotune_metrics_scrape_error gauge\n",
            "stutter_autotune_metrics_scrape_error{{error=\"{scrape_error}\"}} {scrape_error_value}\n",
        ),
        phase_label = escape_label_value(&phase_label),
        phase_value = phase_value(metrics.phase),
        mode_label = escape_label_value(&mode_label),
        mode_value = mode_value(metrics.mode),
        active_experiment = active_experiment,
        last_score = last_score,
        candidate_score = candidate_score,
        rollbacks_total = metrics.rollbacks_total,
        actions_applied_total = metrics.actions_applied_total,
        actions_blocked_total = metrics.actions_blocked_total,
        scrape_error = escape_label_value(scrape_error),
        scrape_error_value = scrape_error_value,
    )
}

fn latest_score(events: &[AutotuneHistoryEvent]) -> Option<u64> {
    for event in events.iter().rev() {
        if event.observation_summary.score_total > 0 {
            return Some(event.observation_summary.score_total);
        }

        if let Some(score) = event.score_after.as_ref() {
            return Some(score.score.total);
        }

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

        if is_candidate_phase(event.phase) && event.observation_summary.score_total > 0 {
            return Some(event.observation_summary.score_total);
        }
    }

    None
}

fn active_experiment_from_history(events: &[AutotuneHistoryEvent]) -> bool {
    let Some(last) = events.last() else {
        return false;
    };

    if last.rollback_performed {
        return false;
    }

    matches!(
        last.phase,
        ControllerPhase::Applying
            | ControllerPhase::Measuring
            | ControllerPhase::Keeping
            | ControllerPhase::Reverting
    ) && last.experiment_id.is_some()
}

fn active_experiment_from_journal(record: Option<&ControllerJournalRecord>) -> bool {
    matches!(
        record,
        Some(ControllerJournalRecord::Applying { .. })
            | Some(ControllerJournalRecord::Applied { .. })
    )
}

fn count_unique_applied_actions(events: &[AutotuneHistoryEvent]) -> u64 {
    let mut unique = BTreeSet::<String>::new();

    for event in events {
        if is_applied_event(event) {
            unique.insert(
                event
                    .action_id
                    .clone()
                    .unwrap_or_else(|| format!("event:{}", event.unix_nanos)),
            );
        }
    }

    unique.len().min(u64::MAX as usize) as u64
}

fn is_applied_event(event: &AutotuneHistoryEvent) -> bool {
    let decision = event.decision.decision.to_ascii_lowercase();

    matches!(event.phase, ControllerPhase::Applying)
        || decision.contains("startexperiment")
        || decision.contains("start_experiment")
        || decision.contains("apply")
        || decision.contains("applied")
}

fn is_blocked_event(event: &AutotuneHistoryEvent) -> bool {
    let decision = event.decision.decision.to_ascii_lowercase();
    let reason = event.reason.to_ascii_lowercase();

    if decision.contains("entercooldown")
        || decision.contains("enter_cooldown")
        || decision.contains("fault")
    {
        return true;
    }

    if !decision.contains("noop") && !decision.contains("reject") && !decision.contains("block") {
        return false;
    }

    [
        "block",
        "blocked",
        "cooldown",
        "low data quality",
        "focus policy",
        "safety class",
        "exceeds",
        "regress",
        "no candidate",
        "target disappeared",
        "controller is disabled",
        "controller is faulted",
    ]
    .iter()
    .any(|needle| reason.contains(needle))
}

fn is_candidate_phase(phase: ControllerPhase) -> bool {
    matches!(
        phase,
        ControllerPhase::Applying
            | ControllerPhase::Measuring
            | ControllerPhase::Keeping
            | ControllerPhase::Reverting
    )
}

fn phase_value(phase: ControllerPhase) -> u64 {
    match phase {
        ControllerPhase::Disabled => 0,
        ControllerPhase::Observing => 1,
        ControllerPhase::Planning => 2,
        ControllerPhase::Applying => 3,
        ControllerPhase::Measuring => 4,
        ControllerPhase::Keeping => 5,
        ControllerPhase::Reverting => 6,
        ControllerPhase::Cooldown => 7,
        ControllerPhase::Faulted => 8,
    }
}

fn mode_value(mode: AutotuneMode) -> u64 {
    match mode {
        AutotuneMode::Observe => 0,
        AutotuneMode::Suggest => 1,
        AutotuneMode::ApplyLowRisk => 2,
        AutotuneMode::ApplyMediumRisk => 3,
        AutotuneMode::ApplyHighRisk => 4,
    }
}

fn escape_label_value(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('"', "\\\"")
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
                AutotuneDecisionSummary, AutotuneHistoryEvent, ObservationSummary, SituationKind,
                append_autotune_history_event,
            },
        },
        scorer::StutterScore,
    };

    fn temp_dir(name: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "stutter-autotune-prometheus-test-{name}-{}-{}",
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

    fn event(
        unix_nanos: u128,
        phase: ControllerPhase,
        mode: AutotuneMode,
        decision: &str,
        reason: &str,
        rollback_performed: bool,
    ) -> AutotuneHistoryEvent {
        AutotuneHistoryEvent {
            schema_version: 1,
            unix_nanos,
            controller_id: "controller-1".to_owned(),
            phase,
            mode,
            target: None,
            situation: SituationKind::GameCpuSchedulerPressure,
            observation_summary: observation(330),
            decision: AutotuneDecisionSummary {
                decision: decision.to_owned(),
                candidate_name: Some("game-main-suggested".to_owned()),
                action_kind: Some("cpu_affinity_profile".to_owned()),
                eligible: true,
                rollback_policy: "rollback-on-exit".to_owned(),
            },
            experiment_id: Some("experiment-1".to_owned()),
            action_id: Some("cpu-affinity-profile:game-main-suggested".to_owned()),
            score_before: Some(score(412)),
            score_after: Some(score(330)),
            rollback_performed,
            reason: reason.to_owned(),
        }
    }

    #[test]
    fn render_autotune_metrics_includes_requested_names() {
        let metrics = AutotunePrometheusMetrics {
            phase: ControllerPhase::Measuring,
            mode: AutotuneMode::ApplyLowRisk,
            active_experiment: true,
            last_score: Some(330),
            candidate_score: Some(330),
            rollbacks_total: 1,
            actions_applied_total: 2,
            actions_blocked_total: 3,
            history_path: PathBuf::from("/tmp/history.jsonl"),
            journal_path: PathBuf::from("/tmp/controller_journal.json"),
            scrape_error: None,
        };

        let output = render_autotune_prometheus_metrics(&metrics);

        for metric in [
            "stutter_autotune_phase",
            "stutter_autotune_mode",
            "stutter_autotune_active_experiment",
            "stutter_autotune_last_score",
            "stutter_autotune_candidate_score",
            "stutter_autotune_rollbacks_total",
            "stutter_autotune_actions_applied_total",
            "stutter_autotune_actions_blocked_total",
        ] {
            assert!(
                output.contains(metric),
                "missing metric {metric} in output:\n{output}"
            );
        }

        assert!(output.contains("stutter_autotune_phase{phase=\"Measuring\"} 4"));
        assert!(output.contains("stutter_autotune_mode{mode=\"ApplyLowRisk\"} 2"));
        assert!(output.contains("stutter_autotune_active_experiment 1"));
        assert!(output.contains("stutter_autotune_last_score 330"));
        assert!(output.contains("stutter_autotune_candidate_score 330"));
        assert!(output.contains("stutter_autotune_rollbacks_total 1"));
        assert!(output.contains("stutter_autotune_actions_applied_total 2"));
        assert!(output.contains("stutter_autotune_actions_blocked_total 3"));
    }

    #[test]
    fn load_metrics_from_history_counts_applied_blocked_and_rollbacks() {
        let dir = temp_dir("history");
        let history_path = dir.join("history.jsonl");
        let journal_path = dir.join("controller_journal.json");

        append_autotune_history_event(
            &history_path,
            &event(
                1,
                ControllerPhase::Applying,
                AutotuneMode::ApplyLowRisk,
                "StartExperiment",
                "candidate passed gates",
                false,
            ),
        )
        .unwrap();
        append_autotune_history_event(
            &history_path,
            &event(
                2,
                ControllerPhase::Measuring,
                AutotuneMode::ApplyLowRisk,
                "Noop",
                "cooldown blocks repeated action",
                false,
            ),
        )
        .unwrap();
        append_autotune_history_event(
            &history_path,
            &event(
                3,
                ControllerPhase::Reverting,
                AutotuneMode::ApplyLowRisk,
                "Revert",
                "regressed; rollback performed",
                true,
            ),
        )
        .unwrap();

        let metrics = load_autotune_prometheus_metrics(&history_path, &journal_path);

        assert_eq!(metrics.phase, ControllerPhase::Reverting);
        assert_eq!(metrics.mode, AutotuneMode::ApplyLowRisk);
        assert_eq!(metrics.last_score, Some(330));
        assert_eq!(metrics.candidate_score, Some(330));
        assert_eq!(metrics.rollbacks_total, 1);
        assert_eq!(metrics.actions_applied_total, 1);
        assert_eq!(metrics.actions_blocked_total, 1);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn applied_journal_sets_active_experiment_without_history() {
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

        let metrics = load_autotune_prometheus_metrics(&history_path, &journal_path);

        assert!(metrics.active_experiment);
        assert_eq!(metrics.phase, ControllerPhase::Disabled);
        assert_eq!(metrics.mode, AutotuneMode::Observe);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn malformed_history_sets_scrape_error_metric() {
        let dir = temp_dir("malformed");
        let history_path = dir.join("history.jsonl");
        let journal_path = dir.join("controller_journal.json");

        let mut file = fs::File::create(&history_path).unwrap();
        file.write_all(b"{not-json}\n").unwrap();

        let metrics = load_autotune_prometheus_metrics(&history_path, &journal_path);
        let output = render_autotune_prometheus_metrics(&metrics);

        assert!(metrics.scrape_error.is_some());
        assert!(output.contains("stutter_autotune_metrics_scrape_error"));
        assert!(output.contains(" 1\n"));

        fs::remove_dir_all(dir).ok();
    }
}
