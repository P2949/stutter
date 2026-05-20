//! Tests for optional sched_wakeup_new coverage reporting.
//!
//! Owns new-task wakeup coverage regression tests. Does not own tracepoint preflight or loader
//! orchestration.

use super::*;

#[test]
fn coverage_is_full_when_sched_wakeup_new_is_available() {
    let mut warnings = Vec::new();

    let coverage = sched_wakeup_new_coverage_status("ok", &mut warnings);

    assert_eq!(coverage, "full");
    assert!(warnings.is_empty());
}

#[test]
fn coverage_warns_without_claiming_scheduler_wakeup_is_broken_when_missing() {
    let mut warnings = Vec::new();

    let coverage = sched_wakeup_new_coverage_status("missing", &mut warnings);

    assert_eq!(coverage, "reduced-new-task-wakeup-coverage");
    assert_eq!(warnings.len(), 1);
    assert!(warnings[0].contains("sched_wakeup_new"));
    assert!(warnings[0].contains("sched_wakeup remains required and usable"));
    assert!(warnings[0].contains("newly created tasks"));
}

#[test]
fn coverage_is_not_requested_when_optional_tracepoint_is_not_requested() {
    let mut warnings = Vec::new();

    let coverage = sched_wakeup_new_coverage_status("not_requested", &mut warnings);

    assert_eq!(coverage, "not_requested");
    assert!(warnings.is_empty());
}
