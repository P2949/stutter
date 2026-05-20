//! Regression coverage for tune scoring and coverage calculations.

use super::{support::*, *};

#[test]
fn tune_counts_only_scored_post_warmup_records() {
    let records = vec![
        interval_record(10, TaskClass::Helper, 200, "helper"),
        interval_record(11, TaskClass::Compositor, 100, "compositor"),
    ];

    assert_eq!(crate::tune::tune_scored_record_counts(&records), (0, 0));

    let mut records = records;
    records.push(interval_record(12, TaskClass::Game, 55, "game"));

    assert_eq!(crate::tune::tune_scored_record_counts(&records), (1, 55));
}

#[test]
fn tune_coverage_counts_duplicate_scored_thread_identities() {
    let mut session = minimal_session_for_report();
    session.tasks = vec![
        session_task(10, 100, TaskClass::Game, "worker", Some(1000), Some(10)),
        session_task(11, 100, TaskClass::Game, "worker", Some(1000), Some(11)),
        session_task(12, 100, TaskClass::Game, "worker", Some(1000), Some(12)),
    ];
    let intervals = vec![
        interval_record(10, TaskClass::Game, 10, "worker"),
        interval_record(11, TaskClass::Game, 10, "worker"),
        interval_record(12, TaskClass::Game, 10, "worker"),
    ];

    let coverage = tune::comparability::tune_coverage_metrics(&session, &intervals);

    assert_eq!(coverage.unique_scored_tasks, 3);
    assert_eq!(
        coverage
            .scored_identity_counts
            .iter()
            .map(|c| c.count)
            .sum::<usize>(),
        3
    );
    assert_eq!(coverage.scored_identity_counts.len(), 3);
}
