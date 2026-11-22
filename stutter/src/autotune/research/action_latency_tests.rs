use serde_json::json;

use super::*;

const SLOTS: u32 = 8;

fn processes() -> Vec<AttemptProcess> {
    (0..SLOTS)
        .map(|slot| AttemptProcess {
            slot,
            pid: stutter_core::ids::Pid::new(100 + slot),
            start_ticks: 50 + u64::from(slot),
            uid: 1000,
            started_unix: 1_000,
            completed_unix: 1_100,
        })
        .collect()
}

/// AB then BA blocks over eight attempts: four pairs.
fn schedule() -> Vec<Treatment> {
    use Treatment::{Off, On};
    vec![Off, On, On, Off, Off, On, On, Off]
}

/// A population of 2000 events whose 99th percentile is `tail` ms (1% of events sit in the last bucket).
fn histogram(tail: f64) -> serde_json::Value {
    json!([{ "le_ms": 1.0, "count": 1900 }, { "le_ms": tail, "count": 100 }])
}

fn window(block: u32, tail: f64, foreground: f64) -> serde_json::Value {
    let _ = block;
    json!({
        "start_unix": 1_010, "end_unix": 1_090, "lost": 0, "same_target": true,
        "histogram": histogram(tail), "foreground_p99_ms": foreground,
    })
}

fn pair(off_slot: u32, on_slot: u32, off_tail: f64, on_tail: f64) -> serde_json::Value {
    let identity = |slot: u32| json!({ "pid": 100 + slot, "start_ticks": 50 + slot, "uid": 1000 });
    json!({
        "off_slot": off_slot, "on_slot": on_slot,
        "off_process": identity(off_slot), "on_process": identity(on_slot),
        "off": window(off_slot, off_tail, 16.0), "on": window(on_slot, on_tail, 16.0),
    })
}

fn pairs(off_tail: f64, on_tail: f64) -> serde_json::Value {
    // Blocks: (0,1) off then on, (2,3) on then off, (4,5), (6,7) on then off.
    json!([
        pair(0, 1, off_tail, on_tail),
        pair(3, 2, off_tail, on_tail),
        pair(4, 5, off_tail, on_tail),
        pair(7, 6, off_tail, on_tail),
    ])
}

fn record(edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let mut value =
        json!({ "schema": 2, "metric": "GameRunnableLatency", "pairs": pairs(9.0, 5.0) });
    edit(&mut value);
    serde_json::to_vec(&value).unwrap()
}

fn check(bytes: &[u8]) -> Result<LatencyResult, &'static str> {
    verify(bytes, &schedule(), &processes())
}

#[test]
fn the_tail_is_recomputed_from_the_populations_and_the_frozen_rule_decides_keep_revert_or_null() {
    assert_eq!(
        check(&record(|_| {})),
        Ok(LatencyResult {
            pairs: 4,
            improved: 4,
            decision: Decision::Keep
        })
    );
    let result = |off: f64, on: f64| {
        check(&record(|v| v["pairs"] = pairs(off, on)))
            .unwrap()
            .decision
    };
    assert_eq!(result(5.0, 9.0), Decision::Revert);
    // Inside the practical-effect threshold: a valid null.
    assert_eq!(result(5.0, 5.1), Decision::Null);
    // A stated percentile is not a field at all; nothing but the population counts.
    let mut stated = record(|_| {});
    let text = String::from_utf8(stated.clone())
        .unwrap()
        .replace("\"lost\"", "\"p99_ms\":1.0,\"lost\"");
    stated = text.into_bytes();
    assert_eq!(check(&stated), Err("ActionLatencyUnreadable"));
}

#[test]
fn every_planned_pair_is_required_and_no_other_is_accepted() {
    let refused = |edit: &dyn Fn(&mut serde_json::Value), why: &'static str| {
        assert_eq!(check(&record(edit)), Err(why));
    };
    // The fourth pair silently omitted, a pair repeated, a pair of the wrong treatments or an unplanned one.
    refused(
        &|v| {
            v["pairs"].as_array_mut().unwrap().pop();
        },
        "ActionLatencyPairNotInTheSchedule",
    );
    refused(
        &|v| v["pairs"][3] = v["pairs"][0].clone(),
        "ActionLatencyPairNotInTheSchedule",
    );
    refused(
        &|v| v["pairs"][1] = pair(2, 3, 9.0, 5.0),
        "ActionLatencyPairNotInTheSchedule",
    );
    // A block whose attempt never completed is not planned, so its absence is fine; but too few left is refused.
    let mut fewer = processes();
    fewer.retain(|p| p.slot < 4);
    assert_eq!(
        verify(&record(|_| {}), &schedule(), &fewer),
        Err("ActionLatencyTooFewPairs")
    );
}

#[test]
fn each_way_a_window_can_fail_to_be_a_measurement_has_its_own_refusal() {
    let refused = |edit: &dyn Fn(&mut serde_json::Value), why: &'static str| {
        assert_eq!(check(&record(edit)), Err(why));
    };
    refused(
        &|v| v["metric"] = "AverageFps".into(),
        "ActionLatencyWrongMetric",
    );
    refused(
        &|v| v["pairs"][0]["on"]["lost"] = 3.into(),
        "ActionLatencySourceNotQualified",
    );
    refused(
        &|v| v["pairs"][0]["on"]["same_target"] = false.into(),
        "ActionLatencySourceNotQualified",
    );
    refused(
        &|v| {
            v["pairs"][0]["off"]["histogram"] =
                json!([{ "le_ms": 1.0, "count": 500 }, { "le_ms": 9.0, "count": 400 }])
        },
        "ActionLatencyTooFewEvents",
    );
    refused(
        &|v| {
            v["pairs"][0]["off"]["histogram"] =
                json!([{ "le_ms": 3.0, "count": 5 }, { "le_ms": 2.0, "count": 2000 }])
        },
        "ActionLatencyNotFinite",
    );
    refused(
        &|v| v["pairs"][1]["off"]["histogram"] = json!([]),
        "ActionLatencyNotFinite",
    );
    refused(
        &|v| v["pairs"][0]["on"]["foreground_p99_ms"] = 0.0.into(),
        "ActionLatencyNotFinite",
    );
    // The foreground got worse by more than the guardrail allows.
    refused(
        &|v| v["pairs"][2]["on"]["foreground_p99_ms"] = 20.0.into(),
        "ActionLatencyGuardrailBroken",
    );
    // Outside its attempt, or from another game process.
    refused(
        &|v| v["pairs"][0]["off"]["start_unix"] = 900.into(),
        "ActionLatencyWindowOutsideTheAttempt",
    );
    refused(
        &|v| v["pairs"][0]["on"]["end_unix"] = 5_000.into(),
        "ActionLatencyWindowOutsideTheAttempt",
    );
    refused(
        &|v| v["pairs"][0]["on_process"]["pid"] = 999.into(),
        "ActionLatencyForAnotherProcess",
    );
    assert_eq!(check(b"{"), Err("ActionLatencyUnreadable"));
}
