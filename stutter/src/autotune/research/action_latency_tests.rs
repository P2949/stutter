use serde_json::json;

use super::*;
use crate::metrics::LatencyHistogram;

const SLOTS: u32 = 8;
const MS: u64 = 1_000_000;

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

/// A population of 2000 events in the controller's own histogram: 1900 at 0.5 ms and 100 at `tail_ns`.
fn histogram(tail_ns: u64) -> (serde_json::Value, u64) {
    let mut histogram = LatencyHistogram::new();
    for _ in 0..1_900 {
        histogram.record(500_000);
    }
    for _ in 0..100 {
        histogram.record(tail_ns);
    }
    (serde_json::to_value(histogram.snapshot()).unwrap(), tail_ns)
}

fn window(tail_ns: u64, foreground: f64) -> serde_json::Value {
    let (histogram, max_ns) = histogram(tail_ns);
    json!({
        "start_unix": 1_010, "end_unix": 1_090, "lost": 0, "same_target": true,
        "histogram": histogram, "max_ns": max_ns,
        "foreground_p99_ms": foreground, "foreground_samples": 1_000, "foreground_over_5ms": 10,
        "thermal_degraded": false, "thermal_throttle_count": 0,
        "cpu_power_limited": false, "gpu_power_limited": false,
    })
}

fn pair(off_slot: u32, on_slot: u32, off_tail: u64, on_tail: u64) -> serde_json::Value {
    let identity = |slot: u32| json!({ "pid": 100 + slot, "start_ticks": 50 + slot, "uid": 1000 });
    json!({
        "off_slot": off_slot, "on_slot": on_slot,
        "off_process": identity(off_slot), "on_process": identity(on_slot),
        "off": window(off_tail, 12.0), "on": window(on_tail, 12.0),
    })
}

fn pairs(off_tail: u64, on_tail: u64) -> serde_json::Value {
    // Blocks: (0,1) off then on, (2,3) on then off, (4,5), (6,7) on then off.
    json!([
        pair(0, 1, off_tail, on_tail),
        pair(3, 2, off_tail, on_tail),
        pair(4, 5, off_tail, on_tail),
        pair(7, 6, off_tail, on_tail),
    ])
}

fn record(edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let mut value = json!({ "schema": 3, "metric": "GameRunnableLatency", "pairs": pairs(20 * MS, 3 * MS / 2) });
    edit(&mut value);
    serde_json::to_vec(&value).unwrap()
}

fn check(bytes: &[u8]) -> Result<LatencyResult, &'static str> {
    verify(bytes, &schedule(), &processes())
}

#[test]
fn the_production_decision_over_the_recomputed_populations_decides_keep_revert_or_null() {
    assert_eq!(
        check(&record(|_| {})),
        Ok(LatencyResult {
            pairs: 4,
            improved: 4,
            regressed: 0,
            decision: Decision::Keep
        })
    );
    let result = |off: u64, on: u64| {
        check(&record(|v| v["pairs"] = pairs(off, on)))
            .unwrap()
            .decision
    };
    assert_eq!(result(3 * MS / 2, 20 * MS), Decision::Revert);
    // No change is a valid null.
    assert_eq!(result(20 * MS, 20 * MS), Decision::Null);
    // A stated percentile is not a field at all; nothing but the population counts.
    let text = String::from_utf8(record(|_| {}))
        .unwrap()
        .replace("\"lost\"", "\"p99_ms\":1.0,\"lost\"");
    assert_eq!(check(text.as_bytes()), Err("ActionLatencyUnreadable"));
}

#[test]
fn a_change_the_histogram_cannot_resolve_is_no_effect_never_a_manufactured_keep() {
    // 10 ms (the 5–10 ms bucket) against 5 ms (the 2–5 ms bucket) reads as 50% to a bucket-bound comparison, but
    // the real tails may differ by a hair across the 5 ms edge: the controller's resolution cannot show a benefit.
    let result = check(&record(|v| v["pairs"] = pairs(10 * MS, 5 * MS))).unwrap();
    assert_eq!((result.improved, result.decision), (0, Decision::Null));
}

#[test]
fn the_controllers_own_guards_turn_pairs_into_regressions() {
    // A foreground tail that rose past the production slack in every pair is a revert, not a refused record.
    let slower = check(&record(|v| {
        for index in 0..4 {
            v["pairs"][index]["on"]["foreground_p99_ms"] = 20.0.into();
        }
    }))
    .unwrap();
    assert_eq!((slower.regressed, slower.decision), (4, Decision::Revert));
    // One regressed pair among improved ones blocks a keep without being a revert.
    let mixed = check(&record(|v| {
        v["pairs"][2]["on"]["foreground_p99_ms"] = 20.0.into()
    }))
    .unwrap();
    assert_eq!(
        (mixed.improved, mixed.regressed, mixed.decision),
        (3, 1, Decision::Null)
    );
    // A throttled or power-limited candidate window is regressed as well.
    for field in ["thermal_degraded", "cpu_power_limited", "gpu_power_limited"] {
        let hot = check(&record(|v| {
            for index in 0..4 {
                v["pairs"][index]["on"][field] = true.into();
            }
        }))
        .unwrap();
        assert_eq!(hot.decision, Decision::Revert, "{field}");
    }
    // A candidate with a larger share of events over 5 ms (6% against 5%) regresses even though its 99th percentile bound fell from 20 ms to 10 ms.
    let heavier = check(&record(|v| {
        for index in 0..4 {
            let mut hist = LatencyHistogram::new();
            for _ in 0..1_880 {
                hist.record(500_000);
            }
            for _ in 0..120 {
                hist.record(10 * MS);
            }
            v["pairs"][index]["on"]["histogram"] = serde_json::to_value(hist.snapshot()).unwrap();
            v["pairs"][index]["on"]["max_ns"] = (10 * MS).into();
        }
    }))
    .unwrap();
    assert_eq!(heavier.decision, Decision::Revert);
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
        &|v| v["pairs"][1] = pair(2, 3, 20 * MS, 2 * MS),
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
    refused(&|v| v["schema"] = 2.into(), "ActionLatencyWrongMetric");
    refused(
        &|v| v["pairs"][0]["on"]["lost"] = 3.into(),
        "ActionLatencySourceNotQualified",
    );
    refused(
        &|v| v["pairs"][0]["on"]["same_target"] = false.into(),
        "ActionLatencySourceNotQualified",
    );
    // Too few events.
    refused(
        &|v| {
            let mut hist = LatencyHistogram::new();
            for _ in 0..500 {
                hist.record(MS);
            }
            v["pairs"][0]["off"]["histogram"] = serde_json::to_value(hist.snapshot()).unwrap();
            v["pairs"][0]["off"]["max_ns"] = MS.into();
        },
        "ActionLatencyTooFewEvents",
    );
    // A layout that is not the controller's, a maximum the buckets cannot hold, and no population at all.
    refused(
        &|v| {
            v["pairs"][0]["off"]["histogram"]
                .as_array_mut()
                .unwrap()
                .pop();
        },
        "ActionLatencyHistogramInvalid",
    );
    refused(
        &|v| v["pairs"][0]["off"]["max_ns"] = (40 * MS).into(),
        "ActionLatencyHistogramInvalid",
    );
    refused(
        &|v| v["pairs"][1]["off"]["histogram"] = json!([]),
        "ActionLatencyTooFewEvents",
    );
    refused(
        &|v| v["pairs"][0]["on"]["foreground_p99_ms"] = 0.0.into(),
        "ActionLatencyNotFinite",
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
