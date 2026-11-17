use serde_json::json;

use super::*;

fn process(slot: u32) -> AttemptProcess {
    AttemptProcess {
        slot,
        pid: stutter_core::ids::Pid::new(100 + slot),
        start_ticks: 5000 + u64::from(slot),
        uid: 1000,
    }
}

fn receipt(slot: u32) -> serde_json::Value {
    json!({ "pid": 100 + slot, "start_ticks": 5000 + slot, "uid": 1000 })
}

/// Off, On, On, Off: slots 1 and 2 are treated.
fn schedule() -> Vec<Treatment> {
    vec![Treatment::Off, Treatment::On, Treatment::On, Treatment::Off]
}

fn all() -> Vec<AttemptProcess> {
    (0..4).map(process).collect()
}

#[test]
fn an_action_applied_on_every_treated_attempt_is_an_applied_comparison() {
    let receipts = json!([receipt(1), receipt(2)]);
    let coverage = attribute(&receipts, &all(), &schedule()).unwrap();
    assert_eq!(
        coverage,
        Coverage {
            on_completed: 2,
            on_applied: 2
        }
    );
    assert!(coverage.applied_on_every_treated_attempt());
}

#[test]
fn a_treated_attempt_the_action_never_reached_makes_the_study_a_policy_comparison_only() {
    // Only one of the two On attempts was acted on: eight runs are not four applied pairs.
    let coverage = attribute(&json!([receipt(1)]), &all(), &schedule()).unwrap();
    assert_eq!(
        coverage,
        Coverage {
            on_completed: 2,
            on_applied: 1
        }
    );
    assert!(!coverage.applied_on_every_treated_attempt());
    // No receipts at all, and no completed On attempt at all, are never "applied".
    assert!(
        !attribute(&json!([]), &all(), &schedule())
            .unwrap()
            .applied_on_every_treated_attempt()
    );
    assert!(
        !attribute(&json!([]), &[], &schedule())
            .unwrap()
            .applied_on_every_treated_attempt()
    );
    // Several transactions on one attempt are still one treated attempt.
    let twice = json!([receipt(1), receipt(1), receipt(2)]);
    assert_eq!(
        attribute(&twice, &all(), &schedule()).unwrap().on_applied,
        2
    );
}

#[test]
fn a_receipt_must_belong_to_a_completed_attempt_treated_on() {
    // A receipt about an Off attempt: the action must never run there.
    assert_eq!(
        attribute(&json!([receipt(0), receipt(1)]), &all(), &schedule()),
        Err("ReceiptOnAnUntreatedAttempt")
    );
    // A receipt about a process no completed attempt names (another process, or one that never
    // sealed a result).
    let foreign = json!([{ "pid": 999, "start_ticks": 5001, "uid": 1000 }]);
    assert_eq!(
        attribute(&foreign, &all(), &schedule()),
        Err("ReceiptForNoCompletedAttempt")
    );
    // The right pid under another start time, or another user, is another process.
    for (key, value) in [("start_ticks", 1), ("uid", 0)] {
        let mut wrong = receipt(1);
        wrong[key] = json!(value);
        assert_eq!(
            attribute(&json!([wrong]), &all(), &schedule()),
            Err("ReceiptForNoCompletedAttempt"),
            "{key}"
        );
    }
}
