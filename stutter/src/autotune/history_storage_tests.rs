//! Regression tests for the history log's storage: concurrent writers across rotations, torn
//! records, bounded tail reads and counters that survive rotation and restarts.
use std::{fs, path::PathBuf, sync::Arc, thread};

use super::{
    history::{
        AutotuneDecisionSummary, AutotuneHistoryEvent, AutotuneMode, ControllerPhase,
        ObservationSummary, SituationKind, append_autotune_history_event_bounded,
        read_autotune_history_events,
    },
    history_rotation::previous_generation,
    history_tail::{read_autotune_history_tail, read_tail_with_limit},
    history_totals::HistoryTotals,
    prometheus_metrics::load_autotune_prometheus_metrics,
};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "stutter-history-storage-{name}-{}-{}",
        std::process::id(),
        crate::audit::unix_nanos_now()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn event(id: u128, rolled_back: bool) -> AutotuneHistoryEvent {
    AutotuneHistoryEvent {
        focus: None,
        schema_version: 1,
        unix_nanos: id,
        controller_id: "controller-1".to_owned(),
        phase: ControllerPhase::Observing,
        mode: AutotuneMode::ApplyLowRisk,
        target: None,
        situation: SituationKind::GameCpuSchedulerPressure,
        observation_summary: ObservationSummary {
            target_present: true,
            active_target_count: 1,
            scored_task_count: 1,
            interval_count: 1,
            scored_samples: 1,
            diagnostic_raw_score_total: 330,
            over_1ms: 0,
            over_2ms: 0,
            over_5ms: 0,
            frame_p99_ms: 12.0,
            frame_max_ms: 20.0,
            drop_counter_total: 0,
            data_quality: "High".to_owned(),
        },
        decision: AutotuneDecisionSummary {
            decision: "Noop".to_owned(),
            candidate_name: None,
            action_kind: None,
            safety_class: None,
            eligible: false,
            rollback_policy: "none".to_owned(),
        },
        experiment_id: None,
        action_id: Some(format!("action-{id}").into()),
        score_before: None,
        score_after: None,
        planner: None,
        objective_evidence: None,
        rollback_performed: rolled_back,
        reason: String::new(),
    }
}

#[test]
fn concurrent_writers_never_corrupt_the_log_and_the_counters_survive_every_rotation() {
    let dir = scratch("concurrent");
    let path = Arc::new(dir.join("history.jsonl"));
    // A tiny limit forces many rotations while eight writers race.
    let max_bytes = 24 * 1024;
    let (writers, each) = (8_u128, 150_u128);
    let handles: Vec<_> = (0..writers)
        .map(|writer| {
            let path = Arc::clone(&path);
            thread::spawn(move || {
                for index in 0..each {
                    let id = writer * 1_000 + index;
                    append_autotune_history_event_bounded(&path, &event(id, true), max_bytes)
                        .unwrap();
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }

    // Every retained line is a whole, valid record; the full reader fails on any torn one.
    let current = read_autotune_history_events(&path).unwrap();
    let previous = read_autotune_history_events(&previous_generation(&path)).unwrap();
    assert!(!current.is_empty());
    assert!(!previous.is_empty(), "the writers must have rotated");
    assert!(!dir.join("history.jsonl.2").exists());
    let mut ids: Vec<u128> = current
        .iter()
        .chain(&previous)
        .map(|e| e.unix_nanos)
        .collect();
    let retained = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(
        ids.len(),
        retained,
        "no record is duplicated by a rotation race"
    );

    // The cumulative counter saw every append even though older generations were dropped.
    let totals = HistoryTotals::load(&path).unwrap();
    assert_eq!(totals.rollbacks_total, (writers * each) as u64);
    assert!(
        (retained as u64) < totals.rollbacks_total,
        "the test must drop old generations to prove continuity"
    );
}

#[test]
fn totals_persist_across_a_restart_and_are_rebuilt_when_the_checkpoint_is_lost() {
    let dir = scratch("restart");
    let path = dir.join("history.jsonl");
    for id in 0..5 {
        append_autotune_history_event_bounded(&path, &event(id, id % 2 == 0), u64::MAX).unwrap();
    }
    let before = HistoryTotals::load(&path).unwrap();
    assert_eq!(before.rollbacks_total, 3);
    // A new process reads the same checkpoint.
    assert_eq!(HistoryTotals::load(&path).unwrap(), before);
    // Losing the checkpoint rebuilds it from the retained generations; the next append stays exact.
    fs::remove_file(super::history_totals::totals_path(&path)).unwrap();
    assert_eq!(HistoryTotals::load(&path).unwrap().rollbacks_total, 3);
    append_autotune_history_event_bounded(&path, &event(99, true), u64::MAX).unwrap();
    assert_eq!(HistoryTotals::load(&path).unwrap().rollbacks_total, 4);
    // A corrupt checkpoint is also rebuilt rather than trusted.
    fs::write(super::history_totals::totals_path(&path), b"{not json").unwrap();
    assert_eq!(HistoryTotals::load(&path).unwrap().rollbacks_total, 4);
}

#[test]
fn the_tail_read_is_bounded_ignores_a_torn_record_and_still_rejects_corruption() {
    let dir = scratch("tail");
    let path = dir.join("history.jsonl");
    for id in 0..50 {
        append_autotune_history_event_bounded(&path, &event(id, false), u64::MAX).unwrap();
    }
    let last_ten = read_autotune_history_tail(&path, 10).unwrap();
    assert_eq!(last_ten.len(), 10);
    assert_eq!(last_ten.last().unwrap().unix_nanos, 49);
    assert_eq!(last_ten.first().unwrap().unix_nanos, 40);
    assert!(read_autotune_history_tail(&path, 0).unwrap().is_empty());

    // A byte window that starts mid-record drops the partial first line and keeps whole records.
    let windowed = read_tail_with_limit(&path, 1_000, 4_000).unwrap();
    assert!(!windowed.is_empty() && windowed.len() < 50);
    assert_eq!(windowed.last().unwrap().unix_nanos, 49);

    // A writer that died mid-line leaves a torn tail; it is ignored, not an error.
    let mut text = fs::read_to_string(&path).unwrap();
    text.push_str("{\"schema_version\":1,\"unix_na");
    fs::write(&path, &text).unwrap();
    assert_eq!(
        read_autotune_history_tail(&path, 5)
            .unwrap()
            .last()
            .unwrap()
            .unix_nanos,
        49
    );

    // A malformed record that is complete is still an error.
    text.push_str("\nnot json\n");
    fs::write(&path, text).unwrap();
    assert!(read_autotune_history_tail(&path, 5).is_err());
}

#[test]
fn prometheus_counters_do_not_reset_when_the_history_rotates() {
    let dir = scratch("metrics");
    let path = dir.join("history.jsonl");
    let journal = dir.join("controller_journal.json");
    let max_bytes = 8 * 1024;
    for id in 0..120 {
        append_autotune_history_event_bounded(&path, &event(id, true), max_bytes).unwrap();
    }
    assert!(previous_generation(&path).exists());
    let metrics = load_autotune_prometheus_metrics(&path, &journal);
    assert_eq!(metrics.rollbacks_total, 120);
    assert_eq!(metrics.actions_applied_total, 0);
}
